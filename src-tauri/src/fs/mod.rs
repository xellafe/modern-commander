//! Comandi Tauri per elenco cartelle e unità. Il lavoro bloccante gira su un thread dedicato.

pub mod content;
pub mod drives;
pub mod listing;
pub mod size;
pub mod watch;

use crate::error::{AppError, ErrorCode};
use std::path::{Path, PathBuf};
use tauri::ipc::Response;

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, None)
    })
}

#[tauri::command]
pub async fn list_dir(path: String) -> Result<Vec<listing::Entry>, AppError> {
    blocking(move || listing::list_dir(Path::new(&path))).await?
}

#[tauri::command]
pub async fn nearest_existing_ancestor(path: String) -> Result<Option<PathBuf>, AppError> {
    blocking(move || listing::nearest_existing_ancestor(Path::new(&path))).await
}

/// Percorsi in forma lunga (i nomi 8.3 del drop da Explorer); quelli non risolvibili restano invariati.
#[tauri::command]
pub async fn normalize_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, AppError> {
    blocking(move || paths.iter().map(|p| listing::long_path(Path::new(p))).collect()).await
}

#[tauri::command]
pub async fn list_drives()-> Result<Vec<drives::Drive>, AppError> {
    blocking(drives::list_drives).await
}

#[tauri::command]
pub async fn file_info(path: String) -> Result<content::FileInfo, AppError> {
    blocking(move || content::file_info(Path::new(&path))).await?
}

/// I byte arrivano al frontend come `ArrayBuffer`, senza passare da JSON.
#[tauri::command]
pub async fn read_chunk(path: String, offset: u64, len: u32) -> Result<Response, AppError> {
    let bytes = blocking(move || content::read_chunk(Path::new(&path), offset, len)).await??;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn read_file(path: String, max: u64) -> Result<Response, AppError> {
    let bytes = blocking(move || content::read_whole(Path::new(&path), max)).await??;
    Ok(Response::new(bytes))
}

/// Etichetta della codifica ANSI di sistema (code page `GetACP`) nel formato di `TextDecoder`.
#[tauri::command]
pub fn ansi_encoding() -> String {
    content::encoding_for_acp(content::ansi_code_page()).to_string()
}
