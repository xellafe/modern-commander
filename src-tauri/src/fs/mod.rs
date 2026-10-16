//! Comandi Tauri per elenco cartelle e unità. Il lavoro bloccante gira su un thread dedicato.

pub mod drives;
pub mod listing;
pub mod watch;

use crate::error::{AppError, ErrorCode};
use std::path::{Path, PathBuf};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, AppError> {
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

#[tauri::command]
pub async fn list_drives() -> Result<Vec<drives::Drive>, AppError> {
    blocking(drives::list_drives).await
}
