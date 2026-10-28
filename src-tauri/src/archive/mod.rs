//! Archivi: elenco per l'ispettore (`list`), contenitore e indice in memoria per leggerli come cartelle (`container`, `index`).

pub mod container;
pub mod index;
pub mod list;
pub mod sevenz_guard;
mod tar_walk;

use crate::error::AppError;
use crate::fs::blocking;
use crate::requests::Requests;
use serde::Serialize;
use std::path::Path;
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

/// Elenca le prime `limit` voci di `path`. Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn list_archive(
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    limit: usize,
) -> Result<ArchiveListing, AppError> {
    let path = crate::fs::local_only(&path)?;
    let guard = requests.register(request_id);
    blocking(move || list::list_archive(&path, limit, guard.flag())).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_extension() {
        assert!(matches!(format_of(Path::new("a.TAR.GZ")), Some(ArchiveFormat::TarGz)));
        assert!(matches!(format_of(Path::new("a.tgz")), Some(ArchiveFormat::TarGz)));
        assert!(format_of(Path::new("a.gz")).is_none());
    }
}
