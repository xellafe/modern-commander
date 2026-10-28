//! Elenco dell'indice di ZIP, 7z, TAR e TAR.GZ. Solo lettura dell'indice (mai estrazione); l'archivio non
//! è fidato: nomi e dimensioni assurdi si rifiutano o si limitano, le voci non superano `limit`.

use super::container::open_local;
use super::index::{Index, build_7z, build_zip};
use super::tar_walk::{skip_read, skip_seek, walk_tar};
use super::{ArchiveEntry, ArchiveFormat, ArchiveListing, format_of};
use crate::error::{AppError, ErrorCode};
use crate::fs::content::open_shared;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Nome di voce più lungo accettato (byte); oltre, l'archivio si considera danneggiato.
const MAX_NAME: usize = 32 * 1024;

/// Oltre 2^53 - 1 il frontend (numero JS) non rappresenta più le dimensioni con esattezza.
const MAX_SIZE: u64 = (1 << 53) - 1;

pub(super) fn corrupt(path: &Path, detail: impl ToString) -> AppError {
    AppError { detail: Some(detail.to_string()), ..AppError::new(ErrorCode::CorruptArchive, Some(path)) }
}

/// Errore di lettura: quello del provider del contenitore o del sistema (accesso negato, disco...) com'è,
/// altrimenti formato non valido.
pub(super) fn io_err(e: &io::Error, path: &Path) -> AppError {
    if let Some(e) = e.get_ref().and_then(|e| e.downcast_ref::<AppError>()) {
        return e.clone();
    }
    if e.raw_os_error().is_some() { AppError::from_io(e, path) } else { corrupt(path, e) }
}

/// Lettore che si interrompe quando la richiesta viene annullata, anche dentro una voce enorme da saltare.
pub(super) struct Cancellable<'a, R>(R, &'a AtomicBool);

impl<'a, R> Cancellable<'a, R> {
    pub(super) fn new(r: R, cancel: &'a AtomicBool) -> Self {
        Self(r, cancel)
    }
}

impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.1.load(Ordering::Relaxed) {
            // `Interrupted` verrebbe ritentato da `read_exact`.
            return Err(io::Error::other("annullato"));
        }
        self.0.read(buf)
    }
}

impl<R: io::Seek> io::Seek for Cancellable<'_, R> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

/// Voce con percorso a `\`, senza `./` iniziale né barra finale; `None` per la radice (`./`).
pub(super) fn entry(raw: &str, is_dir: bool, size: u64, modified_ms: Option<i64>, archive: &Path) -> Result<Option<ArchiveEntry>, AppError> {
    if raw.len() > MAX_NAME {
        return Err(corrupt(archive, "nome troppo lungo"));
    }
    let p = raw.replace('/', "\\");
    let p = p.strip_prefix(".\\").unwrap_or(&p).trim_end_matches('\\');
    if p.is_empty() || p == "." {
        return Ok(None);
    }
    Ok(Some(ArchiveEntry { path: p.to_string(), is_dir, size: size.min(MAX_SIZE), modified_ms }))
}

/// Elenca le prime `limit` voci di `path`; il formato si decide dall'estensione.
pub fn list_archive(path: &Path, limit: usize, cancel: &AtomicBool) -> Result<ArchiveListing, AppError> {
    let format = format_of(path).ok_or_else(|| AppError::new(ErrorCode::Unsupported, Some(path)))?;
    let file = || open_shared(path).map_err(|e| AppError::from_io(&e, path));
    let r = match format {
        ArchiveFormat::Zip => build_zip(open_local(path)?, cancel).and_then(|(index, _)| list_index(&index, path, limit)),
        ArchiveFormat::SevenZ => build_7z(open_local(path)?, cancel).and_then(|(index, _)| list_index(&index, path, limit)),
        ArchiveFormat::Tar => list_tar(file()?, false, path, limit, cancel),
        ArchiveFormat::TarGz => list_tar(file()?, true, path, limit, cancel),
    };
    // Un errore a richiesta annullata è l'annullamento, non un archivio rotto.
    r.map_err(|e| if cancel.load(Ordering::Relaxed) { AppError::new(ErrorCode::Cancelled, Some(path)) } else { e })
}

