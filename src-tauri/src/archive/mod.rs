//! Archivi: elenco per l'ispettore (`list`), contenitore e indice in memoria per leggerli come cartelle (`container`,
//! `index`), provider di cartella in cache (`provider`, `cache`) con lettori a canale e cursori (`pipe`, `cursor`).

pub mod cache;
pub mod container;
mod cursor;
pub mod index;
pub mod list;
pub mod pipe;
pub mod provider;
pub mod sevenz_guard;
mod tar_walk;

use crate::error::{AppError, ErrorCode};
use crate::fs::blocking;
use crate::requests::Requests;
use crate::vfs::loc::Loc;
use crate::vfs::router::Vfs;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tauri::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    Tar,
    TarGz,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    /// Percorso interno con `\`, senza barra finale.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveListing {
    pub entries: Vec<ArchiveEntry>,
    /// Voci totali: noto per ZIP e 7z (indice centrale), per TAR solo se l'elenco non si è fermato al limite.
    pub total: Option<u64>,
    pub truncated: bool,
}

/// Formato dall'estensione (senza distinguere maiuscole); `.gz` da solo non è un archivio.
pub fn format_of(path: &Path) -> Option<ArchiveFormat> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let ext = |e: &str| name.ends_with(e);
    if ext(".tar.gz") || ext(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else if ext(".zip") {
        Some(ArchiveFormat::Zip)
    } else if ext(".7z") {
        Some(ArchiveFormat::SevenZ)
    } else if ext(".tar") {
        Some(ArchiveFormat::Tar)
    } else {
        None
    }
}

/// Elenca le prime `limit` voci di `path`, locale o remoto. Si annulla con `cancel_request(request_id)` (errore
/// `CANCELLED`).
#[tauri::command]
pub async fn list_archive(
    vfs: State<'_, Arc<Vfs>>,
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    limit: usize,
) -> Result<ArchiveListing, AppError> {
    let loc = Loc::parse(&path)?;
    let guard = requests.register(request_id);
    let vfs = vfs.inner().clone();
    blocking(move || match loc {
        Loc::Local(p) => list::list_archive(&p, limit, guard.flag()),
        loc => list_remote(&vfs, &loc, limit, guard.flag()),
    })
    .await?
}

/// Le prime `limit` voci di un archivio remoto, dall'indice in cache: solo ZIP e 7z, che si leggono a salti. TAR e
/// TAR.GZ remoti si dovrebbero leggere (o scaricare) per intero, e l'ispettore non lo fa.
fn list_remote(vfs: &Vfs, container: &Loc, limit: usize, cancel: &AtomicBool) -> Result<ArchiveListing, AppError> {
    let format = container.file_name().and_then(|n| format_of(Path::new(&n)));
    match (container, format) {
        (Loc::Remote { .. }, Some(ArchiveFormat::Zip | ArchiveFormat::SevenZ)) => {
            let p = vfs.archive(container, cancel)?;
            list::list_index(p.index(), Path::new(&container.to_string()), limit)
        }
        _ => Err(container.error(ErrorCode::Unsupported)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_inspector_lists_zip_but_not_tar() {
        use crate::vfs::mem::{MemProvider, vfs};
        let m = std::sync::Arc::new(MemProvider::new());
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("d/x.txt", zip::write::SimpleFileOptions::default()).unwrap();
        m.add_file("/a.zip", &w.finish().unwrap().into_inner());
        m.add_file("/a.tar", &[0; 1024]);
        let v = vfs(&[("m", &m)]);
        let no = AtomicBool::new(false);
        let l = list_remote(&v, &Loc::parse("sftp://m/a.zip").unwrap(), 10, &no).unwrap();
        assert_eq!((l.entries[0].path.as_str(), l.total), (r"d\x.txt", Some(1)));
        let e = list_remote(&v, &Loc::parse("sftp://m/a.tar").unwrap(), 10, &no).unwrap_err();
        assert_eq!(e.code, ErrorCode::Unsupported);
        let e = list_remote(&v, &Loc::parse(r"arc://C:\a.zip/b.zip").unwrap(), 10, &no).unwrap_err();
        assert_eq!(e.code, ErrorCode::Unsupported);
    }

    #[test]
    fn formats_by_extension() {
        assert!(matches!(format_of(Path::new("a.TAR.GZ")), Some(ArchiveFormat::TarGz)));
        assert!(matches!(format_of(Path::new("a.tgz")), Some(ArchiveFormat::TarGz)));
        assert!(format_of(Path::new("a.gz")).is_none());
    }
}
