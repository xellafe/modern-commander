//! Connessioni salvate nel file interno `connections`, senza password (stanno nel Credential Manager).

use super::credentials::Secrets;
use super::trust;
use crate::config;
use crate::error::{AppError, ErrorCode};
use crate::vfs::loc::Scheme;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub start_dir: String,
    pub ftps_implicit: bool,
}

/// Quello che vede il frontend: la password mai, solo se c'è.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    #[serde(flatten)]
    pub conn: Connection,
    pub has_password: bool,
}

const FILE: &str = "connections";

/// Serializza lettura-modifica-scrittura del file.
static LOCK: Mutex<()> = Mutex::new(());

fn io(e: serde_json::Error) -> AppError {
    AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) }
}

fn load(dir: &Path) -> Result<Vec<Connection>, AppError> {
    match config::read_internal_in(dir, FILE)? {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v).map_err(io),
    }
}

fn store(dir: &Path, all: &[Connection]) -> Result<(), AppError> {
    config::write_internal_in(dir, FILE, &serde_json::to_value(all).map_err(io)?)
}

fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound, None)
}

/// Un archivio delle password che non risponde non nasconde le connessioni: risultano senza password.
fn view(creds: &dyn Secrets, conn: Connection) -> ConnectionView {
    let has_password = matches!(creds.load(&conn.id), Ok(Some(_)));
    ConnectionView { conn, has_password }
}

pub(crate) fn find(dir: &Path, id: &str) -> Result<Connection, AppError> {
    load(dir)?.into_iter().find(|c| c.id == id).ok_or_else(not_found)
}

pub(crate) fn list(dir: &Path, creds: &dyn Secrets) -> Result<Vec<ConnectionView>, AppError> {
    Ok(load(dir)?.into_iter().map(|c| view(creds, c)).collect())
}

/// Un `id` vuoto crea la connessione con un uuid nuovo; un `id` che non esiste dà `NOT_FOUND`. Cambiando host o
/// porta si dimentica l'impronta del vecchio indirizzo. La password si salva solo con `remember`.
pub(crate) fn save(
    dir: &Path,
    creds: &dyn Secrets,
    mut conn: Connection,
    password: Option<String>,
    remember: bool,
) -> Result<ConnectionView, AppError> {
    {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = load(dir)?;
        if conn.id.is_empty() {
            conn.id = uuid::Uuid::new_v4().to_string();
            all.push(conn.clone());
        } else {
            let old = all.iter_mut().find(|c| c.id == conn.id).ok_or_else(not_found)?;
            if old.host != conn.host || old.port != conn.port {
                trust::forget_in(dir, &old.host, old.port)?;
            }
            *old = conn.clone();
        }
        store(dir, &all)?;
    }
    if remember && let Some(pw) = &password {
        creds.store(&conn.id, &conn.user, pw)?;
    }
    Ok(view(creds, conn))
}

/// Elimina anche la password e l'impronta della connessione.
pub(crate) fn delete(dir: &Path, creds: &dyn Secrets, id: &str) -> Result<(), AppError> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(dir)?;
    let i = all.iter().position(|c| c.id == id).ok_or_else(not_found)?;
    // Prima la password: se il Credential Manager rifiuta, la connessione resta e si può riprovare.
    creds.delete(id)?;
    let old = all.remove(i);
    store(dir, &all)?;
    trust::forget_in(dir, &old.host, old.port)
}

#[cfg(test)]
pub(crate) fn sample(scheme: Scheme) -> Connection {
    Connection {
        id: String::new(),
        name: "Server".into(),
        scheme,
        host: "example.org".into(),
        port: 22,
        user: "utente".into(),
        start_dir: "/".into(),
        ftps_implicit: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::credentials::MemSecrets;
    use crate::net::trust::{self, Trust};

    #[test]
    fn connections_roundtrip_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let creds = MemSecrets::default();
        let view = save(d, &creds, sample(Scheme::Sftp), Some("segreto-123".into()), true).unwrap();
        assert!(!view.conn.id.is_empty());
        assert!(view.has_password);
        let text = std::fs::read_to_string(d.join("connections.json")).unwrap();
        assert!(!text.contains("segreto-123"));
        assert!(text.contains("startDir") && text.contains("ftpsImplicit"));
        assert_eq!(list(d, &creds).unwrap(), vec![view.clone()]);
        // Il frontend riceve la connessione appiattita con `hasPassword`, mai la password.
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["hasPassword"], true);
        assert_eq!(json["host"], "example.org");

        // Cambiando host si dimentica l'impronta del vecchio; senza password nuova quella salvata resta.
        trust::accept_in(d, "ssh", "example.org", 22, "SHA256:a").unwrap();
        let moved = Connection { host: "other.org".into(), ..view.conn.clone() };
        let view = save(d, &creds, moved, None, false).unwrap();
        assert!(view.has_password);
        assert_eq!(trust::check_in(d, "ssh", "example.org", 22, "SHA256:a").unwrap(), Trust::Unknown);
        assert_eq!(find(d, &view.conn.id).unwrap().host, "other.org");

        // Un id che non esiste non si crea: gli id nuovi li dà il backend.
        let ghost = Connection { id: "ghost".into(), ..sample(Scheme::Sftp) };
        assert_eq!(save(d, &creds, ghost, None, false).unwrap_err().code, crate::error::ErrorCode::NotFound);

        // Eliminando la connessione se ne vanno anche password e impronta.
        trust::accept_in(d, "ssh", "other.org", 22, "SHA256:a").unwrap();
        delete(d, &creds, &view.conn.id).unwrap();
        assert!(list(d, &creds).unwrap().is_empty());
        assert_eq!(creds.load(&view.conn.id).unwrap(), None);
        assert_eq!(trust::check_in(d, "ssh", "other.org", 22, "SHA256:a").unwrap(), Trust::Unknown);
    }
}
