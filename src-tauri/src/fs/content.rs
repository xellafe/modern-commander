//! Lettura dei file per visualizzatore, editor e ispettore: informazioni, blocchi di byte e lettura
//! intera con limite. I file si aprono in condivisione piena, così si leggono anche quelli che un altro
//! programma sta scrivendo (log, per esempio).

use super::listing::FILETIME_UNIX_DIFF_MS;
use crate::error::{AppError, ErrorCode};
use crate::paths::to_extended;
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use windows::Win32::Globalization::GetACP;

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
}

/// Apre in sola lettura senza escludere nessuno, anche con percorsi oltre i 260 caratteri.
pub(crate) fn open_shared(path: &Path) -> std::io::Result<File> {
    File::options().read(true).share_mode(SHARE_ALL).open(to_extended(path))
}

fn ms(filetime: u64) -> i64 {
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
    })
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
}
