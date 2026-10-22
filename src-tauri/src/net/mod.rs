//! Connessioni salvate, password nel Credential Manager, impronte accettate e sessioni verso i server.

pub mod connections;
pub mod credentials;
pub mod ftp;
pub mod sessions;
pub mod sftp;
pub mod tls;
pub mod trust;

use crate::error::AppError;
use crate::fs::blocking;
use connections::{Connection, ConnectionView};
use sessions::Sessions;
use std::sync::Arc;
use tauri::State;

type Net<'a> = State<'a, Arc<Sessions>>;

#[tauri::command]
pub async fn list_connections(net: Net<'_>) -> Result<Vec<ConnectionView>, AppError> {
    let s = net.inner().clone();
    blocking(move || s.list()).await?
}

#[tauri::command]
pub async fn save_connection(
    net: Net<'_>,
    conn: Connection,
    password: Option<String>,
    remember: bool,
) -> Result<ConnectionView, AppError> {
    let s = net.inner().clone();
    blocking(move || s.save(conn, password, remember)).await?
}

#[tauri::command]
pub async fn delete_connection(net: Net<'_>, id: String) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.delete(&id)).await?
}

#[tauri::command]
pub async fn forget_password(net: Net<'_>, id: String) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.forget_password(&id)).await?
}

#[tauri::command]
pub async fn connect(net: Net<'_>, id: String, password: Option<String>, remember: bool) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.connect(&id, password, remember)).await?
}

#[tauri::command]
pub async fn disconnect(net: Net<'_>, id: String) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.disconnect(&id)).await
}

#[tauri::command]
pub async fn trust_host(net: Net<'_>, id: String, fingerprint: String) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.trust_host(&id, &fingerprint)).await?
}

#[tauri::command]
pub async fn forget_host(net: Net<'_>, id: String) -> Result<(), AppError> {
    let s = net.inner().clone();
    blocking(move || s.forget_host(&id)).await?
}
