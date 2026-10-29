//! Lettura dei file per visualizzatore, editor e ispettore: informazioni, blocchi di byte e lettura
//! intera con limite. I file si aprono in condivisione piena, così si leggono anche quelli che un altro
//! programma sta scrivendo (log, per esempio). Qui c'è anche il salvataggio del testo dell'editor.

use super::listing::FILETIME_UNIX_DIFF_MS;
use crate::archive::edit::ZipEdit;
use crate::error::{AppError, ErrorCode};
use crate::net::temp;
use crate::ops::model::{ConflictPolicy, JobKind, JobSpec};
use crate::ops::queue::{JobState, Queue};
use crate::ops::exec::{PART_SUFFIX, from_win, wide};
use crate::paths::{invalid_windows_name, to_extended};
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Provider, Purpose, stream_error};
use crate::vfs::router::Vfs;
use crate::vfs::transfer;
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use windows::Win32::Globalization::GetACP;
use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW};
use windows::core::PCWSTR;

/// Massimo di byte per una lettura a blocchi.
pub const MAX_CHUNK: u32 = 4 << 20;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub size: u64,
    pub is_dir: bool,
    /// Date in millisecondi dall'epoca Unix.
    pub created_ms: i64,
    pub modified_ms: i64,
    pub accessed_ms: i64,
    /// Attributi Win32 grezzi (`FILE_ATTRIBUTE_*`).
    pub attributes: u32,
    /// Date al secondo o meglio: sempre in locale, `Caps::precise_mtime` sul remoto.
    pub mtime_reliable: bool,
}

/// Apre in sola lettura senza escludere nessuno, anche con percorsi oltre i 260 caratteri.
pub(crate) fn open_shared(path: &Path) -> std::io::Result<File> {
    File::options().read(true).share_mode(SHARE_ALL).open(to_extended(path))
}

pub(crate) fn ms(filetime: u64) -> i64 {
    (filetime / 10_000) as i64 - FILETIME_UNIX_DIFF_MS
}

pub fn file_info(path: &Path) -> Result<FileInfo, AppError> {
    let m = std::fs::metadata(to_extended(path)).map_err(|e| AppError::from_io(&e, path))?;
    Ok(FileInfo {
        size: m.len(),
        is_dir: m.is_dir(),
        created_ms: ms(m.creation_time()),
        modified_ms: ms(m.last_write_time()),
        accessed_ms: ms(m.last_access_time()),
        attributes: m.file_attributes(),
        mtime_reliable: true,
    })
}

/// `file_info` di un elemento remoto: una sola data, ripetuta in creazione e accesso; niente attributi.
pub fn remote_file_info(p: &dyn Provider, path: &str) -> Result<FileInfo, AppError> {
    let s = p.stat(path)?;
    let t = s.modified_ms.unwrap_or(0);
    Ok(FileInfo {
        size: s.size,
        is_dir: s.is_dir,
        created_ms: t,
        modified_ms: t,
        accessed_ms: t,
        attributes: 0,
        mtime_reliable: p.caps().precise_mtime,
    })
}

/// `file_info` di una voce d'archivio. Le date DOS di uno ZIP sono a 2 s, ma il salvataggio dell'editor controlla
/// anche dimensione e data del contenitore: la data è affidabile quanto quella del contenitore (sempre in locale, la
/// precisione del server sul remoto), così F4 non chiede conferma come per un FTP senza `MDTM`.
pub fn archive_file_info(vfs: &Vfs, loc: &Loc) -> Result<FileInfo, AppError> {
    let Loc::Archive { container, .. } = loc else { return Err(AppError::new(ErrorCode::InvalidRequest, None)) };
    let (p, path) = vfs.remote(loc, Purpose::Browse)?;
    let info = remote_file_info(&*p, &path)?;
    let mtime_reliable = match &**container {
        Loc::Local(_) => true,
        c => vfs.remote(c, Purpose::Browse)?.0.caps().precise_mtime,
    };
    Ok(FileInfo { mtime_reliable, ..info })
}

/// Fino a `len` byte da `offset`; oltre la fine del file il risultato è più corto o vuoto.
pub fn read_chunk(path: &Path, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
    if len > MAX_CHUNK {
        return Err(AppError::new(ErrorCode::InvalidRequest, Some(path)));
    }
    let err = |e: std::io::Error| AppError::from_io(&e, path);
    let mut f = open_shared(path).map_err(err)?;
    f.seek(SeekFrom::Start(offset)).map_err(err)?;
    let mut buf = Vec::new();
    f.take(u64::from(len)).read_to_end(&mut buf).map_err(err)?;
    Ok(buf)
}

/// `read_chunk` di un file remoto.
pub fn remote_chunk(p: &dyn Provider, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
    if len > MAX_CHUNK {
        return Err(AppError::new(ErrorCode::InvalidRequest, Some(Path::new(path))));
    }
    p.read_at(path, offset, len)
}

