//! Indice in memoria di un archivio: voci con percorso POSIX pulito, cartelle implicite e posizione dei dati. Si
//! costruisce una volta leggendo il contenitore (vedi `container`) e risponde a `stat` e `list` senza rileggerlo.

use super::ArchiveFormat;
use super::container::ContainerFile;
use super::list::{Cancellable, corrupt, dos_to_ms, io_err};
use super::tar_walk::{TarWalker, skip_read, skip_seek};
use crate::error::{AppError, ErrorCode};
use crate::vfs::provider::Stat;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Voci per archivio oltre cui l'indice non si costruisce.
const MAX_ENTRIES: usize = 1_000_000;

/// Byte di nomi oltre cui l'indice di un TAR non si costruisce: un TAR.GZ di pochi MB può dichiarare milioni di
/// nomi lunghi `L` da 64 KiB. Negli ZIP i nomi stanno nella directory centrale, quindi nel file.
const MAX_NAME_BYTES: usize = 128 << 20;

#[derive(Debug, Clone)]
pub struct IndexEntry {
    /// POSIX con `/` iniziale, pulito da `normalize_name`.
    pub path: String,
    pub is_dir: bool,
    /// Collegamento (simbolico o fisico): si mostra come file e non si estrae.
    pub link: bool,
    /// Il nome originale risaliva (`..`), era assoluto o aveva una lettera di unità.
    pub bad_name: bool,
    pub size: u64,
    pub modified_ms: Option<i64>,
    /// Posizione nell'archivio.
    pub order: u64,
    pub at: EntryAt,
}

/// Dove stanno i dati di una voce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryAt {
    /// Indice nello `ZipArchive`.
    Zip(usize),
    SevenZ { file: usize, block: Option<usize> },
    /// Inizio dei dati nel contenitore.
    Tar { data: u64 },
    /// Si raggiunge solo decomprimendo dall'inizio (vedi `order`).
    TarGz,
}

#[derive(Debug)]
pub struct Index {
    pub format: ArchiveFormat,
    /// Le voci si leggono solo decomprimendo dall'inizio di un blocco condiviso (7z solido, TAR.GZ).
    pub solid: bool,
    entries: Vec<IndexEntry>,
    by_path: HashMap<String, usize>,
    /// Nomi dei figli di ogni cartella, anche implicita, nell'ordine di prima apparizione.
    children: HashMap<String, Vec<String>>,
}

/// Cartella madre e nome di un percorso pulito diverso da `/`.
fn split(path: &str) -> (&str, &str) {
    let i = path.rfind('/').unwrap_or(0);
    (if i == 0 { "/" } else { &path[..i] }, &path[i + 1..])
}

impl Index {
    /// Indice delle voci in ordine d'archivio: tra percorsi uguali vince l'ultima voce, la radice non si elenca.
    fn new(format: ArchiveFormat, solid: bool, mut entries: Vec<IndexEntry>) -> Index {
        entries.retain(|e| e.path != "/");
        let mut by_path: HashMap<String, usize> = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_path.insert(e.path.clone(), i);
        }
        let mut i = 0;
        entries.retain(|e| {
            i += 1;
            by_path[&e.path] == i - 1
        });
        let mut children: HashMap<String, Vec<String>> = HashMap::from([("/".to_owned(), Vec::new())]);
        let mut listed = HashSet::new();
        for (i, e) in entries.iter().enumerate() {
            *by_path.get_mut(&e.path).expect("voce indicizzata") = i;
            if e.is_dir {
                children.entry(e.path.clone()).or_default();
            }
            // La voce e le cartelle che la contengono, ognuna sotto la sua madre la prima volta che compare.
            let mut p = e.path.as_str();
            while p != "/" && listed.insert(p) {
                let (parent, name) = split(p);
                children.entry(parent.to_owned()).or_default().push(name.to_owned());
                p = parent;
            }
        }
        Index { format, solid, entries, by_path, children }
    }

    pub fn get(&self, path: &str) -> Option<&IndexEntry> {
        self.by_path.get(path).map(|&i| &self.entries[i])
    }

    /// Voce o cartella implicita; la radice `/` è sempre una cartella.
    pub fn stat(&self, path: &str) -> Option<Stat> {
        let name = if path == "/" { String::new() } else { split(path).1.to_owned() };
        let stat = |is_dir, size, modified_ms| Stat { name, is_dir, dir_link: false, size, modified_ms, mode: None };
        match self.get(path) {
            Some(e) => Some(stat(e.is_dir, e.size, e.modified_ms)),
            None => self.children.contains_key(path).then(|| stat(true, 0, None)),
        }
    }

    /// Contenuto di una cartella; `None` se `dir` non esiste o non è una cartella.
    pub fn list(&self, dir: &str) -> Option<Vec<Stat>> {
        if !self.stat(dir)?.is_dir {
            return None;
        }
        let base = if dir == "/" { "" } else { dir };
        let names = self.children.get(dir).map(Vec::as_slice).unwrap_or_default();
        Some(names.iter().filter_map(|n| self.stat(&format!("{base}/{n}"))).collect())
    }

    /// Le voci in ordine d'archivio.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

