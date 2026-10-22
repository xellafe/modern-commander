//! Provider sul disco locale, usato solo dai trasferimenti tra provider diversi (`Vfs::endpoint`): liste, viewer
//! e coda locale restano sul codice di `fs` e `ops`. I percorsi sono quelli Windows, sempre con `to_extended`.

use super::provider::{Caps, Provider, Stat};
use crate::error::AppError;
use crate::fs::content::{ms, open_shared, read_chunk};
use crate::ops::exec::{clear_readonly_of, move_file};
use crate::paths::to_extended;
use std::fs::{File, Metadata};
use std::os::windows::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING};

pub struct LocalProvider;

/// `FILE_WRITE_ATTRIBUTES`: basta per cambiare le date.
const WRITE_ATTRIBUTES: u32 = 0x100;
/// `FILE_FLAG_BACKUP_SEMANTICS`: apre anche le cartelle.
const BACKUP_SEMANTICS: u32 = 0x0200_0000;

fn ext(path: &str) -> PathBuf {
    to_extended(Path::new(path))
}

fn io_err(path: &str) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |e| AppError::from_io(&e, Path::new(path))
}

fn stat_of(name: String, m: &Metadata) -> Stat {
    let dir_link = m.file_type().is_symlink_dir();
    Stat {
        name,
        is_dir: m.is_dir() || dir_link,
        dir_link,
        size: m.len(),
        modified_ms: Some(ms(m.last_write_time())),
        mode: None,
    }
}

impl Provider for LocalProvider {
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: true, precise_mtime: true, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let m = std::fs::symlink_metadata(ext(path)).map_err(io_err(path))?;
        let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(stat_of(name, &m))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        std::fs::read_dir(ext(path))
            .map_err(io_err(path))?
            .map(|e| {
                let e = e.map_err(io_err(path))?;
                // Su Windows `DirEntry::metadata` non segue i collegamenti.
                Ok(stat_of(e.file_name().to_string_lossy().into_owned(), &e.metadata().map_err(io_err(path))?))
            })
            .collect()
    }

    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        read_chunk(Path::new(path), offset, len)
    }

    fn reader(&self, path: &str) -> Result<Box<dyn std::io::Read + Send>, AppError> {
        Ok(Box::new(open_shared(Path::new(path)).map_err(io_err(path))?))
    }

    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn std::io::Write + Send>, AppError> {
        let f = File::options().write(true).create(true).truncate(true).create_new(create_new).open(ext(path));
        Ok(Box::new(f.map_err(io_err(path))?))
    }

    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        if replace {
            // Come nelle copie locali: MoveFileExW non sostituisce un file di sola lettura.
            clear_readonly_of(Path::new(to))?;
        }
        let flags = if replace { MOVEFILE_REPLACE_EXISTING } else { MOVE_FILE_FLAGS(0) };
        move_file(Path::new(from), Path::new(to), flags)
    }

    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir { std::fs::remove_dir(ext(path)) } else { std::fs::remove_file(ext(path)) }.map_err(io_err(path))
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        std::fs::create_dir(ext(path)).map_err(io_err(path))
    }

    /// Solo la data: in locale i permessi POSIX non hanno senso.
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, _mode: Option<u32>) -> Result<(), AppError> {
        let Some(t) = modified_ms else { return Ok(()) };
        let d = Duration::from_millis(t.unsigned_abs());
        let time = if t >= 0 { UNIX_EPOCH + d } else { UNIX_EPOCH - d };
        let f = File::options().access_mode(WRITE_ATTRIBUTES).custom_flags(BACKUP_SEMANTICS).open(ext(path));
        f.and_then(|f| f.set_modified(time)).map_err(io_err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::io::{Read, Write};

    #[test]
    fn local_provider_basics() {
        let d = tempfile::tempdir().unwrap();
        let p = |n: &str| d.path().join(n).to_string_lossy().into_owned();
        let lp = LocalProvider;
        lp.writer(&p("a"), true).unwrap().write_all(b"uno").unwrap();
        assert_eq!(lp.writer(&p("a"), true).err().unwrap().code, ErrorCode::AlreadyExists);
        lp.writer(&p("b"), false).unwrap().write_all(b"due").unwrap();
        assert_eq!(lp.rename(&p("b"), &p("a"), false).unwrap_err().code, ErrorCode::AlreadyExists);
        lp.rename(&p("b"), &p("a"), true).unwrap();
        let mut s = String::new();
        lp.reader(&p("a")).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "due");
        assert_eq!(lp.read_at(&p("a"), 1, 10).unwrap(), b"ue");
        lp.set_meta(&p("a"), Some(1_000_000_000_000), Some(0o644)).unwrap();
        assert_eq!(lp.stat(&p("a")).unwrap().modified_ms, Some(1_000_000_000_000));
        lp.mkdir(&p("c")).unwrap();
        let mut names: Vec<_> = lp.list(&d.path().to_string_lossy()).unwrap().into_iter().map(|s| (s.name, s.is_dir)).collect();
        names.sort();
        assert_eq!(names, [("a".into(), false), ("c".into(), true)]);
        lp.delete(&p("c"), true).unwrap();
        lp.delete(&p("a"), false).unwrap();
        assert_eq!(lp.stat(&p("a")).unwrap_err().code, ErrorCode::NotFound);
    }
}
