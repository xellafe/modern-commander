//! Un archivio come provider di cartella, in sola lettura: elenco e metadati dall'indice, voci lette dal contenitore
//! (a salti per ZIP e TAR, con i cursori per i formati a flusso). Le modifiche passano dal runner.

use super::container::{self, ContainerFile};
use super::cursor::{self, Slot};
use super::index::{EntryAt, Index, build_7z, build_tar, build_zip};
use super::list::{corrupt, io_err};
use super::pipe::piped;
use super::{ArchiveFormat, format_of, sevenz_guard};
use crate::error::{AppError, ErrorCode};
use crate::net::temp;
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Caps, Connector, Provider, Purpose, Stat, stream_error};
use crate::vfs::router::Vfs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

enum Data {
    /// Si clona per ogni lettura.
    Zip(zip::ZipArchive<ContainerFile>),
    SevenZ(Arc<sevenz_rust2::Archive>),
    Tar,
}

/// Copia locale di un contenitore remoto (TAR.GZ, o server senza letture a salti): si elimina con il provider.
struct TempCopy(PathBuf);

impl Drop for TempCopy {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub struct ArchiveProvider {
    container: Loc,
    index: Index,
    data: Data,
    file: ContainerFile,
    /// Il contenitore è scrivibile.
    writable: bool,
    cursor: Slot,
    _copy: Option<TempCopy>,
}

fn unsupported(path: &Path, detail: &str) -> AppError {
    AppError { detail: Some(detail.into()), ..AppError::new(ErrorCode::Unsupported, Some(path)) }
}

fn zip_error(e: zip::result::ZipError, name: &Path) -> AppError {
    use zip::result::ZipError;
    match e {
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => unsupported(name, "encrypted"),
        ZipError::UnsupportedArchive(_) | ZipError::CompressionMethodNotSupported(_) => AppError::new(ErrorCode::Unsupported, Some(name)),
        ZipError::Io(e) => io_err(&e, name),
        e => corrupt(name, e),
    }
}

/// Copia una voce nel lettore a canale; un lettore chiuso non è un errore (non c'è più nessuno a cui dirlo).
fn copy_entry(r: &mut dyn Read, w: &mut dyn Write, name: &Path) -> Result<(), AppError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(&e, name)),
        };
        if w.write_all(&buf[..n]).is_err() {
            return Ok(());
        }
    }
}

/// Voce di un TAR letta direttamente dal contenitore: esattamente `left` byte, o archivio troncato.
struct Exact {
    inner: io::Take<ContainerFile>,
    left: u64,
    name: PathBuf,
}