/// Percorso POSIX pulito (`\` come separatore, niente `./`, `.`, barre doppie o finali, `..` risolti senza uscire
/// dalla radice) e `bad_name`: il nome originale risaliva, era assoluto o iniziava con una lettera di unità.
pub(crate) fn normalize_name(raw: &str) -> (String, bool) {
    let s = raw.replace('\\', "/");
    let b = s.as_bytes();
    let mut bad = s.starts_with('/') || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic());
    let mut parts: Vec<&str> = Vec::new();
    for c in s.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                bad = true;
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    (format!("/{}", parts.join("/")), bad)
}

fn too_many(path: &Path) -> AppError {
    AppError { detail: Some("too_many_entries".into()), ..AppError::new(ErrorCode::Unsupported, Some(path)) }
}

/// Un errore a richiesta annullata è l'annullamento, non un archivio rotto.
fn unless_cancelled<T>(r: Result<T, AppError>, cancel: &AtomicBool, path: &Path) -> Result<T, AppError> {
    r.map_err(|e| if cancel.load(Ordering::Relaxed) { AppError::new(ErrorCode::Cancelled, Some(path)) } else { e })
}

/// Indice di uno ZIP dalla directory centrale, senza leggere le intestazioni locali. Le voci cifrate restano: l'errore
/// arriva alla lettura.
pub fn build_zip(file: ContainerFile, cancel: &AtomicBool) -> Result<(Index, zip::ZipArchive<ContainerFile>), AppError> {
    let name = file.name().to_path_buf();
    let path = name.as_path();
    let zip_err = |e: zip::result::ZipError| match e {
        zip::result::ZipError::Io(e) => io_err(&e, path),
        e => corrupt(path, e),
    };
    let check = || if cancel.load(Ordering::Relaxed) { Err(AppError::new(ErrorCode::Cancelled, Some(path))) } else { Ok(()) };
    let build = || {
        check()?;
        let mut central = file.clone();
        let archive = zip::ZipArchive::new(file).map_err(zip_err)?;
        if archive.len() > MAX_ENTRIES {
            return Err(too_many(path));
        }
        let mut entries = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            check()?;
            let e = archive.by_index_data(i).map_err(zip_err)?;
            // Senza il flag UTF-8 il crate legge i nomi in CP437.
            let raw = e.name().map(|n| n.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(e.name_raw()).into_owned());
            let (p, bad_name) = normalize_name(&raw);
            let dos = || e.last_modified().filter(|d| d.is_valid()).and_then(|d| dos_to_ms(d.year(), d.month(), d.day(), d.hour(), d.minute(), d.second()));
            let modified_ms = unix_mtime(&mut central, e.central_header_start()).map_err(|e| io_err(&e, path))?.or_else(dos);
            let link = e.is_symlink();
            let is_dir = e.is_dir() && !link;
            entries.push(IndexEntry { path: p, is_dir, link, bad_name, size: e.size(), modified_ms, order: i as u64, at: EntryAt::Zip(i) });
        }
        Ok((Index::new(ArchiveFormat::Zip, false, entries), archive))
    };
    unless_cancelled(build(), cancel, path)
}

/// Data Unix del campo esteso UT (`0x5455`) nell'intestazione centrale che inizia a `at`. Il crate la espone solo
/// in `ZipFile::extra_data_fields`, cioè dopo aver letto l'intestazione locale: una lettura (remota) per voce.
// ponytail: la directory centrale si legge due volte (il crate e qui); se pesa sul remoto, un solo passaggio a mano.
fn unix_mtime(r: &mut ContainerFile, at: u64) -> io::Result<Option<i64>> {
    let mut head = [0u8; 46];
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut head)?;
    if head[..4] != *b"PK\x01\x02" {
        return Ok(None);
    }
    let le16 = |i: usize| u16::from_le_bytes([head[i], head[i + 1]]);
    r.seek(SeekFrom::Current(i64::from(le16(28))))?;
    let mut extra = vec![0u8; usize::from(le16(30))];
    r.read_exact(&mut extra)?;
    let mut d = extra.as_slice();
    while d.len() >= 4 {
        let (id, len) = (u16::from_le_bytes([d[0], d[1]]), usize::from(u16::from_le_bytes([d[2], d[3]])));
        let Some(body) = d.get(4..4 + len) else { break };
        // Nella directory centrale c'è al più la data di modifica, dopo i flag.
        if id == 0x5455 && body.len() >= 5 && body[0] & 1 != 0 {
            return Ok(Some(i64::from(u32::from_le_bytes([body[1], body[2], body[3], body[4]])) * 1000));
        }
        d = &d[4 + len..];
    }
    Ok(None)
}