/// Le prime `limit` voci dell'indice (ZIP e 7z), senza le cartelle implicite.
fn list_index(index: &Index, path: &Path, limit: usize) -> Result<ArchiveListing, AppError> {
    let all = index.entries();
    let mut entries = Vec::with_capacity(all.len().min(limit));
    for e in all.iter().take(limit) {
        entries.extend(entry(&e.path[1..], e.is_dir, e.size, e.modified_ms, path)?);
    }
    Ok(ArchiveListing { entries, total: Some(all.len() as u64), truncated: all.len() > limit })
}

/// Data e ora DOS (ora locale di chi ha scritto l'archivio, senza fuso) come millisecondi Unix, lette nel fuso di
/// questo PC come fanno Explorer e 7-Zip. `None` se Windows non la converte.
pub(super) fn dos_to_ms(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<i64> {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::Time::TzSpecificLocalTimeToSystemTime;
    let (month, day, hour, minute, second) = (month.into(), day.into(), hour.into(), minute.into(), second.into());
    let local = SYSTEMTIME { wYear: year, wMonth: month, wDay: day, wHour: hour, wMinute: minute, wSecond: second, ..Default::default() };
    let mut utc = SYSTEMTIME::default();
    unsafe { TzSpecificLocalTimeToSystemTime(None, &local, &mut utc) }.ok()?;
    Some(utc_to_ms(utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute, utc.wSecond))
}

/// Data e ora UTC del calendario gregoriano come millisecondi Unix.
fn utc_to_ms(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> i64 {
    // Giorni dall'epoca (Howard Hinnant, `days_from_civil`).
    let (y, m, d) = (i64::from(year) - i64::from(month <= 2), i64::from(month), i64::from(day));
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)) * 1000
}