impl Read for Exact {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let n = self.inner.read(buf).map_err(|e| cursor::to_io(e, &self.name))?;
        if n == 0 {
            return Err(io::Error::other(corrupt(&self.name, "voce troncata")));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

impl ArchiveProvider {
    /// Legge l'indice di `container` (locale o remoto). Un TAR.GZ remoto, o un contenitore su un server senza letture
    /// a salti, si scarica prima nella cartella temporanea dell'app. `cancel` interrompe download e lettura.
    pub(super) fn open(connector: &Arc<dyn Connector>, container: &Loc, cancel: &AtomicBool) -> Result<Self, AppError> {
        let format = container.file_name().and_then(|n| format_of(Path::new(&n))).ok_or_else(|| container.error(ErrorCode::Unsupported))?;
        let (writable, ranged) = match container {
            Loc::Remote { scheme, conn, .. } => {
                let caps = connector.provider(*scheme, conn, Purpose::Browse)?.caps();
                (caps.writable, caps.ranged_read)
            }
            _ => (true, true),
        };
        let copy = if container.is_remote() && (format == ArchiveFormat::TarGz || !ranged) {
            let vfs = Vfs::new(connector.clone());
            Some(TempCopy(temp::fetch_in(&vfs, container, &temp::dir(), cancel, &mut |_| {})?))
        } else {
            None
        };
        let source = copy.as_ref().map_or_else(|| container.clone(), |c| Loc::Local(c.0.clone()));
        let file = container::open(connector, &source)?;
        let built = match format {
            ArchiveFormat::Zip => build_zip(file.clone(), cancel).map(|(i, z)| (i, Data::Zip(z))),
            ArchiveFormat::SevenZ => build_7z(file.clone(), cancel).map(|(i, a)| (i, Data::SevenZ(Arc::new(a)))),
            ArchiveFormat::Tar | ArchiveFormat::TarGz => build_tar(file.clone(), format == ArchiveFormat::TarGz, cancel).map(|i| (i, Data::Tar)),
        };
        // Gli errori della copia locale nominano il contenitore, non il temporaneo.
        let (index, data) = built.map_err(|e| if copy.is_some() { AppError { path: Some(container.to_string()), ..e } } else { e })?;
        file.set_indexed();
        Ok(Self { container: container.clone(), index, data, file, writable, cursor: Slot::default(), _copy: copy })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Una lettura del contenitore è fallita: la cache non deve più servire questo archivio.
    pub(super) fn broken(&self) -> bool {
        self.file.broken()
    }

    /// L'archivio del crate di uno ZIP (per la riscrittura); `None` per gli altri formati.
    pub(crate) fn zip(&self) -> Option<&zip::ZipArchive<ContainerFile>> {
        match &self.data {
            Data::Zip(z) => Some(z),
            _ => None,
        }
    }

    /// Forma testuale di una voce, per gli errori.
    fn name(&self, path: &str) -> PathBuf {
        PathBuf::from(Loc::archive(self.container.clone(), path).to_string())
    }

    fn err(&self, code: ErrorCode, path: &str) -> AppError {
        AppError::new(code, Some(&self.name(path)))
    }

    pub(super) fn zip_reader(&self, z: &zip::ZipArchive<ContainerFile>, i: usize, name: PathBuf) -> Result<Box<dyn Read + Send>, AppError> {
        use zip::CompressionMethod;
        let meta = z.by_index_data(i).map_err(|e| zip_error(e, &name))?;
        if meta.encrypted() {
            return Err(unsupported(&name, "encrypted"));
        }
        let method = meta.compression();
        if !zip::SUPPORTED_COMPRESSION_METHODS.contains(&method) && method != CompressionMethod::Lzma {
            return Err(AppError::new(ErrorCode::Unsupported, Some(&name)));
        }
        let mut z = z.clone();
        if method == CompressionMethod::Lzma {
            // Il crate alloca il dizionario dichiarato nella voce: stesso tetto del 7z. Intestazione LZMA dello ZIP:
            // versione (2 byte), lunghezza delle proprietà (2), proprietà (5, con il dizionario dopo il primo byte).
            let mut head = [0u8; 9];
            z.by_index_raw(i).map_err(|e| zip_error(e, &name))?.read_exact(&mut head).map_err(|e| io_err(&e, &name))?;
            sevenz_guard::check_coder(&[0x03, 0x01, 0x01], &head[4..]).map_err(|e| AppError { path: Some(name.to_string_lossy().into_owned()), ..e })?;
        }
        Ok(piped(move |w| {
            let mut f = z.by_index(i).map_err(|e| zip_error(e, &name))?;
            copy_entry(&mut f, w, &name)
        }))
    }
}

impl Provider for ArchiveProvider {
    fn caps(&self) -> Caps {
        let zip = self.index.format == ArchiveFormat::Zip;
        Caps { writable: zip && self.writable, atomic_replace: false, precise_mtime: !zip, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        self.index.stat(path).ok_or_else(|| self.err(ErrorCode::NotFound, path))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        let missing = if self.index.stat(path).is_some() { ErrorCode::InvalidRequest } else { ErrorCode::NotFound };
        self.index.list(path).ok_or_else(|| self.err(missing, path))
    }

    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        // Dentro una voce compressa non si salta: si decomprime fino a `offset`.
        let mut r = self.reader(path)?;
        let skipped = io::copy(&mut r.by_ref().take(offset), &mut io::sink()).map_err(|e| stream_error(e, path))?;
        let mut out = Vec::new();
        if skipped == offset {
            r.take(u64::from(len)).read_to_end(&mut out).map_err(|e| stream_error(e, path))?;
        }
        Ok(out)
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let Some(e) = self.index.get(path) else {
            let code = if self.index.stat(path).is_some() { ErrorCode::InvalidRequest } else { ErrorCode::NotFound };
            return Err(self.err(code, path));
        };
        let name = self.name(path);
        if e.is_dir {
            return Err(self.err(ErrorCode::InvalidRequest, path));
        } else if e.bad_name {
            return Err(self.err(ErrorCode::InvalidName, path));
        } else if e.link {
            // Come `unsupported_link` del runner: i collegamenti non si estraggono.
            return Err(unsupported(&name, "reparse_point"));
        } else if e.sparse {
            return Err(unsupported(&name, "sparse"));
        }
        match (&self.data, e.at) {
            (Data::Zip(z), EntryAt::Zip(i)) => self.zip_reader(z, i, name),
            (Data::Tar, EntryAt::Tar { data }) => {
                let mut f = self.file.clone();
                f.seek(SeekFrom::Start(data)).map_err(|err| io_err(&err, &name))?;
                Ok(Box::new(Exact { inner: f.take(e.size), left: e.size, name }))
            }
            (Data::Tar, EntryAt::TarGz) => cursor::targz(&self.cursor, &self.file, e.order, name),
            (Data::SevenZ(_), EntryAt::SevenZ { block: None, .. }) => Ok(Box::new(io::empty())),
            (Data::SevenZ(a), EntryAt::SevenZ { file, block: Some(b) }) => {
                if cursor::encrypted_block(a, b) {
                    return Err(unsupported(&name, "encrypted"));
                }
                Ok(cursor::sevenz(&self.cursor, a, &self.file, b, file, name))
            }
            _ => Err(self.err(ErrorCode::InvalidRequest, path)),
        }
    }

    fn stream_order(&self, path: &str) -> Option<u64> {
        self.index.get(path).filter(|_| self.index.solid).map(|e| e.order)
    }

    fn writer(&self, path: &str, _: bool) -> Result<Box<dyn Write + Send>, AppError> {
        Err(self.err(ErrorCode::Unsupported, path))
    }

    fn rename(&self, from: &str, _: &str, _: bool) -> Result<(), AppError> {
        Err(self.err(ErrorCode::Unsupported, from))
    }

    fn delete(&self, path: &str, _: bool) -> Result<(), AppError> {
        Err(self.err(ErrorCode::Unsupported, path))
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        Err(self.err(ErrorCode::Unsupported, path))
    }

    fn set_meta(&self, path: &str, _: Option<i64>, _: Option<u32>) -> Result<(), AppError> {
        Err(self.err(ErrorCode::Unsupported, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::mem::{MemProvider, vfs};
    use std::collections::HashSet;
    use std::time::Duration;

    /// ZIP "stored" con le voci date.
    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, o).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// Byte pseudo-casuali (non comprimibili), diversi per ogni `seed`.
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    fn read_all(p: &dyn Provider, path: &str) -> Vec<u8> {
        let mut out = Vec::new();
        p.reader(path).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    fn names(p: &dyn Provider, path: &str) -> Vec<String> {
        p.list(path).unwrap().into_iter().map(|s| s.name).collect()
    }

    fn at(container: &Loc) -> Loc {
        Loc::archive(container.clone(), "/")
    }

    #[test]
    fn zip_on_remote_reads_index_and_one_entry_only() {
        let m = Arc::new(MemProvider::new());
        let big: Vec<u8> = (0..4u32 << 20).map(|i| (i % 251) as u8).collect();
        m.add_file("/rel/app.zip", &zip_bytes(&[("big.bin", &big), ("conf/a.xml", b"0123456789")]));
        let v = vfs(&[("m", &m)]);
        let (p, path) = v.remote(&Loc::parse("arc://sftp:%2F%2Fm%2Frel%2Fapp.zip/conf").unwrap(), Purpose::Browse).unwrap();
        assert_eq!(names(&*p, &path), ["a.xml"]);
        assert_eq!(read_all(&*p, "/conf/a.xml"), b"0123456789");
        let read = m.bytes_read.load(std::sync::atomic::Ordering::Relaxed);
        assert!(read < 1 << 20, "letti {read} byte");
        assert_eq!(p.stat("/big.bin").unwrap().size, 4 << 20);
        assert!(p.caps().writable && !p.caps().precise_mtime);
    }

    #[test]
    fn solid_7z_block_decoded_once() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
        let contents: Vec<Vec<u8>> = (0..200).map(|i| noise(i, 20 * 1024)).collect();
        let mut w = ArchiveWriter::new(io::Cursor::new(Vec::new())).unwrap();
        let entries = (0..200).map(|i| ArchiveEntry::new_file(&format!("d/f{i:03}.bin"))).collect();
        w.push_archive_entries(entries, contents.iter().map(|c| SourceReader::new(c.as_slice())).collect()).unwrap();
        let bytes = w.finish().unwrap().into_inner();
        let m = Arc::new(MemProvider::new());
        m.add_file("/s.7z", &bytes);
        let v = vfs(&[("m", &m)]);
        let (p, _) = v.remote(&at(&Loc::parse("sftp://m/s.7z").unwrap()), Purpose::Browse).unwrap();
        let mut paths: Vec<(u64, usize)> = (0..200).map(|i| (p.stream_order(&format!("/d/f{i:03}.bin")).unwrap(), i)).collect();
        paths.sort();
        for (_, i) in paths {
            assert_eq!(read_all(&*p, &format!("/d/f{i:03}.bin")), contents[i], "f{i:03}");
        }
        let read = m.bytes_read.load(std::sync::atomic::Ordering::Relaxed) as f64;
        assert!(read <= bytes.len() as f64 * 1.1 + 65536.0, "letti {read} su {}", bytes.len());
    }

    #[test]
    fn sevenz_non_solid_and_empty_entries() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let d = tempfile::tempdir().unwrap();
        let p7 = d.path().join("n.7z");
        let mut w = ArchiveWriter::create(&p7).unwrap();
        w.push_archive_entry(ArchiveEntry::new_file("a"), Some(&b"alpha"[..])).unwrap();
        w.push_archive_entry(ArchiveEntry::new_file("empty"), Some(&b""[..])).unwrap();
        w.push_archive_entry(ArchiveEntry::new_file("b"), Some(&b"beta"[..])).unwrap();
        w.finish().unwrap();
        let v = vfs(&[]);
        let (p, _) = v.remote(&at(&Loc::Local(p7)), Purpose::Browse).unwrap();
        assert_eq!(p.stream_order("/a"), None);
        // In ordine inverso: ogni voce ha il suo blocco.
        assert_eq!(read_all(&*p, "/b"), b"beta");
        assert_eq!(read_all(&*p, "/empty"), b"");
        assert_eq!(read_all(&*p, "/a"), b"alpha");
        assert_eq!(p.read_at("/b", 1, 2).unwrap(), b"et");
    }

    fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn targz_cursor_goes_forward_and_restarts_backwards() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (noise(1, 3000), noise(2, 70_000), noise(3, 10));
        let t = tar_bytes(&[("a", &a), ("b", &b), ("c", &c)]);
        let g = d.path().join("t.tar.gz");
        std::fs::write(&g, gz(&t)).unwrap();
        let v = vfs(&[]);
        let (p, _) = v.remote(&at(&Loc::Local(g)), Purpose::Browse).unwrap();
        assert_eq!(read_all(&*p, "/c"), c);
        assert_eq!(read_all(&*p, "/a"), a);
        assert_eq!(read_all(&*p, "/c"), c); // avanti dal cursore, saltando `b`
        assert_eq!(p.read_at("/b", 65_000, 10).unwrap(), &b[65_000..65_010]); // chiuso a metà: il cursore non torna
        assert_eq!(read_all(&*p, "/b"), b);
        let order: Vec<u64> = ["/a", "/b", "/c"].iter().map(|n| p.stream_order(n).unwrap()).collect();
        assert!(order[0] < order[1] && order[1] < order[2]);
        // Lo stesso TAR non compresso: voci lette a salti, senza ordine di flusso.
        let plain = d.path().join("t.tar");
        std::fs::write(&plain, &t).unwrap();
        let (p, _) = v.remote(&at(&Loc::Local(plain)), Purpose::Browse).unwrap();
        assert_eq!((read_all(&*p, "/b"), p.stream_order("/b")), (b, None));
    }

    #[test]
    fn remote_targz_is_downloaded_once() {
        let m = Arc::new(MemProvider::new());
        let bytes = gz(&tar_bytes(&[("d/x.txt", b"abc"), ("y", b"12345")]));
        m.add_file("/t.tgz", &bytes);
        let v = vfs(&[("m", &m)]);
        let loc = at(&Loc::parse("sftp://m/t.tgz").unwrap());
        for _ in 0..2 {
            let (p, _) = v.remote(&loc, Purpose::Browse).unwrap();
            assert_eq!(names(&*p, "/").into_iter().collect::<HashSet<_>>(), HashSet::from(["d".to_owned(), "y".to_owned()]));
            assert_eq!(read_all(&*p, "/y"), b"12345");
        }
        assert_eq!(m.bytes_read.load(std::sync::atomic::Ordering::Relaxed), bytes.len() as u64);
    }

    #[test]
    fn cache_revalidates_on_change() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("c.zip");
        std::fs::write(&z, zip_bytes(&[("a", b"1")])).unwrap();
        let v = vfs(&[]);
        let loc = at(&Loc::Local(z.clone()));
        let list = || names(&*v.remote(&loc, Purpose::Browse).unwrap().0, "/");
        assert_eq!(list(), ["a"]);
        std::fs::write(&z, zip_bytes(&[("a", b"1"), ("b", b"2")])).unwrap();
        std::thread::sleep(Duration::from_millis(2100));
        assert_eq!(list(), ["a", "b"]);
        std::fs::write(&z, zip_bytes(&[("a", b"1"), ("b", b"2"), ("c", b"3")])).unwrap();
        v.archives().invalidate(&Loc::Local(z.clone()));
        assert_eq!(list(), ["a", "b", "c"]);
    }

    #[test]
    fn idle_container_is_not_kept_open() {
        let d = tempfile::tempdir().unwrap();
        let rel = d.path().join("rel");
        std::fs::create_dir(&rel).unwrap();
        let z = rel.join("a.zip");
        std::fs::write(&z, zip_bytes(&[("a", b"1")])).unwrap();
        let v = vfs(&[]);
        let loc = at(&Loc::Local(z.clone()));
        let wait = || std::thread::sleep(container::IDLE + Duration::from_millis(500));
        assert_eq!(names(&*v.remote(&loc, Purpose::Browse).unwrap().0, "/"), ["a"]);
        assert_eq!(read_all(&*v.remote(&loc, Purpose::Browse).unwrap().0, "/a"), b"1");
        wait();
        // L'archivio resta in cache, ma il file no: dopo la pausa si riapre per leggere.
        assert_eq!(read_all(&*v.remote(&loc, Purpose::Browse).unwrap().0, "/a"), b"1");
        wait();
        // F5 Sovrascrivi sullo ZIP, poi la rinomina della cartella che lo contiene, con l'executor dell'app
        // (`MoveFileExW`: con il file aperto entrambe danno `ACCESS_DENIED`, anche se aperto con condivisione totale).
        use crate::ops::exec::{Executor, LocalExecutor};
        let new = d.path().join("new.zip");
        std::fs::write(&new, zip_bytes(&[("b", b"2")])).unwrap();
        LocalExecutor.move_path(&new, &z, true).unwrap();
        LocalExecutor.rename(&rel, &d.path().join("rel2")).unwrap();
    }

    #[test]
    fn read_error_evicts_the_archive_from_the_cache() {
        let m = Arc::new(MemProvider::new());
        // La voce grande sta fuori dalla finestra letta con l'indice: leggerla torna al server.
        m.add_file("/a.zip", &zip_bytes(&[("big", &noise(1, 600_000)), ("a", b"1")]));
        let v = vfs(&[("m", &m)]);
        let loc = at(&Loc::parse("sftp://m/a.zip").unwrap());
        let (p, _) = v.remote(&loc, Purpose::Browse).unwrap();
        m.delete("/a.zip", false).unwrap();
        assert!(p.read_at("/big", 0, 10).is_err());
        // Prima del ricontrollo (2 s): l'archivio in cache non serve più, si rilegge (e qui non c'è).
        assert_eq!(v.remote(&loc, Purpose::Browse).err().map(|e| e.code), Some(ErrorCode::NotFound));
    }

    /// Connettore che annota lo scopo di ogni provider chiesto.
    struct Recording(Arc<MemProvider>, std::sync::Mutex<Vec<Purpose>>);

    impl Connector for Recording {
        fn provider(&self, _: crate::vfs::loc::Scheme, _: &str, purpose: Purpose) -> Result<Arc<dyn Provider>, AppError> {
            self.1.lock().unwrap().push(purpose);
            Ok(self.0.clone())
        }
    }

    #[test]
    fn index_reads_browse_and_entry_reads_transfer() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/a.zip", &zip_bytes(&[("big", &noise(1, 600_000)), ("a", b"1")]));
        let rec = Arc::new(Recording(m, Default::default()));
        let connector: Arc<dyn Connector> = rec.clone();
        let cache = crate::archive::cache::ArchiveCache::new();
        let p = cache.open(&connector, &Loc::parse("sftp://m/a.zip").unwrap(), &AtomicBool::new(false)).unwrap();
        let taken = || std::mem::take(&mut *rec.1.lock().unwrap());
        // Come la lista: nessuna sessione di trasferimento (su FTP una connessione in più).
        assert!(taken().iter().all(|&p| p == Purpose::Browse));
        assert_eq!(p.read_at("/big", 0, 3).unwrap().len(), 3);
        let t = taken();
        assert!(!t.is_empty() && t.iter().all(|&p| p == Purpose::Transfer));
    }

    /// Sostituisce ogni `from` con `to` (stessa lunghezza).
    fn patch(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        for i in 0..=bytes.len() - from.len() {
            if &bytes[i..i + from.len()] == from {
                bytes[i..i + from.len()].copy_from_slice(to);
            }
        }
    }

    #[test]
    fn bad_entries_fail_on_read() {
        let d = tempfile::tempdir().unwrap();
        let mut w = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for name in ["XXup", "enc", "ok"] {
            w.start_file(name, o).unwrap();
            w.write_all(b"data").unwrap();
        }
        w.add_symlink("l", "ok", o).unwrap();
        let mut bytes = w.finish().unwrap().into_inner();
        patch(&mut bytes, b"XXup", b"../u"); // `ZipWriter` non scrive nomi che risalgono
        // Bit 0 dei flag (cifrata) di `enc`, nell'intestazione locale e in quella centrale.
        let find = |b: &[u8], sig: &[u8], name_at: usize| (0..b.len() - 4).find(|&i| &b[i..i + 4] == sig && b[i + name_at..].starts_with(b"enc")).unwrap();
        let local = find(&bytes, b"PK\x03\x04", 30);
        bytes[local + 6] |= 1;
        let central = find(&bytes, b"PK\x01\x02", 46);
        bytes[central + 8] |= 1;
        let z = d.path().join("bad.zip");
        std::fs::write(&z, bytes).unwrap();
        let v = vfs(&[]);
        let (p, _) = v.remote(&at(&Loc::Local(z)), Purpose::Browse).unwrap();
        let err = |path: &str| p.reader(path).err().map(|e| (e.code, e.detail));
        assert_eq!(err("/u"), Some((ErrorCode::InvalidName, None)));
        assert_eq!(err("/l"), Some((ErrorCode::Unsupported, Some("reparse_point".into()))));
        assert_eq!(err("/enc"), Some((ErrorCode::Unsupported, Some("encrypted".into()))));
        assert!(!p.stat("/l").unwrap().is_dir);
        assert_eq!(read_all(&*p, "/ok"), b"data");
        assert_eq!(p.mkdir("/new").unwrap_err().code, ErrorCode::Unsupported);

        // TAR GNU sparse: la dimensione è quella vera, i dati no.
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::GNUSparse);
        h.set_size(512);
        h.set_path("s").unwrap();
        h.as_mut_bytes()[483..495].copy_from_slice(b"00000011610\0");
        h.set_cksum();
        let t = d.path().join("s.tar");
        std::fs::write(&t, [h.as_bytes().as_slice(), &[9; 512], &[0; 1024]].concat()).unwrap();
        let (p, _) = v.remote(&at(&Loc::Local(t)), Purpose::Browse).unwrap();
        assert_eq!(p.stat("/s").unwrap().size, 5000);
        let e = p.reader("/s").err().unwrap();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("sparse")));
    }
}