/// `read_whole` di un file remoto.
pub fn remote_whole(p: &dyn Provider, path: &str, max: u64) -> Result<Vec<u8>, AppError> {
    let too_large = || AppError::new(ErrorCode::TooLarge, Some(Path::new(path)));
    if p.stat(path)?.size > max {
        return Err(too_large());
    }
    let mut buf = Vec::new();
    p.reader(path)?.take(max.saturating_add(1)).read_to_end(&mut buf).map_err(|e| stream_error(e, path))?;
    if buf.len() as u64 > max {
        return Err(too_large());
    }
    Ok(buf)
}

/// Tutto il file, se non supera `max` byte (`TOO_LARGE` altrimenti, anche se è cresciuto durante la lettura).
pub fn read_whole(path: &Path, max: u64) -> Result<Vec<u8>, AppError> {
    let err = |e: std::io::Error| AppError::from_io(&e, path);
    let f = open_shared(path).map_err(err)?;
    if f.metadata().map_err(err)?.len() > max {
        return Err(AppError::new(ErrorCode::TooLarge, Some(path)));
    }
    let mut buf = Vec::new();
    f.take(max.saturating_add(1)).read_to_end(&mut buf).map_err(err)?;
    if buf.len() as u64 > max {
        return Err(AppError::new(ErrorCode::TooLarge, Some(path)));
    }
    Ok(buf)
}

/// Etichetta WHATWG (quella di `TextDecoder`) della code page ANSI `acp`; sconosciuta: `windows-1252`.
pub fn encoding_for_acp(acp: u32) -> &'static str {
    match acp {
        874 => "windows-874",
        1250 => "windows-1250",
        1251 => "windows-1251",
        1252 => "windows-1252",
        1253 => "windows-1253",
        1254 => "windows-1254",
        1255 => "windows-1255",
        1256 => "windows-1256",
        1257 => "windows-1257",
        1258 => "windows-1258",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        65001 => "utf-8",
        _ => "windows-1252",
    }
}

pub fn ansi_code_page() -> u32 {
    unsafe { GetACP() }
}

/// Il testo nei byte della codifica `encoding` (etichetta WHATWG, come in `TextDecoder`); `bom` antepone
/// il segno d'ordine, ma solo per UTF-8 e UTF-16. `UNMAPPABLE_CHARS` se la codifica non rappresenta un carattere.
pub fn encode_text(text: &str, encoding: &str, bom: bool) -> Result<Vec<u8>, AppError> {
    let invalid = || AppError::new(ErrorCode::InvalidRequest, None);
    let enc = encoding_rs::Encoding::for_label(encoding.as_bytes()).ok_or_else(invalid)?;
    if enc == encoding_rs::REPLACEMENT {
        return Err(invalid()); // `encode` la tratterebbe come UTF-8: non è una codifica di scrittura
    }
    // `encoding_rs` codifica UTF-16 come UTF-8 (regola WHATWG): UTF-16 si scrive a mano.
    let utf16 = |le: bool| {
        text.encode_utf16()
            .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect::<Vec<u8>>()
    };
    let (mark, body): (&[u8], Vec<u8>) = if enc == encoding_rs::UTF_16LE {
        (&[0xff, 0xfe], utf16(true))
    } else if enc == encoding_rs::UTF_16BE {
        (&[0xfe, 0xff], utf16(false))
    } else {
        let (bytes, _, had_unmappable) = enc.encode(text);
        if had_unmappable {
            return Err(AppError::new(ErrorCode::UnmappableChars, None));
        }
        (if enc == encoding_rs::UTF_8 { &[0xef, 0xbb, 0xbf] } else { &[] }, bytes.into_owned())
    };
    Ok(if bom { [mark, &body].concat() } else { body })
}

/// `<file>.<8 cifre>.mcpart` nella stessa cartella (la sostituzione finale resta sullo stesso volume). Il nome
/// è unico per scrittura: un `.mcpart` di una copia in corso, di un altro salvataggio o dell'utente non è nostro.
fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{}{PART_SUFFIX}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    PathBuf::from(s)
}

/// Crea il `.mcpart` solo se non esiste (`create_new`): se c'è già non si tocca, e chi chiama non lo elimina.
fn create_part(part: &Path) -> Result<File, AppError> {
    File::options().write(true).create_new(true).open(to_extended(part)).map_err(|e| AppError::from_io(&e, part))
}

/// Il contenuto va su disco (non solo in cache) prima che il file vero venga toccato.
fn write_part(mut f: File, part: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let err = |e: std::io::Error| AppError::from_io(&e, part);
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)
}

