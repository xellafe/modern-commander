//! Riscrittura di uno ZIP con un lotto di modifiche (aggiunta o sostituzione, eliminazione, rinomina, nuova
//! cartella): il nuovo archivio si scrive una volta, copiando già compresse le voci che restano, e prende il posto
//! dell'originale solo quando è completo. Errore o annullamento eliminano i temporanei e l'originale resta com'era.

use super::container::ContainerFile;
use super::index::{EntryAt, NT_UNIX_DIFF_MS};
use super::list::{corrupt, io_err};
use super::provider::ArchiveProvider;
use super::{ArchiveFormat, format_of};
use crate::error::{AppError, ErrorCode};
use crate::net::temp;
use crate::paths::to_extended;
use crate::vfs::local::LocalProvider;
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Provider, Purpose, stream_error};
use crate::vfs::router::Vfs;
use crate::vfs::transfer::{install, part_name};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use zip::write::FullFileOptions;

/// Da questa dimensione una voce nuova ha i campi ZIP64: il margine copre la crescita di Deflate sui dati che non
/// si comprimono (pochi byte ogni 64 KiB).
const LARGE: u64 = u32::MAX as u64 - (16 << 20);

/// Dimensione e data del contenitore.
type Stamp = (u64, Option<i64>);

enum Op {
    Put(String, PathBuf, Option<i64>),
    Mkdir(String),
    Delete(String),
    Rename(String, String),
}

/// Origine di una voce del nuovo archivio.
enum Src {
    /// Indice nell'archivio originale; `dir` per una cartella esplicita (il nome finisce con `/`).
    Original { at: usize, dir: bool },
    Staged(PathBuf, Option<i64>),
    Dir,
}

/// Un lotto di modifiche a uno ZIP, locale o remoto. I percorsi sono quelli POSIX di `Loc::Archive` (`/` iniziale).
pub struct ZipEdit {
    container: Loc,
    /// All'apertura di uno ZIP esistente; `None` per uno ZIP nuovo.
    stamp: Option<Stamp>,
    replace: bool,
    ops: Vec<Op>,
    /// Cartella di lavoro sotto `temp::dir()`, creata al primo uso; si elimina con l'edit.
    work: Option<PathBuf>,
    staged: u32,
}