/// Indice di un TAR (`gz` falso: i dati si saltano e restano raggiungibili a salti) o di un TAR.GZ (si decomprime
/// tutto, le voci si raggiungono solo dall'inizio).
pub fn build_tar(file: ContainerFile, gz: bool, cancel: &AtomicBool) -> Result<Index, AppError> {
    let name = file.name().to_path_buf();
    let path = name.as_path();
    let r = if gz {
        let r = flate2::read::MultiGzDecoder::new(Cancellable::new(file, cancel));
        index_tar(TarWalker::new(r, skip_read, path), ArchiveFormat::TarGz, path)
    } else {
        index_tar(TarWalker::new(Cancellable::new(file, cancel), skip_seek, path), ArchiveFormat::Tar, path)
    };
    unless_cancelled(r, cancel, path)
}

fn index_tar<R: Read>(mut walker: TarWalker<R>, format: ArchiveFormat, path: &Path) -> Result<Index, AppError> {
    let (mut entries, mut names) = (Vec::new(), 0usize);
    while let Some(h) = walker.next_header()? {
        names += h.name.len();
        if entries.len() == MAX_ENTRIES || names > MAX_NAME_BYTES {
            return Err(too_many(path));
        }
        let (p, bad_name) = normalize_name(&h.name);
        let link = matches!(h.flag, b'1' | b'2');
        let at = if format == ArchiveFormat::TarGz { EntryAt::TarGz } else { EntryAt::Tar { data: h.data_offset } };
        let order = entries.len() as u64;
        entries.push(IndexEntry { path: p, is_dir: h.is_dir() && !link, link, bad_name, size: h.shown_size, modified_ms: h.modified_ms, order, at });
    }
    Ok(Index::new(format, format == ArchiveFormat::TarGz, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::container;
    use crate::error::ErrorCode;
    use crate::vfs::loc::Loc;
    use crate::vfs::mem::MemConnector;
    use std::collections::{HashMap, HashSet};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::Path;
    use std::sync::Arc;

    fn no() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn file(p: &Path) -> ContainerFile {
        let connector: Arc<dyn crate::vfs::provider::Connector> = Arc::new(MemConnector(HashMap::new()));
        container::open(&connector, &Loc::Local(p.to_path_buf())).unwrap()
    }

    /// ZIP "stored" con le voci date (una cartella finisce con `/`).
    fn zip_bytes(entries: &[(&str, &[u8])], o: zip::write::SimpleFileOptions) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = o.compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, o).unwrap();
            } else {
                w.start_file(*name, o).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    /// Sostituisce ogni `from` con `to` (stessa lunghezza): nomi che `ZipWriter` non scriverebbe.
    fn patch(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let mut i = 0;
        while i + from.len() <= bytes.len() {
            if &bytes[i..i + from.len()] == from {
                bytes[i..i + from.len()].copy_from_slice(to);
            }
            i += 1;
        }
    }

    fn zip_index(dir: &Path, bytes: &[u8]) -> Index {
        let p = dir.join("t.zip");
        std::fs::write(&p, bytes).unwrap();
        build_zip(file(&p), &no()).unwrap().0
    }

    #[test]
    fn implicit_dirs_backslashes_and_cp437_names() {
        let d = tempfile::tempdir().unwrap();
        let mut z = zip_bytes(&[("a\\b\\c.txt", b"abc"), ("x/y.txt", b"y"), ("Q.txt", b"q")], Default::default());
        patch(&mut z, b"Q.txt", b"\x82.txt"); // CP437, senza flag UTF-8
        let ix = zip_index(d.path(), &z);
        let names: HashSet<_> = ix.list("/").unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, HashSet::from(["a".to_owned(), "x".to_owned(), "é.txt".to_owned()]));
        assert!(ix.stat("/a/b").unwrap().is_dir);
        assert!(ix.stat("/").unwrap().is_dir);
        assert_eq!(ix.get("/a/b/c.txt").unwrap().size, 3);
        assert_eq!(ix.list("/a/b").unwrap().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["c.txt"]);
        assert!(ix.list("/a/b/c.txt").is_none());
        assert!(ix.stat("/nope").is_none());
    }

    #[test]
    fn duplicates_last_wins_and_bad_names_flagged() {
        let d = tempfile::tempdir().unwrap();
        let mut z = zip_bytes(
            &[("d/f", b"1"), ("d/g", b"22"), ("../up", b""), ("/abs", b""), ("C:/drv", b"")],
            Default::default(),
        );
        patch(&mut z, b"d/g", b"d/f"); // `ZipWriter` rifiuta i nomi doppi
        let ix = zip_index(d.path(), &z);
        assert_eq!(ix.get("/d/f").unwrap().size, 2);
        assert!(!ix.get("/d/f").unwrap().bad_name);
        assert!(ix.get("/up").unwrap().bad_name);
        assert!(ix.get("/abs").unwrap().bad_name);
        assert!(ix.get("/C:/drv").unwrap().bad_name);
    }

    /// Millisecondi Unix dell'ora locale data, convertita come fa Windows.
    fn local_ms(y: u16, mo: u16, d: u16, h: u16) -> i64 {
        use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
        use windows::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
        let local = SYSTEMTIME { wYear: y, wMonth: mo, wDay: d, wHour: h, ..Default::default() };
        let (mut utc, mut ft) = (SYSTEMTIME::default(), FILETIME::default());
        unsafe {
            TzSpecificLocalTimeToSystemTime(None, &local, &mut utc).unwrap();
            SystemTimeToFileTime(&utc, &mut ft).unwrap();
        }
        ((u64::from(ft.dwHighDateTime) << 32 | u64::from(ft.dwLowDateTime)) / 10_000) as i64 - 11_644_473_600_000
    }

    #[test]
    fn zip_dos_time_is_local() {
        let d = tempfile::tempdir().unwrap();
        let t = zip::DateTime::from_date_and_time(2026, 7, 1, 12, 0, 0).unwrap();
        let ix = zip_index(d.path(), &zip_bytes(&[("f", b"x")], zip::write::SimpleFileOptions::default().last_modified_time(t)));
        assert_eq!(ix.get("/f").unwrap().modified_ms, Some(local_ms(2026, 7, 1, 12)));
    }

    #[test]
    fn zip_extended_timestamp_wins_over_dos_time() {
        let d = tempfile::tempdir().unwrap();
        let mut o = zip::write::FullFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut ut = vec![1u8];
        ut.extend(1_700_000_000u32.to_le_bytes());
        o.add_extra_field(0x5455, ut, false).unwrap();
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("f", o).unwrap();
        w.write_all(b"x").unwrap();
        let ix = zip_index(d.path(), &w.finish().unwrap().into_inner());
        assert_eq!(ix.get("/f").unwrap().modified_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn normalize_name_cleans_and_flags() {
        assert_eq!(normalize_name("./a\\b/"), ("/a/b".to_owned(), false));
        assert_eq!(normalize_name("a/../../b"), ("/b".to_owned(), true));
        assert_eq!(normalize_name("/x//y"), ("/x/y".to_owned(), true));
        assert_eq!(normalize_name("c:x"), ("/c:x".to_owned(), true));
        assert_eq!(normalize_name("./"), ("/".to_owned(), false));
        assert_eq!(normalize_name("a..b/c."), ("/a..b/c.".to_owned(), false));
    }

    fn tar_header(kind: tar::EntryType, size: u64) -> tar::Header {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(kind);
        h.set_size(size);
        h.set_mode(0o644);
        h.set_mtime(1_700_000_000);
        h
    }

    #[test]
    fn tar_index_offsets_and_links() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.tar");
        let mut b = tar::Builder::new(std::fs::File::create(&p).unwrap());
        b.append_data(&mut tar_header(tar::EntryType::Regular, 5), "a", &b"hello"[..]).unwrap();
        let mut h = tar_header(tar::EntryType::Symlink, 0);
        h.set_link_name("a").unwrap();
        b.append_data(&mut h, "l", std::io::empty()).unwrap();
        b.append_data(&mut tar_header(tar::EntryType::Directory, 0), "d/", std::io::empty()).unwrap();
        b.into_inner().unwrap();
        let ix = build_tar(file(&p), false, &no()).unwrap();
        let a = ix.get("/a").unwrap();
        let EntryAt::Tar { data } = a.at else { panic!("{:?}", a.at) };
        let mut f = file(&p);
        f.seek(SeekFrom::Start(data)).unwrap();
        let mut got = vec![0; 5];
        f.read_exact(&mut got).unwrap();
        assert_eq!(got, b"hello");
        assert_eq!(a.modified_ms, Some(1_700_000_000_000));
        assert!(ix.get("/l").unwrap().link);
        assert!(!ix.stat("/l").unwrap().is_dir);
        assert!(ix.stat("/d").unwrap().is_dir);
        assert_eq!(ix.entries().iter().map(|e| e.order).collect::<Vec<_>>(), [0, 1, 2]);
        // Lo stesso archivio compresso: le voci si raggiungono solo decomprimendo dall'inizio.
        let g = d.path().join("t.tgz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&g).unwrap(), flate2::Compression::fast());
        enc.write_all(&std::fs::read(&p).unwrap()).unwrap();
        enc.finish().unwrap();
        let ix = build_tar(file(&g), true, &no()).unwrap();
        assert!(matches!(ix.get("/a").unwrap().at, EntryAt::TarGz));
        assert_eq!(ix.format, ArchiveFormat::TarGz);
    }

    #[test]
    fn tar_truncated_in_data_is_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let whole = crate::archive::list::tests::tar_into(Vec::new());
        // Ultima voce `y.bin`: intestazione a 1536, dati da 2048.
        let cut = &whole[..2050];
        let t = d.path().join("cut.tar");
        std::fs::write(&t, cut).unwrap();
        assert_eq!(build_tar(file(&t), false, &no()).unwrap_err().code, ErrorCode::CorruptArchive);
        let g = d.path().join("cut.tgz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&g).unwrap(), flate2::Compression::fast());
        enc.write_all(cut).unwrap();
        enc.finish().unwrap();
        assert_eq!(build_tar(file(&g), true, &no()).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn tar_duplicates_last_wins_and_multi_member_gzip_is_read() {
        let d = tempfile::tempdir().unwrap();
        let mut b = tar::Builder::new(Vec::new());
        b.append_data(&mut tar_header(tar::EntryType::Regular, 1), "f", &b"1"[..]).unwrap();
        b.append_data(&mut tar_header(tar::EntryType::Regular, 2), "./f", &b"22"[..]).unwrap();
        let bytes = b.into_inner().unwrap();
        // Due membri gzip: il secondo contiene la seconda metà del TAR.
        let g = d.path().join("m.tgz");
        let mut out = std::fs::File::create(&g).unwrap();
        for part in [&bytes[..1024], &bytes[1024..]] {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(part).unwrap();
            out.write_all(&enc.finish().unwrap()).unwrap();
        }
        drop(out);
        let ix = build_tar(file(&g), true, &no()).unwrap();
        assert_eq!(ix.entries().len(), 1);
        assert_eq!((ix.get("/f").unwrap().size, ix.get("/f").unwrap().order), (2, 1));
    }

    #[test]
    fn remote_container_is_read_through_its_provider() {
        use crate::vfs::mem::MemProvider;
        use crate::vfs::provider::Provider;
        let m = Arc::new(MemProvider::new());
        m.add_file("/a.zip", &zip_bytes(&[("d/f", b"abc")], Default::default()));
        let connector: Arc<dyn crate::vfs::provider::Connector> = Arc::new(MemConnector(HashMap::from([("m".to_owned(), m.clone())])));
        let loc = Loc::parse("sftp://m/a.zip").unwrap();
        let ix = build_zip(container::open(&connector, &loc).unwrap(), &no()).unwrap().0;
        assert_eq!(ix.get("/d/f").unwrap().size, 3);
        // L'errore del provider arriva com'è, non come archivio danneggiato.
        let f = container::open(&connector, &loc).unwrap();
        m.delete("/a.zip", false).unwrap();
        assert_eq!(build_zip(f, &no()).err().unwrap().code, ErrorCode::NotFound);
    }

    #[test]
    fn cancelled_build_is_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.zip");
        std::fs::write(&p, zip_bytes(&[("f", b"x")], Default::default())).unwrap();
        assert_eq!(build_zip(file(&p), &AtomicBool::new(true)).err().unwrap().code, ErrorCode::Cancelled);
        let t = d.path().join("t.tar");
        std::fs::write(&t, crate::archive::list::tests::tar_into(Vec::new())).unwrap();
        assert_eq!(build_tar(file(&t), false, &AtomicBool::new(true)).unwrap_err().code, ErrorCode::Cancelled);
    }
}