/// Dopo un errore di `ReplaceFileW` il `.mcpart` resta se è l'unica copia dei dati: con 1176/1177
/// (`ERROR_UNABLE_TO_MOVE_REPLACEMENT`, `_2`) il file sostituito può non avere più il suo nome.
fn part_holds_the_data(hresult: i32) -> bool {
    matches!((hresult as u32) & 0xFFFF, 1176 | 1177)
}

/// `ReplaceFileW` conserva attributi, ACL e data di creazione del file sostituito; un file che non
/// esiste (o è sparito) prende il nome con `MoveFileExW` senza flag, che non sovrascrive mai.
/// In caso di errore dice anche se il `.mcpart` è l'unica copia dei dati (e quindi non va eliminato).
fn put_in_place(part: &Path, path: &Path, exists: bool) -> Result<(), (AppError, bool)> {
    if exists {
        unsafe { ReplaceFileW(&wide(path), &wide(part), PCWSTR::null(), REPLACE_FILE_FLAGS(0), None, None) }
    } else {
        unsafe { MoveFileExW(&wide(part), &wide(path), MOVE_FILE_FLAGS(0)) }
    }
    .map_err(|e| (from_win(&e, path), exists && part_holds_the_data(e.code().0)))
}

/// Salva il testo dell'editor. Mai sul posto: si codifica prima di toccare il disco, si scrive il
/// `.mcpart` e lo si sostituisce al file (eliminandolo se qualcosa va storto, ma solo se l'abbiamo creato noi).
/// Con `expected_modified_ms` il file deve avere ancora quella data, altrimenti `CHANGED_ON_DISK` senza
/// scrivere; senza, si sovrascrive.
pub fn write_text_file(path: &Path, text: &str, encoding: &str, bom: bool, expected_modified_ms: Option<i64>) -> Result<FileInfo, AppError> {
    let current = match file_info(path) {
        Ok(i) => Some(i),
        Err(e) if e.code == ErrorCode::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(t) = expected_modified_ms {
        if current.as_ref().map(|i| i.modified_ms) != Some(t) {
            return Err(AppError::new(ErrorCode::ChangedOnDisk, Some(path)));
        }
    }
    let bytes = encode_text(text, encoding, bom).map_err(|e| AppError { path: Some(path.to_string_lossy().into_owned()), ..e })?;
    let part = part_path(path);
    let file = create_part(&part)?; // non creato = niente da eliminare
    // ponytail: tra il controllo della data qui sopra e la sostituzione c'è una finestra di millisecondi in cui
    // un altro programma può ancora scrivere il file; chiuderla richiederebbe di tenerlo aperto in esclusiva.
    let done = write_part(file, &part, &bytes).map_err(|e| (e, false)).and_then(|()| put_in_place(&part, path, current.is_some()));
    if let Err((e, keep)) = done {
        if keep {
            // Il testo salvato è solo nel `.mcpart`: si lascia, e il suo nome si dice nel dettaglio dell'errore.
            let detail = format!("{}; {}", e.detail.as_deref().unwrap_or_default(), part.display());
            return Err(AppError { detail: Some(detail), ..e });
        }
        let _ = std::fs::remove_file(to_extended(&part));
        return Err(e);
    }
    file_info(path)
}

/// `write_text_file` di un file remoto. Con `expected_modified_ms` il file deve avere ancora la data (e, se data,
/// la dimensione) dell'apertura, altrimenti `CHANGED_ON_DISK`. Si scrive con la sequenza di `transfer::write`
/// (temporaneo e rinomina); se il server non la permette, `UNSAFE_WRITE` senza toccare il file, a meno di
/// `allow_unsafe` (l'utente ha confermato), che scrive direttamente sul file.
#[allow(clippy::too_many_arguments)]
pub fn remote_write_text_file(
    vfs: &Vfs,
    loc: &Loc,
    text: &str,
    encoding: &str,
    bom: bool,
    expected_modified_ms: Option<i64>,
    expected_size: Option<u64>,
    allow_unsafe: bool,
) -> Result<FileInfo, AppError> {
    let bytes = encode_text(text, encoding, bom).map_err(|e| AppError { path: Some(loc.to_string()), ..e })?;
    let (p, path) = vfs.remote(loc, Purpose::Browse)?;
    if let Some(t) = expected_modified_ms {
        let current = match p.stat(&path) {
            Ok(s) => Some(s),
            Err(e) if e.code == ErrorCode::NotFound => None,
            Err(e) => return Err(e),
        };
        // `unwrap_or(0)` come in `remote_file_info`, da cui l'editor ha preso la data.
        let same = current.is_some_and(|s| s.modified_ms.unwrap_or(0) == t && expected_size.is_none_or(|n| n == s.size));
        if !same {
            return Err(loc.error(ErrorCode::ChangedOnDisk));
        }
    }
    let mut refused = false;
    let mut confirm = |_: &Loc| {
        refused = !allow_unsafe;
        allow_unsafe
    };
    // ponytail: come in locale, tra il controllo qui sopra e la rinomina un altro client può ancora scrivere il file.
    match transfer::write(vfs, loc, true, &mut confirm, &mut |_| true, || Ok((bytes.as_slice(), None, path.clone()))) {
        Err(e) if e.code == ErrorCode::Cancelled && refused => return Err(loc.error(ErrorCode::UnsafeWrite)),
        r => r?,
    }
    remote_file_info(&*p, &path)
}

/// `write_text_file` di una voce dentro uno ZIP. Si codifica come per il remoto e, con `expected_modified_ms`, la
/// voce deve avere ancora quella data (e, se data, quella dimensione), altrimenti `CHANGED_ON_DISK` senza scrivere.
/// Poi il testo va in un temporaneo locale e un job della coda lo copia sulla voce (Sovrascrivi): la riscrittura
/// dell'archivio si mette in fila con gli altri job sullo stesso ZIP e si vede, con avanzamento e annullamento, nella
/// vista Operazioni. Si aspetta la fine del job: il suo errore (`CANCELLED` se annullato) è quello del salvataggio.
/// `allow_unsafe` non c'è: la riscrittura non scrive mai direttamente.
#[allow(clippy::too_many_arguments)]
pub fn archive_write_text_file(
    q: &Queue,
    vfs: &Vfs,
    loc: &Loc,
    text: &str,
    encoding: &str,
    bom: bool,
    expected_modified_ms: Option<i64>,
    expected_size: Option<u64>,
) -> Result<FileInfo, AppError> {
    let Loc::Archive { container, .. } = loc else { return Err(AppError::new(ErrorCode::InvalidRequest, None)) };
    // Solo ZIP: un altro formato dà `UNSUPPORTED` subito, non un job fallito.
    ZipEdit::open(vfs, container)?;
    let bytes = encode_text(text, encoding, bom).map_err(|e| AppError { path: Some(loc.to_string()), ..e })?;
    if let Some(t) = expected_modified_ms {
        // L'indice in cache può avere fino a 2 s: il confronto è con la voce com'è ora.
        vfs.archives().invalidate(container);
        let (p, inner) = vfs.remote(loc, Purpose::Browse)?;
        let current = match p.stat(&inner) {
            Ok(s) => Some(s),
            Err(e) if e.code == ErrorCode::NotFound => None,
            Err(e) => return Err(e),
        };
        // `unwrap_or(0)` come in `remote_file_info`, da cui l'editor ha preso la data.
        let same = current.is_some_and(|s| s.modified_ms.unwrap_or(0) == t && expected_size.is_none_or(|n| n == s.size));
        if !same {
            return Err(loc.error(ErrorCode::ChangedOnDisk));
        }
    }
    // Il temporaneo ha il nome della voce: è la sorgente che il job mostra.
    let dir = temp::dir().join(format!("save-{}", uuid::Uuid::new_v4().simple()));
    let name = loc.file_name().filter(|n| !invalid_windows_name(n)).unwrap_or_else(|| "testo".into());
    let staged = dir.join(name);
    let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&staged, &bytes)).map_err(|e| AppError::from_io(&e, &staged));
    let end = written.map(|()| {
        let spec = JobSpec { kind: JobKind::Copy, sources: vec![Loc::Local(staged)], dest: Some(loc.clone()), filter: None, policy: ConflictPolicy::Overwrite };
        q.enqueue_and_wait(spec)
    });
    let _ = std::fs::remove_dir_all(&dir);
    let end = end?;
    match (end.state, end.errors.into_iter().next()) {
        (JobState::Done, None) => {}
        (JobState::Cancelled, _) => return Err(loc.error(ErrorCode::Cancelled)),
        (_, e) => return Err(e.unwrap_or_else(|| loc.error(ErrorCode::Io))),
    }
    archive_file_info(vfs, loc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn chunk_reads_range_and_stops_at_eof() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.bin");
        std::fs::write(&f, b"0123456789").unwrap();
        assert_eq!(read_chunk(&f, 3, 4).unwrap(), b"3456");
        assert_eq!(read_chunk(&f, 8, 100).unwrap(), b"89");
        assert!(read_chunk(&f, 50, 10).unwrap().is_empty());
        assert_eq!(read_chunk(&f, 0, MAX_CHUNK + 1).unwrap_err().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn reads_a_file_another_program_is_writing() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("log.txt");
        std::fs::write(&f, b"riga").unwrap();
        let writer = std::fs::OpenOptions::new().append(true).share_mode(0x1 | 0x2).open(&f).unwrap(); // READ|WRITE
        assert_eq!(read_chunk(&f, 0, 4).unwrap(), b"riga");
        drop(writer);
        let _excl = std::fs::OpenOptions::new().read(true).share_mode(0).open(&f).unwrap();
        assert_eq!(read_chunk(&f, 0, 4).unwrap_err().code, ErrorCode::FileInUse);
    }

    #[test]
    fn whole_file_respects_the_limit() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ten.bin");
        std::fs::write(&f, b"0123456789").unwrap();
        let e = read_whole(&f, 9).unwrap_err();
        assert_eq!(e.code, ErrorCode::TooLarge);
        assert_eq!(e.path.as_deref(), f.to_str());
        assert_eq!(read_whole(&f, 10).unwrap(), b"0123456789");
        assert_eq!(read_whole(&f, 1000).unwrap().len(), 10);
    }

    #[test]
    fn info_has_times_and_attributes() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let i = file_info(&f).unwrap();
        assert_eq!((i.size, i.is_dir), (5, false));
        assert!(i.modified_ms > 0 && i.created_ms > 0 && i.accessed_ms > 0);
        assert_ne!(i.attributes & 0x20, 0); // FILE_ATTRIBUTE_ARCHIVE: file appena scritto
        let dir = file_info(d.path()).unwrap();
        assert!(dir.is_dir);
        assert_ne!(dir.attributes & 0x10, 0); // FILE_ATTRIBUTE_DIRECTORY
        assert_eq!(file_info(&d.path().join("gone")).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn long_path_chunk() {
        let d = tempfile::tempdir().unwrap();
        let mut dir = d.path().to_path_buf();
        for _ in 0..30 {
            dir.push("cartella_lunga"); // > 260 caratteri
        }
        std::fs::create_dir_all(crate::paths::to_extended(&dir)).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(crate::paths::to_extended(&f), b"0123456789").unwrap();
        assert!(f.as_os_str().len() > 260);
        assert_eq!(read_chunk(&f, 2, 3).unwrap(), b"234");
        assert_eq!(read_whole(&f, 10).unwrap(), b"0123456789");
        assert_eq!(file_info(&f).unwrap().size, 10);
    }

    #[test]
    fn acp_labels() {
        assert_eq!(encoding_for_acp(1252), "windows-1252");
        assert_eq!(encoding_for_acp(1250), "windows-1250");
        assert_eq!(encoding_for_acp(1258), "windows-1258");
        assert_eq!(encoding_for_acp(874), "windows-874");
        assert_eq!(encoding_for_acp(932), "shift_jis");
        assert_eq!(encoding_for_acp(936), "gbk");
        assert_eq!(encoding_for_acp(949), "euc-kr");
        assert_eq!(encoding_for_acp(950), "big5");
        assert_eq!(encoding_for_acp(65001), "utf-8");
        assert_eq!(encoding_for_acp(12345), "windows-1252");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn encodes_supported_charsets() {
        assert_eq!(encode_text("€", "windows-1252", false).unwrap(), [0x80]);
        assert_eq!(encode_text("a", "utf-16le", true).unwrap(), [0xff, 0xfe, 0x61, 0x00]);
        assert_eq!(encode_text("a", "utf-16be", false).unwrap(), [0x00, 0x61]);
        assert_eq!(encode_text("a", "utf-8", true).unwrap(), [0xef, 0xbb, 0xbf, 0x61]);
        assert_eq!(encode_text("ł", "windows-1252", false).unwrap_err().code, ErrorCode::UnmappableChars);
        assert_eq!(encode_text("a", "klingon", false).unwrap_err().code, ErrorCode::InvalidRequest);
        // UTF-16 fuori dal piano base: coppia surrogata, little endian.
        assert_eq!(encode_text("😀", "utf-16le", false).unwrap(), [0x3d, 0xd8, 0x00, 0xde]);
        // Il BOM c'è solo per le codifiche Unicode.
        assert_eq!(encode_text("a", "windows-1252", true).unwrap(), [0x61]);
    }

    #[test]
    fn stale_write_is_refused_and_leaves_the_file_alone() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, "v1").unwrap();
        let info = file_info(&f).unwrap();
        // Un altro programma riscrive il file e la data si sposta.
        std::fs::write(&f, "v2").unwrap();
        let t = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
        let e = write_text_file(&f, "mio", "utf-8", false, Some(info.modified_ms)).unwrap_err();
        assert_eq!(e.code, ErrorCode::ChangedOnDisk);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v2");
        assert_eq!(names(d.path()), ["a.txt"]);
    }

    #[test]
    fn vanished_file_counts_as_changed_unless_forced() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("gone.txt");
        assert_eq!(write_text_file(&f, "x", "utf-8", false, Some(1)).unwrap_err().code, ErrorCode::ChangedOnDisk);
        assert!(names(d.path()).is_empty());
        // Senza data attesa (Sovrascrivi) il file si ricrea con MoveFileExW.
        let info = write_text_file(&f, "x", "utf-8", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x");
        assert_eq!(info.size, 1);
        assert_eq!(names(d.path()), ["gone.txt"]);
    }

    #[test]
    fn unmappable_text_never_touches_the_disk() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ansi.txt");
        std::fs::write(&f, [b'p', 0xe8]).unwrap();
        let info = file_info(&f).unwrap();
        let e = write_text_file(&f, "pè ł", "windows-1252", false, Some(info.modified_ms)).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnmappableChars);
        assert_eq!(std::fs::read(&f).unwrap(), [b'p', 0xe8]);
        assert_eq!(file_info(&f).unwrap().modified_ms, info.modified_ms);
        assert_eq!(names(d.path()), ["ansi.txt"]);
    }

    #[test]
    fn write_replaces_content_and_keeps_attributes() {
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
        use windows::core::HSTRING;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("h.txt");
        std::fs::write(&f, "vecchio").unwrap();
        unsafe { SetFileAttributesW(&HSTRING::from(f.as_path()), FILE_ATTRIBUTE_HIDDEN) }.unwrap();
        // Il file deve risultare più vecchio di ora, altrimenti la data nuova potrebbe coincidere.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(old).unwrap();
        let expected = file_info(&f).unwrap().modified_ms;
        let after = write_text_file(&f, "nuovo", "utf-8", true, Some(expected)).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), [&[0xef, 0xbb, 0xbf][..], b"nuovo"].concat());
        assert_ne!(after.attributes & 0x2, 0, "HIDDEN perso");
        assert!(after.modified_ms > expected);
        assert_eq!((after.size, after.modified_ms), (8, file_info(&f).unwrap().modified_ms));
        assert_eq!(names(d.path()), ["h.txt"]);
    }

    #[test]
    fn failed_replace_removes_the_part_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ro.txt");
        std::fs::write(&f, "intatto").unwrap();
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        let e = write_text_file(&f, "altro", "utf-8", false, None).unwrap_err();
        assert_eq!(e.code, ErrorCode::AccessDenied);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "intatto");
        assert_eq!(names(d.path()), ["ro.txt"]);
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(false);
        std::fs::set_permissions(&f, perm).unwrap();
    }

    #[test]
    fn save_while_another_program_holds_the_file_without_delete_share() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("held.txt");
        std::fs::write(&f, "v1").unwrap();
        let _held = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&f).unwrap(); // READ|WRITE, niente DELETE
        assert_eq!(write_text_file(&f, "v2", "utf-8", false, None).unwrap_err().code, ErrorCode::FileInUse);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v1");
        assert_eq!(names(d.path()), ["held.txt"]);
    }

    #[test]
    fn a_part_file_that_already_exists_is_never_touched() {
        // Il `.mcpart` di una copia in corso (o un file dell'utente con quel nome) non è nostro:
        // la scrittura usa un nome proprio e non lo sovrascrive né lo elimina, nemmeno se fallisce.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        let other = d.path().join("a.txt.mcpart");
        std::fs::write(&f, "v1").unwrap();
        std::fs::write(&other, "altro").unwrap();
        write_text_file(&f, "v2", "utf-8", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "v2");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "altro");
        assert_eq!(names(d.path()), ["a.txt", "a.txt.mcpart"]);

        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        assert_eq!(write_text_file(&f, "v3", "utf-8", false, None).unwrap_err().code, ErrorCode::AccessDenied);
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "altro");
        assert_eq!(names(d.path()), ["a.txt", "a.txt.mcpart"]);
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(false);
        std::fs::set_permissions(&f, perm).unwrap();
    }

    #[test]
    fn parallel_writes_never_leave_a_half_written_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("p.txt");
        std::fs::write(&f, "v0").unwrap();
        let texts: Vec<String> = (0..8).map(|i| char::from(b'a' + i).to_string().repeat(512 * 1024)).collect();
        let handles: Vec<_> = texts
            .iter()
            .cloned()
            .map(|t| {
                let f = f.clone();
                std::thread::spawn(move || write_text_file(&f, &t, "utf-8", false, None))
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(results.iter().any(|r| r.is_ok()));
        // Il file è intero: uno dei testi, mai un misto o un troncamento.
        assert!(texts.contains(&std::fs::read_to_string(&f).unwrap()));
    }

    fn remote(m: &std::sync::Arc<crate::vfs::mem::MemProvider>) -> (crate::vfs::router::Vfs, crate::vfs::loc::Loc) {
        (crate::vfs::mem::vfs(&[("m", m)]), crate::vfs::loc::Loc::parse("sftp://m/d/a.txt").unwrap())
    }

    #[test]
    fn remote_save_detects_changes() {
        let m = std::sync::Arc::new(crate::vfs::mem::MemProvider::new());
        m.add_file("/d/a.txt", b"v1");
        m.set_meta("/d/a.txt", Some(2_000), Some(0o640)).unwrap();
        let (v, l) = remote(&m);
        // Data diversa da quella dell'apertura, poi stessa data ma dimensione diversa: niente scrittura.
        for (t, size) in [(1_000, Some(2)), (2_000, Some(5))] {
            let e = remote_write_text_file(&v, &l, "mio", "utf-8", false, Some(t), size, false).unwrap_err();
            assert_eq!(e.code, ErrorCode::ChangedOnDisk);
            assert_eq!(m.data("/d/a.txt").unwrap(), b"v1");
        }
        let info = remote_write_text_file(&v, &l, "mio", "utf-8", false, Some(2_000), Some(2), false).unwrap();
        assert_eq!((m.data("/d/a.txt").unwrap(), info.size), (b"mio".to_vec(), 3));
        // Scrittura sicura (temporaneo e rinomina): nessun `.mcpart` resta, i permessi POSIX si riapplicano.
        assert_eq!(m.list("/d").unwrap().into_iter().map(|s| s.name).collect::<Vec<_>>(), ["a.txt"]);
        assert_eq!(m.stat("/d/a.txt").unwrap().mode, Some(0o640));
        // Un file sparito dal server conta come cambiato.
        m.delete("/d/a.txt", false).unwrap();
        let e = remote_write_text_file(&v, &l, "mio", "utf-8", false, Some(2_000), Some(3), false).unwrap_err();
        assert_eq!(e.code, ErrorCode::ChangedOnDisk);
        assert!(m.data("/d/a.txt").is_none());
    }

    #[test]
    fn remote_save_unsafe_needs_flag() {
        let mut m = crate::vfs::mem::MemProvider::new();
        m.atomic_replace = false;
        let m = std::sync::Arc::new(m);
        m.add_file("/d/a.txt", b"vecchio");
        let (v, l) = remote(&m);
        let e = remote_write_text_file(&v, &l, "nuovo", "utf-8", false, None, None, false).unwrap_err();
        assert_eq!((e.code, e.path.as_deref()), (ErrorCode::UnsafeWrite, Some("sftp://m/d/a.txt")));
        assert_eq!(m.data("/d/a.txt").unwrap(), b"vecchio");
        remote_write_text_file(&v, &l, "nuovo", "utf-8", false, None, None, true).unwrap();
        assert_eq!(m.data("/d/a.txt").unwrap(), b"nuovo");
        assert_eq!(m.list("/d").unwrap().len(), 1);
    }

    /// `t.zip` con `conf/a.xml` e `big.bin`; la voce `a.xml` come `Loc`.
    fn zip_with_xml(d: &Path) -> (PathBuf, Loc) {
        let z = d.join("t.zip");
        let mut w = zip::ZipWriter::new(File::create(&z).unwrap());
        w.start_file("conf/a.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"<a>vecchio</a>").unwrap();
        w.start_file("big.bin", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&b"0123456789abcdef".repeat(4096)).unwrap();
        w.finish().unwrap();
        let loc = Loc::archive(Loc::Local(z.clone()), "/conf/a.xml");
        (z, loc)
    }

    /// Byte compressi e CRC di una voce.
    fn raw(z: &Path, name: &str) -> (Vec<u8>, u32) {
        let mut a = zip::ZipArchive::new(File::open(z).unwrap()).unwrap();
        let i = a.index_for_name(name).unwrap();
        let mut f = a.by_index_raw(i).unwrap();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        (data, f.crc32())
    }

    /// Coda con l'executor locale e il `Vfs` senza connessioni (per i salvataggi dentro uno ZIP).
    fn queue() -> (Queue, std::sync::Arc<Vfs>) {
        use std::sync::Arc;
        let v = Arc::new(crate::vfs::mem::vfs(&[]));
        let elevate: crate::ops::queue::Elevate = Arc::new(|| panic!("helper elevato non previsto"));
        (Queue::new(Arc::new(crate::ops::exec::LocalExecutor), elevate, v.clone(), Arc::new(|_| {})), v)
    }

    #[test]
    fn archive_entry_date_is_as_reliable_as_its_container() {
        let d = tempfile::tempdir().unwrap();
        let (_z, loc) = zip_with_xml(d.path());
        let (q, v) = queue();
        // Le date DOS sono a 2 s, ma il salvataggio controlla anche dimensione e data del contenitore: su un disco
        // locale l'editor non deve chiedere conferma come per un server FTP senza date precise.
        assert!(archive_file_info(&v, &loc).unwrap().mtime_reliable);
        let info = archive_write_text_file(&q, &v, &loc, "x", "utf-8", false, None, None).unwrap();
        assert!(info.mtime_reliable);
        // Sul remoto vale la precisione del server del contenitore.
        let mut m = crate::vfs::mem::MemProvider::new();
        m.precise_mtime = false;
        let m = std::sync::Arc::new(m);
        m.add_file("/t.zip", &std::fs::read(d.path().join("t.zip")).unwrap());
        let rv = crate::vfs::mem::vfs(&[("m", &m)]);
        let remote = Loc::archive(Loc::parse("ftp://m/t.zip").unwrap(), "/conf/a.xml");
        assert!(!archive_file_info(&rv, &remote).unwrap().mtime_reliable);
    }

    #[test]
    fn save_entry_replaces_one_entry() {
        let d = tempfile::tempdir().unwrap();
        let (z, loc) = zip_with_xml(d.path());
        let (q, v) = queue();
        let before = raw(&z, "big.bin");
        let old = {
            let (p, path) = v.remote(&loc, Purpose::Browse).unwrap();
            p.stat(&path).unwrap() // il provider si rilascia: appena letto, tiene aperto lo ZIP per un po'
        };
        let info = archive_write_text_file(&q, &v, &loc, "<a>nuovo</a>", "utf-8", false, old.modified_ms, Some(old.size)).unwrap();
        assert_eq!(info.size, 12);
        let text = {
            let (p, path) = v.remote(&loc, Purpose::Browse).unwrap();
            remote_whole(&*p, &path, 1000).unwrap()
        };
        assert_eq!(text, b"<a>nuovo</a>");
        assert_eq!(raw(&z, "big.bin"), before);
        assert_eq!(names(d.path()), ["t.zip"]);
        // Senza data attesa si sovrascrive; con una dimensione sbagliata no.
        archive_write_text_file(&q, &v, &loc, "x", "utf-8", false, None, None).unwrap();
        let e = archive_write_text_file(&q, &v, &loc, "y", "utf-8", false, Some(0), Some(99)).unwrap_err();
        assert_eq!(e.code, ErrorCode::ChangedOnDisk);
    }

    #[test]
    fn save_entry_detects_change() {
        let d = tempfile::tempdir().unwrap();
        let (z, loc) = zip_with_xml(d.path());
        let (q, v) = queue();
        let orig = std::fs::read(&z).unwrap();
        let old = {
            let (p, path) = v.remote(&loc, Purpose::Browse).unwrap();
            p.stat(&path).unwrap() // il provider si rilascia: appena letto, tiene aperto lo ZIP per un po'
        };
        let e = archive_write_text_file(&q, &v, &loc, "x", "utf-8", false, old.modified_ms, Some(old.size + 1)).unwrap_err();
        assert_eq!(e.code, ErrorCode::ChangedOnDisk);
        // Con data e dimensione giuste il job salva.
        let info = archive_write_text_file(&q, &v, &loc, "x", "utf-8", false, old.modified_ms, Some(old.size)).unwrap();
        assert_eq!(info.size, 1);
        std::fs::write(&z, &orig).unwrap();
        // Il job che salva è nella coda, come gli altri.
        assert_eq!(q.list().iter().map(|j| (j.state, j.dest.clone())).collect::<Vec<_>>(), [(JobState::Done, Some(loc.to_string()))]);
        let e = archive_write_text_file(&q, &v, &loc, "x", "utf-8", false, Some(0), None).unwrap_err();
        assert_eq!(e.code, ErrorCode::ChangedOnDisk);
        assert_eq!(std::fs::read(&z).unwrap(), orig);
        assert_eq!(names(d.path()), ["t.zip"]);
    }

    #[test]
    fn save_entry_on_7z_is_unsupported() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("t.7z");
        std::fs::write(&z, b"7z").unwrap();
        let (q, v) = queue();
        let e = archive_write_text_file(&q, &v, &Loc::archive(Loc::Local(z), "/a.txt"), "x", "utf-8", false, None, None).unwrap_err();
        assert_eq!(e.code, ErrorCode::Unsupported);
        assert!(q.list().is_empty());
        assert_eq!(e.code, ErrorCode::Unsupported);
    }

    #[test]
    fn replace_errors_that_leave_the_data_only_in_the_part_keep_it() {
        // ERROR_UNABLE_TO_MOVE_REPLACEMENT (1176) e _2 (1177): il file sostituito può non esistere più
        // e la copia nuova è l'unica rimasta, quindi il `.mcpart` non si elimina.
        let hr = |code: u32| windows::core::HRESULT::from_win32(code).0;
        assert!(part_holds_the_data(hr(1176)));
        assert!(part_holds_the_data(hr(1177)));
        assert!(!part_holds_the_data(hr(1175))); // ERROR_UNABLE_TO_REMOVE_REPLACED: i due file sono intatti
        assert!(!part_holds_the_data(hr(5)));
    }
}