fn stamp(vfs: &Vfs, container: &Loc) -> Result<Stamp, AppError> {
    let (p, path) = vfs.endpoint(container, Purpose::Transfer)?;
    let s = p.stat(&path)?;
    Ok((s.size, s.modified_ms))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl ZipEdit {
    fn new(container: &Loc, stamp: Option<Stamp>, replace: bool) -> ZipEdit {
        ZipEdit { container: container.clone(), stamp, replace, ops: Vec::new(), work: None, staged: 0 }
    }

    /// Modifica di uno ZIP esistente: se ne annotano dimensione e data, che `commit` ricontrolla.
    pub fn open(vfs: &Vfs, container: &Loc) -> Result<ZipEdit, AppError> {
        let zip = container.file_name().and_then(|n| format_of(Path::new(&n))) == Some(ArchiveFormat::Zip);
        if !zip || matches!(container, Loc::Archive { .. }) {
            return Err(container.error(ErrorCode::Unsupported));
        }
        Ok(Self::new(container, Some(stamp(vfs, container)?), true))
    }

    /// ZIP nuovo; senza `replace` un file esistente con quel nome dà `ALREADY_EXISTS` all'installazione.
    pub fn create(container: &Loc, replace: bool) -> ZipEdit {
        Self::new(container, None, replace)
    }

    pub fn container(&self) -> &Loc {
        &self.container
    }

    fn work_dir(&mut self) -> Result<PathBuf, AppError> {
        if let Some(d) = &self.work {
            return Ok(d.clone());
        }
        let d = temp::dir().join(format!("edit-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).map_err(|e| AppError::from_io(&e, &d))?;
        self.work = Some(d.clone());
        Ok(d)
    }

    /// Percorso nuovo (il file non c'è ancora) nella cartella di lavoro, per il contenuto di un `put`.
    pub fn stage_file(&mut self) -> Result<PathBuf, AppError> {
        self.staged += 1;
        Ok(self.work_dir()?.join(self.staged.to_string()))
    }

    /// Aggiunge o sostituisce la voce `path` con il file `staged`; senza data, quella del commit.
    pub fn put(&mut self, path: &str, staged: PathBuf, modified_ms: Option<i64>) {
        self.ops.push(Op::Put(path.to_owned(), staged, modified_ms));
    }

    /// Cartella esplicita (se `path` non è già una voce).
    pub fn mkdir(&mut self, path: &str) {
        if path != "/" {
            self.ops.push(Op::Mkdir(path.to_owned()));
        }
    }

    /// La voce e tutto quello sotto `path/`, anche se `path` è una cartella implicita.
    pub fn delete(&mut self, path: &str) {
        self.ops.push(Op::Delete(path.to_owned()));
    }

    /// La voce e tutto quello sotto `from/`; i nomi di arrivo già presenti si sostituiscono.
    pub fn rename(&mut self, from: &str, to: &str) {
        self.ops.push(Op::Rename(from.to_owned(), to.to_owned()));
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Scrive il nuovo archivio e lo mette al posto del contenitore. `progress` riceve i byte scritti più quelli
    /// caricati (contenitore remoto); con `false` `CANCELLED`. Con `open`, un contenitore cambiato da allora dà
    /// `CHANGED_ON_DISK` senza scrivere niente.
    pub fn commit(mut self, vfs: &Vfs, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
        let r = self.write_and_install(vfs, progress);
        vfs.archives().invalidate(&self.container);
        r
    }

    /// Elimina la cartella di lavoro (lo fa anche `Drop`).
    pub fn abort(self) {}

    fn write_and_install(&mut self, vfs: &Vfs, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
        // ponytail: tra questo controllo e la rinomina finale un altro programma può ancora scrivere il contenitore.
        if let Some(old) = self.stamp
            && stamp(vfs, &self.container)? != old
        {
            return Err(self.container.error(ErrorCode::ChangedOnDisk));
        }
        // L'indice in cache può essere di prima del controllo (si ricontrolla al più ogni 2 s): si rilegge.
        let source = match self.stamp {
            Some(_) => {
                vfs.archives().invalidate(&self.container);
                Some(vfs.archive(&self.container, &AtomicBool::new(false))?)
            }
            None => None,
        };
        let entries = plan(&self.ops, source.as_deref());
        let zip = source.as_ref().map(|p| p.zip().cloned().ok_or_else(|| self.container.error(ErrorCode::Unsupported))).transpose()?;
        let container = self.container.clone();
        match &container {
            Loc::Local(p) => {
                let path = p.to_string_lossy();
                let tmp = PathBuf::from(part_name(&path));
                let file = create(&tmp)?; // non creato = niente da eliminare
                let r = write_zip(file, &tmp, entries, zip, &container, progress).and_then(|_| {
                    // Il contenitore non resta aperto dall'indice in cache mentre lo si sostituisce: Windows non
                    // sostituisce un file aperto, neanche con la condivisione in eliminazione.
                    // ponytail: una lettura in corso nello stesso istante (estrazione, F3) lo tiene aperto e la
                    // rinomina fallisce con `ACCESS_DENIED`, originale intatto; servirebbe la rinomina POSIX.
                    drop(source);
                    vfs.archives().invalidate(&container);
                    LocalProvider.rename(&tmp.to_string_lossy(), &path, self.replace)
                });
                if r.is_err() {
                    let _ = std::fs::remove_file(to_extended(&tmp));
                }
                r
            }
            _ => {
                let tmp = self.work_dir()?.join("new.zip");
                let written = write_zip(create(&tmp)?, &tmp, entries, zip, &container, progress)?;
                install(vfs, &container, &tmp, self.replace, &mut |n| progress(written + n))
            }
        }
    }
}

impl Drop for ZipEdit {
    fn drop(&mut self) {
        if let Some(d) = &self.work {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// Le voci del nuovo archivio: quelle originali nel loro ordine, poi le nuove, con le operazioni applicate in ordine.
fn plan(ops: &[Op], source: Option<&ArchiveProvider>) -> Vec<(String, Src)> {
    // Nome → (ordine, origine): l'ordine alfabetico dà i figli di una cartella come intervallo contiguo.
    let mut map: BTreeMap<String, (u64, Src)> = BTreeMap::new();
    let mut next = 0;
    for e in source.map(|p| p.index().entries()).unwrap_or_default() {
        if let EntryAt::Zip(at) = e.at {
            map.insert(e.path.clone(), (at as u64, Src::Original { at, dir: e.is_dir }));
            next = next.max(at as u64 + 1);
        }
    }
    let subtree = |map: &BTreeMap<String, (u64, Src)>, p: &str| -> Vec<String> {
        let prefix = format!("{p}/");
        let below = map.range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)).map(|(k, _)| k.clone());
        map.get_key_value(p).map(|(k, _)| k.clone()).into_iter().chain(below).collect()
    };
    for op in ops {
        match op {
            Op::Put(p, staged, ms) => {
                map.insert(p.clone(), (next, Src::Staged(staged.clone(), *ms)));
                next += 1;
            }
            Op::Mkdir(p) => {
                if !map.contains_key(p) {
                    map.insert(p.clone(), (next, Src::Dir));
                    next += 1;
                }
            }
            Op::Delete(p) => {
                for k in subtree(&map, p) {
                    map.remove(&k);
                }
            }
            Op::Rename(from, to) => {
                let moved: Vec<_> = subtree(&map, from).into_iter().map(|k| (format!("{to}{}", &k[from.len()..]), map.remove(&k).unwrap())).collect();
                map.extend(moved);
            }
        }
    }
    let mut entries: Vec<_> = map.into_iter().collect();
    entries.sort_by_key(|(_, (order, _))| *order);
    entries.into_iter().map(|(name, (_, src))| (name, src)).collect()
}

fn create(path: &Path) -> Result<File, AppError> {
    File::options().read(true).write(true).create_new(true).open(to_extended(path)).map_err(|e| AppError::from_io(&e, path))
}

/// Scrive in `file` (il temporaneo `tmp`) lo ZIP con `entries`, copiando le voci originali da `zip` senza
/// ricomprimerle, e dà i byte scritti. `progress` dopo ogni scrittura.
fn write_zip(
    file: File,
    tmp: &Path,
    entries: Vec<(String, Src)>,
    mut zip: Option<zip::ZipArchive<ContainerFile>>,
    container: &Loc,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<u64, AppError> {
    let tname = tmp.to_string_lossy();
    let cname = PathBuf::from(container.to_string());
    let werr = |e: zip::result::ZipError| match e {
        zip::result::ZipError::Io(e) => stream_error(e, &tname),
        e => corrupt(&cname, e),
    };
    let mut w = zip::ZipWriter::new(Counted { inner: BufWriter::new(file), pos: 0, done: 0, progress, name: &cname });
    for (path, src) in entries {
        let name = &path[1..];
        match src {
            Src::Original { at, dir } => {
                let z = zip.as_mut().expect("voce originale senza archivio");
                let entry = Loc::archive(container.clone(), &path).to_string();
                let f = z.by_index_raw(at).map_err(|e| match e {
                    zip::result::ZipError::Io(e) => io_err(&e, Path::new(&entry)),
                    e => corrupt(Path::new(&entry), e),
                })?;
                // Senza la cifratura nel crate, la copia perderebbe il flag: la voce diventerebbe illeggibile.
                if f.encrypted() {
                    return Err(AppError { detail: Some("encrypted".into()), ..AppError::new(ErrorCode::Unsupported, Some(Path::new(&entry))) });
                }
                // ponytail: il crate non ricopia il campo UT (data UTC) delle voci invariate; resta la data DOS.
                w.raw_copy_file_rename(f, if dir { format!("{name}/") } else { name.to_owned() }).map_err(werr)?;
            }
            Src::Dir => w.add_directory(name, options(now_ms())).map_err(werr)?,
            Src::Staged(p, ms) => {
                let mut f = File::open(to_extended(&p)).map_err(|e| AppError::from_io(&e, &p))?;
                let len = f.metadata().map_err(|e| AppError::from_io(&e, &p))?.len();
                w.start_file(name, options(ms.unwrap_or_else(now_ms)).large_file(len >= LARGE)).map_err(werr)?;
                io::copy(&mut f, &mut w).map_err(|e| stream_error(e, &tname))?;
            }
        }
    }
    let c = w.finish().map_err(werr)?;
    let done = c.done;
    let file = c.inner.into_inner().map_err(|e| AppError::from_io(e.error(), tmp))?;
    file.sync_all().map_err(|e| AppError::from_io(&e, tmp))?;
    Ok(done)
}

/// Opzioni di una voce nuova: Deflate, data DOS in ora locale e campo UT (al secondo, in UTC).
fn options(ms: i64) -> FullFileOptions<'static, 'static> {
    let mut o = FullFileOptions::default().compression_method(zip::CompressionMethod::Deflated).last_modified_time(dos_time(ms));
    if let Ok(secs) = u32::try_from(ms.div_euclid(1000)) {
        let mut ut = vec![1u8];
        ut.extend(secs.to_le_bytes());
        let _ = o.add_extra_field(0x5455, ut, false);
    }
    o
}

/// Data DOS dei millisecondi Unix `ms`, nell'ora locale di questo PC (come la leggono Explorer, 7-Zip e
/// `list::dos_to_ms`); fuori dall'intervallo DOS (1980-2107) il 1° gennaio 1980.
fn dos_time(ms: i64) -> zip::DateTime {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = ms.checked_add(NT_UNIX_DIFF_MS).and_then(|t| t.checked_mul(10_000)).and_then(|t| u64::try_from(t).ok());
    let Some(ticks) = ticks else { return zip::DateTime::default() };
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut t) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    let ok = unsafe { FileTimeToSystemTime(&ft, &mut utc).is_ok() && SystemTimeToTzSpecificLocalTime(None, &utc, &mut t).is_ok() };
    let dos = || zip::DateTime::from_date_and_time(t.wYear, t.wMonth as u8, t.wDay as u8, t.wHour as u8, t.wMinute as u8, t.wSecond as u8).ok();
    ok.then(dos).flatten().unwrap_or_default()
}

/// Il file del nuovo archivio: conta i byte (la posizione più avanti raggiunta) e chiede a `progress` se continuare.
struct Counted<'a, W> {
    inner: W,
    pos: u64,
    done: u64,
    progress: &'a mut dyn FnMut(u64) -> bool,
    name: &'a Path,
}

impl<W: Write> Write for Counted<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.pos += n as u64;
        if self.pos > self.done {
            self.done = self.pos;
            if !(self.progress)(self.done) {
                return Err(io::Error::other(AppError::new(ErrorCode::Cancelled, Some(self.name))));
            }
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Seek> Seek for Counted<'_, W> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = self.inner.seek(pos)?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::list::dos_to_ms;
    use crate::error::ErrorCode;
    use crate::ops::exec::PART_SUFFIX;
    use crate::vfs::mem::{MemProvider, vfs};
    use crate::vfs::provider::Provider;
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn archive(bytes: Vec<u8>) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap()
    }

    /// Nomi delle voci in ordine d'archivio.
    fn names(bytes: Vec<u8>) -> Vec<String> {
        let mut z = archive(bytes);
        (0..z.len()).map(|i| z.by_index_raw(i).unwrap().name().unwrap().into_owned()).collect()
    }

    /// Byte compressi e CRC di una voce.
    fn raw(bytes: Vec<u8>, name: &str) -> (Vec<u8>, u32) {
        let mut z = archive(bytes);
        let i = z.index_for_name(name).unwrap();
        let mut f = z.by_index_raw(i).unwrap();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        (data, f.crc32())
    }

    fn parts_in(dir: &Path) -> Vec<String> {
        let names = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned());
        names.filter(|n| n.ends_with(PART_SUFFIX)).collect()
    }

    /// `t.zip` locale con `a.txt`, `d/b.txt`, `d/c.txt`.
    fn local_zip() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("t.zip");
        let a = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".repeat(50);
        std::fs::write(&z, zip_bytes(&[("a.txt", &a), ("d/b.txt", b"b"), ("d/c.txt", b"c")])).unwrap();
        (d, z)
    }

    #[test]
    fn edit_put_delete_rename_mkdir_keeps_untouched_entries_raw() {
        let (d, z) = local_zip();
        let before = raw(std::fs::read(&z).unwrap(), "a.txt");
        let v = vfs(&[]);
        let c = Loc::Local(z.clone());
        let no = AtomicBool::new(false);
        // L'indice in cache prima della modifica: dopo il commit si rilegge.
        assert!(v.archive(&c, &no).unwrap().index().get("/new.txt").is_none());
        let mut e = ZipEdit::open(&v, &c).unwrap();
        assert!(e.is_empty());
        let s = e.stage_file().unwrap();
        std::fs::write(&s, b"nuovo").unwrap();
        let t = 1_700_000_001_000;
        e.put("/new.txt", s, Some(t));
        e.delete("/d/b.txt");
        e.rename("/d", "/e");
        e.mkdir("/empty");
        e.mkdir("/");
        assert!(!e.is_empty());
        e.commit(&v, &mut |_| true).unwrap();
        let bytes = std::fs::read(&z).unwrap();
        assert_eq!(names(bytes.clone()), ["a.txt", "e/c.txt", "new.txt", "empty/"]);
        assert_eq!(raw(bytes.clone(), "a.txt"), before);
        let mut z2 = archive(bytes);
        let mut text = String::new();
        z2.by_name("new.txt").unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "nuovo");
        // La data sopravvive entro 2 s, sia quella DOS (ora locale) sia quella dell'indice (campo UT).
        let dos = z2.by_name("new.txt").unwrap().last_modified().unwrap();
        let dos = dos_to_ms(dos.year(), dos.month(), dos.day(), dos.hour(), dos.minute(), dos.second()).unwrap();
        assert!((t - dos).abs() < 2000, "{t} {dos}");
        let ix = v.archive(&c, &no).unwrap();
        let m = ix.index().get("/new.txt").unwrap().modified_ms.unwrap();
        assert!((t - m).abs() < 2000, "{t} {m}");
        assert!(parts_in(d.path()).is_empty());
    }

    #[test]
    fn commit_failure_and_cancel_keep_original() {
        let (d, z) = local_zip();
        let orig = std::fs::read(&z).unwrap();
        let v = vfs(&[]);
        let c = Loc::Local(z.clone());
        // Annullamento al primo blocco.
        let mut e = ZipEdit::open(&v, &c).unwrap();
        let s = e.stage_file().unwrap();
        let work = s.parent().unwrap().to_path_buf();
        std::fs::write(&s, vec![7u8; 1 << 20]).unwrap();
        e.put("/x.bin", s, None);
        assert_eq!(e.commit(&v, &mut |_| false).unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!(std::fs::read(&z).unwrap(), orig);
        assert!(parts_in(d.path()).is_empty());
        assert!(!work.exists());
        // File preparato sparito prima del commit.
        let mut e = ZipEdit::open(&v, &c).unwrap();
        let s = e.stage_file().unwrap();
        let work = s.parent().unwrap().to_path_buf();
        std::fs::write(&s, b"x").unwrap();
        e.put("/y.txt", s.clone(), None);
        std::fs::remove_file(&s).unwrap();
        assert_eq!(e.commit(&v, &mut |_| true).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(std::fs::read(&z).unwrap(), orig);
        assert!(parts_in(d.path()).is_empty());
        assert!(!work.exists());
    }

    #[test]
    fn commit_detects_external_change() {
        let (_d, z) = local_zip();
        let v = vfs(&[]);
        let mut e = ZipEdit::open(&v, &Loc::Local(z.clone())).unwrap();
        e.mkdir("/x");
        let external = zip_bytes(&[("altro.txt", b"scritto da un altro programma")]);
        std::fs::write(&z, &external).unwrap();
        assert_eq!(e.commit(&v, &mut |_| true).unwrap_err().code, ErrorCode::ChangedOnDisk);
        assert_eq!(std::fs::read(&z).unwrap(), external);
    }

    #[test]
    fn remote_commit_uploads_beside_the_original() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/r/t.zip", &zip_bytes(&[("a.txt", b"a")]));
        let v = vfs(&[("m", &m)]);
        let c = Loc::parse("sftp://m/r/t.zip").unwrap();
        let mut e = ZipEdit::open(&v, &c).unwrap();
        e.rename("/a.txt", "/b.txt");
        let mut last = 0;
        e.commit(&v, &mut |n| {
            last = n;
            true
        })
        .unwrap();
        let bytes = m.data("/r/t.zip").unwrap();
        // Byte scritti più byte caricati.
        assert_eq!(last, 2 * bytes.len() as u64);
        assert_eq!(names(bytes), ["b.txt"]);
        let left: Vec<String> = m.list("/r").unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(left, ["t.zip"]);
        // ZIP nuovo sul remoto.
        let c = Loc::parse("sftp://m/r/n.zip").unwrap();
        let mut e = ZipEdit::create(&c, false);
        e.mkdir("/d");
        e.commit(&v, &mut |_| true).unwrap();
        assert_eq!(names(m.data("/r/n.zip").unwrap()), ["d/"]);
    }
}
