//! Sorgente di blocchi di byte per le ricerche del viewer (inizi riga, colonne): un file locale o uno remoto.

use super::content::open_shared;
use crate::error::AppError;
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Provider, Purpose};
use crate::vfs::router::Vfs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub trait BlockSource {
    /// Dimensione all'apertura.
    fn len(&self) -> u64;
    /// Fino a `len` byte da `offset`: più corto solo a fine file.
    fn read_at(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, AppError>;
}

/// File locale aperto in condivisione piena (vedi `open_shared`).
pub struct LocalFile {
    file: File,
    len: u64,
    path: PathBuf,
}

impl LocalFile {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let err = |e: std::io::Error| AppError::from_io(&e, path);
        let file = open_shared(path).map_err(err)?;
        let len = file.metadata().map_err(err)?.len();
        Ok(Self { file, len, path: path.to_path_buf() })
    }
}

impl BlockSource for LocalFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let err = |e: std::io::Error| AppError::from_io(&e, &self.path);
        self.file.seek(SeekFrom::Start(offset)).map_err(err)?;
        let mut buf = Vec::new();
        (&mut self.file).take(u64::from(len)).read_to_end(&mut buf).map_err(err)?;
        Ok(buf)
    }
}

pub struct RemoteFile {
    provider: Arc<dyn Provider>,
    path: String,
    len: u64,
}

impl BlockSource for RemoteFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        self.provider.read_at(&self.path, offset, len)
    }
}

pub fn open_source(vfs: &Vfs, loc: &Loc) -> Result<Box<dyn BlockSource>, AppError> {
    match loc {
        Loc::Local(p) => Ok(Box::new(LocalFile::open(p)?)),
        Loc::Remote { .. } | Loc::Archive { .. } => {
            let (provider, path) = vfs.remote(loc, Purpose::Browse)?;
            let len = provider.stat(&path)?.size;
            Ok(Box::new(RemoteFile { provider, path, len }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{columns, lines};
    use crate::vfs::mem::{MemConnector, MemProvider};
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn remote_lines_match_local_lines() {
        // Più di un blocco da 1 MiB: righe da 10.000 caratteri (con accenti e tab) alternate a righe corte.
        let mut text = String::new();
        for i in 0..60 {
            text.push_str(&"è\tabcdefgh".repeat(1000));
            text.push_str(if i % 2 == 0 { "\r\n" } else { "\n" });
            text.push_str(&format!("riga {i}\n"));
        }
        text.push_str("fine senza a capo");
        let bytes = text.into_bytes();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &bytes).unwrap();
        let m = Arc::new(MemProvider::new());
        m.add_file("/logs/a.log", &bytes);
        let vfs = Vfs::new(Arc::new(MemConnector(HashMap::from([("m".to_owned(), m)]))));
        let local = Loc::Local(tmp.path().to_path_buf());
        let remote = Loc::parse("sftp://m/logs/a.log").unwrap();

        let no = AtomicBool::new(false);
        let len = bytes.len() as u64;
        let both = |f: &dyn Fn(&mut dyn BlockSource) -> Vec<String>| {
            let l = f(&mut *open_source(&vfs, &local).unwrap());
            let r = f(&mut *open_source(&vfs, &remote).unwrap());
            assert_eq!(l, r);
        };
        assert_eq!(open_source(&vfs, &remote).unwrap().len(), len);
        let points = [0, 1, 5_000, 20_011, 1 << 20, (1 << 20) + 7, len / 2, len - 3, len, len + 10];
        both(&|s| {
            let mut out = vec![];
            for &p in &points {
                let mut seen = vec![];
                out.push(format!("{:?}", lines::next_line_start(s, p, 1, false, &no, &mut |x| seen.push(x))));
                out.push(format!("{:?}", lines::line_start(s, p.min(len), 1, false, &no, &mut |x| seen.push(x))));
                out.push(format!("{seen:?}"));
            }
            out
        });
        both(&|s| {
            let mut out = vec![];
            for &p in &points[..8] {
                let start = lines::line_start(s, p, 1, false, &no, &mut |_| {}).unwrap();
                let from = columns::Mark { offset: start, column: 0 };
                for target in [0, 3, 7_000, u64::MAX] {
                    out.push(format!("{:?}", columns::column_offset(s, "utf-8", from, target, &no, &mut |_| {})));
                }
            }
            out
        });
    }
}