/// TAR e TAR.GZ da flusso (vedi `tar_walk`): si legge solo fino a `limit` voci. Il TAR su file salta i contenuti
/// con `seek`, il TAR.GZ li decomprime e scarta a blocchi: mai tutto in memoria.
fn list_tar(file: std::fs::File, gz: bool, path: &Path, limit: usize, cancel: &AtomicBool) -> Result<ArchiveListing, AppError> {
    let mut reader = Cancellable(BufReader::new(file), cancel);
    if gz {
        walk_tar(&mut flate2::read::MultiGzDecoder::new(reader), skip_read, path, limit)
    } else {
        walk_tar(&mut reader, skip_seek, path, limit)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::io::Write;

    fn no() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Contenuto comune: cartella `d/`, `d/x.txt` (3 byte), `y.bin` (5 byte).
    fn zip_file(dir: &Path, n: usize) -> std::path::PathBuf {
        let p = dir.join("a.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let all: [(&str, &[u8]); 3] = [("d/", b""), ("d/x.txt", b"abc"), ("y.bin", b"12345")];
        for (name, data) in all.iter().take(n) {
            if name.ends_with('/') {
                w.add_directory(*name, o).unwrap();
            } else {
                w.start_file(*name, o).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
        p
    }

    pub(crate) fn tar_into<W: Write>(w: W) -> W {
        let mut b = tar::Builder::new(w);
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_size(0);
        h.set_mode(0o755);
        b.append_data(&mut h, "d/", std::io::empty()).unwrap();
        for (name, data) in [("d/x.txt", &b"abc"[..]), ("y.bin", &b"12345"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            b.append_data(&mut h, name, data).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn sevenz_file(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("a.7z");
        let mut w = sevenz_rust2::ArchiveWriter::create(&p).unwrap();
        w.push_archive_entry::<&[u8]>(sevenz_rust2::ArchiveEntry::new_directory("d"), None).unwrap();
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file("d/x.txt"), Some(&b"abc"[..])).unwrap();
        w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file("y.bin"), Some(&b"12345"[..])).unwrap();
        w.finish().unwrap();
        p
    }

    fn expect_three(p: &Path) {
        let l = list_archive(p, 100, &no()).unwrap();
        let got: Vec<_> = l.entries.iter().map(|e| (e.path.as_str(), e.is_dir, e.size)).collect();
        assert_eq!(got, [("d", true, 0), ("d\\x.txt", false, 3), ("y.bin", false, 5)], "{}", p.display());
        assert!(!l.truncated);
        assert_eq!(l.total, Some(3));
    }

    #[test]
    fn lists_zip_tar_tgz_and_7z() {
        let d = tempfile::tempdir().unwrap();
        expect_three(&zip_file(d.path(), 3));
        expect_three(&sevenz_file(d.path()));
        let t = d.path().join("a.tar");
        tar_into(std::fs::File::create(&t).unwrap());
        expect_three(&t);
        let g = d.path().join("a.tar.gz");
        tar_into(flate2::write::GzEncoder::new(std::fs::File::create(&g).unwrap(), flate2::Compression::fast()))
            .finish()
            .unwrap();
        expect_three(&g);
    }

    #[test]
    fn limit_and_corruption() {
        let d = tempfile::tempdir().unwrap();
        let z = zip_file(d.path(), 3);
        let l = list_archive(&z, 1, &no()).unwrap();
        assert_eq!((l.entries.len(), l.truncated, l.total), (1, true, Some(3)));
        let t = d.path().join("a.tar");
        tar_into(std::fs::File::create(&t).unwrap());
        let l = list_archive(&t, 1, &no()).unwrap();
        assert_eq!((l.entries.len(), l.truncated, l.total), (1, true, None));
        let junk = d.path().join("junk.zip");
        std::fs::write(&junk, (0..100u8).map(|i| i.wrapping_mul(37).wrapping_add(11)).collect::<Vec<_>>()).unwrap();
        assert_eq!(list_archive(&junk, 10, &no()).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn encrypted_7z_header_is_unsupported_and_other_extensions_too() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, Password, encoder_options::{AesEncoderOptions, Lzma2Options}};
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("enc.7z");
        let mut w = ArchiveWriter::create(&p).unwrap();
        w.set_content_methods(vec![AesEncoderOptions::new(Password::from("pw")).into(), Lzma2Options::from_level(1).into()]);
        w.set_encrypt_header(true);
        w.push_archive_entry(ArchiveEntry::new_file("x"), Some(&b"abc"[..])).unwrap();
        w.finish().unwrap();
        let e = list_archive(&p, 10, &no()).unwrap_err();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("encrypted")));
        let rar = d.path().join("a.rar");
        std::fs::write(&rar, b"Rar!").unwrap();
        assert_eq!(list_archive(&rar, 10, &no()).unwrap_err().code, ErrorCode::Unsupported);
    }

    #[test]
    fn cancelled_flag_aborts_every_format() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("a.tar");
        tar_into(std::fs::File::create(&t).unwrap());
        let g = d.path().join("a.tgz");
        tar_into(flate2::write::GzEncoder::new(std::fs::File::create(&g).unwrap(), flate2::Compression::fast())).finish().unwrap();
        for p in [zip_file(d.path(), 3), sevenz_file(d.path()), t, g] {
            assert_eq!(list_archive(&p, 10, &AtomicBool::new(true)).unwrap_err().code, ErrorCode::Cancelled, "{}", p.display());
        }
    }

    #[test]
    fn cancelled_7z_stops_while_reading_the_headers() {
        // Senza voci il ciclo sulle voci non controlla mai il flag: solo la lettura annullabile può fermarsi.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("empty.7z");
        sevenz_rust2::ArchiveWriter::create(&p).unwrap().finish().unwrap();
        assert!(list_archive(&p, 10, &no()).unwrap().entries.is_empty());
        assert_eq!(list_archive(&p, 10, &AtomicBool::new(true)).unwrap_err().code, ErrorCode::Cancelled);
    }

    #[test]
    fn entry_cleans_paths_and_guards_names_and_sizes() {
        let a = Path::new("a.tar");
        let e = |raw: &str| entry(raw, false, u64::MAX, None, a);
        assert_eq!(e("./d/x/").unwrap().unwrap(), ArchiveEntry { path: "d\\x".into(), is_dir: false, size: (1 << 53) - 1, modified_ms: None });
        assert!(e("./").unwrap().is_none());
        assert_eq!(e(&"n".repeat(MAX_NAME + 1)).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn utc_dates_become_unix_ms() {
        assert_eq!(utc_to_ms(2020, 1, 2, 3, 4, 6), 1_577_934_246_000);
        assert_eq!(utc_to_ms(1980, 1, 1, 0, 0, 0), 315_532_800_000);
    }

    fn gz_file(dir: &Path, name: &str, body: impl FnOnce(&mut flate2::write::GzEncoder<std::fs::File>)) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut g = flate2::write::GzEncoder::new(std::fs::File::create(&p).unwrap(), flate2::Compression::fast());
        body(&mut g);
        g.finish().unwrap();
        p
    }

    /// Intestazione con `tar::Header` (somma di controllo inclusa) per costruire voci speciali.
    fn special(kind: tar::EntryType, name: &str, size: u64) -> [u8; 512] {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(kind);
        h.set_path(name).unwrap();
        h.set_size(size);
        h.set_cksum();
        *h.as_bytes()
    }

    #[test]
    fn tgz_with_a_huge_long_name_lists_without_buffering_it() {
        // 32 MiB di zeri dichiarati come nome lungo GNU: compressi pesano pochi KB, ma `tar` li leggerebbe in un
        // `Vec` (con un valore da GB, memoria esaurita). Qui si scartano a blocchi e vale il nome dell'intestazione.
        const HUGE: u64 = 32 << 20;
        let d = tempfile::tempdir().unwrap();
        let p = gz_file(d.path(), "bomb.tar.gz", |g| {
            g.write_all(&special(tar::EntryType::GNULongName, "././@LongLink", HUGE)).unwrap();
            let zeros = vec![0u8; 1 << 20];
            for _ in 0..HUGE >> 20 {
                g.write_all(&zeros).unwrap();
            }
            let mut b = tar::Builder::new(&mut *g);
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_mode(0o644);
            b.append_data(&mut h, "ok.txt", &b"hi"[..]).unwrap();
            b.finish().unwrap();
        });
        assert!(std::fs::metadata(&p).unwrap().len() < 1 << 20);
        let l = list_archive(&p, 10, &no()).unwrap();
        assert_eq!(l.entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["ok.txt"]);
    }

    #[test]
    fn pax_global_header_is_not_listed_in_tar_and_tgz() {
        let d = tempfile::tempdir().unwrap();
        let body = |w: &mut dyn Write| {
            let rec = b"52 comment=0123456789abcdef0123456789abcdef01234567
";
            w.write_all(&special(tar::EntryType::XGlobalHeader, "pax_global_header", rec.len() as u64)).unwrap();
            let mut pad = rec.to_vec();
            pad.resize(512, 0);
            w.write_all(&pad).unwrap();
            let mut b = tar::Builder::new(w);
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mode(0o644);
            b.append_data(&mut h, "a.txt", &b"abc"[..]).unwrap();
            b.finish().unwrap();
        };
        let t = d.path().join("g.tar");
        body(&mut std::fs::File::create(&t).unwrap());
        let g = gz_file(d.path(), "g.tgz", |w| body(w));
        for p in [t, g] {
            let l = list_archive(&p, 10, &no()).unwrap();
            assert_eq!(l.entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["a.txt"], "{}", p.display());
            assert_eq!(l.total, Some(1));
        }
    }

    #[test]
    fn broken_tar_and_tgz_are_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let junk: Vec<u8> = (0..700u32).map(|i| (i.wrapping_mul(37) + 11) as u8).collect();
        let t = d.path().join("junk.tar");
        std::fs::write(&t, &junk).unwrap();
        let wrapped = gz_file(d.path(), "wrapped.tgz", |g| g.write_all(&junk).unwrap());
        let good = gz_file(d.path(), "good.tgz", |g| {
            tar_into(&mut *g);
        });
        let bytes = std::fs::read(&good).unwrap();
        let cut = d.path().join("cut.tgz");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let nogz = d.path().join("nogz.tgz");
        std::fs::write(&nogz, &junk).unwrap();
        for p in [t, wrapped, cut, nogz] {
            assert_eq!(list_archive(&p, 100, &no()).unwrap_err().code, ErrorCode::CorruptArchive, "{}", p.display());
        }
    }
}
