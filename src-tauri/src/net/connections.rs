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

/// Un host è un nome, un IPv4 o un IPv6 senza parentesi: vuoto no, e niente spazi, caratteri di controllo né
/// `/ \ ? # @ [ ]`, che finirebbero nel percorso o nell'utente di un URL (`http://intranet/logs:80`).
pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control() || "/\\?#@[]".contains(c))
}

/// Un host non valido (vedi `valid_host`) dà `INVALID_REQUEST`. Un `id` vuoto crea la connessione con un uuid nuovo;
/// un `id` che non esiste dà `NOT_FOUND`. Cambiando host o porta si dimentica l'impronta del vecchio indirizzo. La
/// password si salva solo con `remember`.
pub(crate) fn save(
    dir: &Path,
    creds: &dyn Secrets,
    mut conn: Connection,
    password: Option<String>,
    remember: bool,
) -> Result<ConnectionView, AppError> {
    if !valid_host(&conn.host) {
        return Err(AppError::new(ErrorCode::InvalidRequest, None));
    }
    let is_new = conn.id.is_empty();
    {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = load(dir)?;
        if is_new {
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
    if remember
        && let Some(pw) = &password
        && let Err(e) = creds.store(&conn.id, &conn.user, pw)
    {
        // Una connessione nuova senza la sua password non resta: il frontend non ne ha ricevuto l'id e riprovando
        // creerebbe un doppione. Una modifica resta salvata (riprovando si riscrive la stessa).
        if is_new {
            let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
            if let Ok(mut all) = load(dir) {
                all.retain(|c| c.id != conn.id);
                let _ = store(dir, &all);
            }
        }
        return Err(e);
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
    use crate::net::credentials::{MemSecrets, Refuses};
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
        trust::presented("example.org", 22, "SHA256:a", "ssh-ed25519");
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
        trust::presented("other.org", 22, "SHA256:a", "ssh-ed25519");
        trust::accept_in(d, "ssh", "other.org", 22, "SHA256:a").unwrap();
        delete(d, &creds, &view.conn.id).unwrap();
        assert!(list(d, &creds).unwrap().is_empty());
        assert!(creds.load(&view.conn.id).unwrap().is_none());
        assert_eq!(trust::check_in(d, "ssh", "other.org", 22, "SHA256:a").unwrap(), Trust::Unknown);
    }

    /// Un host con `/` diventerebbe parte dell'URL (`http://intranet/logs:80`): rifiutato, come gli altri caratteri che
    /// cambiano il significato di un indirizzo. Nomi, IPv4 e IPv6 senza parentesi passano.
    #[test]
    fn host_is_validated_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let creds = MemSecrets::default();
        for bad in ["", " ", "intranet/logs", "a b", "a\\b", "a?b", "a#b", "u@h", "h\n", "h\0", "[::1]", " h"] {
            let c = Connection { host: bad.into(), ..sample(Scheme::Http) };
            assert_eq!(save(dir.path(), &creds, c, None, false).unwrap_err().code, ErrorCode::InvalidRequest, "{bad:?}");
        }
        assert!(list(dir.path(), &creds).unwrap().is_empty());
        for good in ["example.org", "10.0.0.1", "::1", "fe80::1", "srv-01"] {
            let c = Connection { host: good.into(), ..sample(Scheme::Http) };
            assert_eq!(save(dir.path(), &creds, c, None, false).unwrap().conn.host, good);
        }
    }

    #[test]
    fn new_connection_is_not_kept_if_its_password_cannot_be_stored() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let e = save(d, &Refuses, sample(Scheme::Sftp), Some("pw".into()), true).unwrap_err();
        assert_eq!(e.code, ErrorCode::AccessDenied);
        // Il frontend non ha ricevuto l'id: riprovando non deve nascere un doppione.
        assert!(list(d, &Refuses).unwrap().is_empty());
    }
}
