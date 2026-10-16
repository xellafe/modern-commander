//! Elenco del contenuto di una cartella con `FindFirstFileExW`: una sola chiamata di sistema per
//! blocco di voci, con nome, attributi, dimensione e data già inclusi (niente `metadata` per file).

use crate::error::{AppError, ErrorCode};
use crate::paths::to_extended;
use serde::Serialize;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    WIN32_FIND_DATAW,
};
use windows::core::HSTRING;

const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Millisecondi tra l'epoca FILETIME (1601) e l'epoca Unix (1970).
const FILETIME_UNIX_DIFF_MS: i64 = 11_644_473_600_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Ultima modifica, millisecondi dall'epoca Unix.
    pub modified_ms: i64,
    /// Attributi Win32 grezzi (`FILE_ATTRIBUTE_*`).
    pub attributes: u32,
    /// Collegamento simbolico o giunzione.
    pub is_symlink: bool,
}

/// Chiude l'handle di ricerca al drop.
struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        let _ = unsafe { FindClose(self.0) };
    }
}

fn entry_from(d: &WIN32_FIND_DATAW) -> Option<Entry> {
    let len = d.cFileName.iter().position(|&c| c == 0).unwrap_or(d.cFileName.len());
    let name = &d.cFileName[..len];
    if name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16] {
        return None;
    }
    let ft = (u64::from(d.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(d.ftLastWriteTime.dwLowDateTime);
    let attributes = d.dwFileAttributes;
    Some(Entry {
        name: String::from_utf16_lossy(name),
        is_dir: attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        size: (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow),
        modified_ms: (ft / 10_000) as i64 - FILETIME_UNIX_DIFF_MS,
        attributes,
        // Dopo FindFirstFile, `dwReserved0` contiene il reparse tag quando l'attributo è presente.
        is_symlink: attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            && matches!(d.dwReserved0, IO_REPARSE_TAG_SYMLINK | IO_REPARSE_TAG_MOUNT_POINT),
    })
}

/// Contenuto di `path` senza `.` e `..`. Un file al posto di una cartella dà `INVALID_REQUEST`.
pub fn list_dir(path: &Path) -> Result<Vec<Entry>, AppError> {
    let err = |e: &std::io::Error| AppError::from_io(e, path);
    let pattern = HSTRING::from(to_extended(path).join("*").as_os_str());
    let mut data = WIN32_FIND_DATAW::default();
    let first = unsafe {
        FindFirstFileExW(
            &pattern,
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match first {
        Ok(h) => FindHandle(h),
        Err(e) => {
            let meta = std::fs::metadata(to_extended(path));
            return match meta {
                // Esiste ma non è una cartella.
                Ok(m) if !m.is_dir() => Err(AppError::new(ErrorCode::InvalidRequest, Some(path))),
                // Radice di unità senza voci: nessun errore.
                Ok(_) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(Vec::new()),
                Ok(_) => Err(err(&std::io::Error::from_raw_os_error((e.code().0 & 0xFFFF) as i32))),
                Err(io) => Err(err(&io)),
            };
        }
    };
    let mut out = Vec::with_capacity(256);
    out.extend(entry_from(&data));
    loop {
        match unsafe { FindNextFileW(handle.0, &mut data) } {
            Ok(()) => out.extend(entry_from(&data)),
            Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(e) => {
                return Err(err(&std::io::Error::from_raw_os_error((e.code().0 & 0xFFFF) as i32)));
            }
        }
    }
    Ok(out)
}

/// Antenato più vicino che esiste ed è una cartella (può essere `path` stesso).
pub fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|a| to_extended(a).is_dir())
        .map(Path::to_path_buf)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::path::Path;

    #[test]
    fn lists_files_and_dirs_with_metadata() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("a.txt"), b"hello").unwrap();
        let mut v = list_dir(d.path()).unwrap();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].name.as_str(), v[0].is_dir, v[0].size), ("a.txt", false, 5));
        assert!(v[1].is_dir && v[1].name == "sub");
        assert!(v[0].modified_ms > 0);
    }

    #[test]
    fn missing_dir_is_not_found() {
        let e = list_dir(Path::new(r"C:\non\esiste\proprio")).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert_eq!(e.path.as_deref(), Some(r"C:\non\esiste\proprio"));
    }

    #[test]
    fn file_is_invalid_request() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(list_dir(&f).unwrap_err().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn empty_dir_and_drive_root_work() {
        let d = tempfile::tempdir().unwrap();
        assert!(list_dir(d.path()).unwrap().is_empty());
        assert!(list_dir(Path::new(r"C:\")).is_ok());
    }

    #[test]
    fn nearest_ancestor_walks_up() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("x").join("y");
        assert_eq!(nearest_existing_ancestor(&gone).unwrap(), d.path());
        assert_eq!(nearest_existing_ancestor(d.path()).unwrap(), d.path());
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(nearest_existing_ancestor(&f).unwrap(), d.path());
    }

    #[test]
    fn junction_is_symlink() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = d.path().join("link");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        let v = list_dir(d.path()).unwrap();
        assert!(v.iter().find(|e| e.name == "link").unwrap().is_symlink);
        assert!(!v.iter().find(|e| e.name == "target").unwrap().is_symlink);
    }
}