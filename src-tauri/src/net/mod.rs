//! Connessioni salvate, password nel Credential Manager, impronte accettate e sessioni verso i server.

pub mod connections;
pub mod credentials;
pub mod ftp;
pub mod http_auth;
pub mod http_index;
pub mod sessions;
pub mod sftp;
pub mod temp;
pub mod tls;
pub mod trust;
pub mod winscp;

use crate::error::{AppError, ErrorCode};
use crate::fs::blocking;
use connections::{Connection, ConnectionView};
use sessions::Sessions;
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use winscp::ImportCandidate;

type Net<'a> = State<'a, Arc<Sessions>>;

/// Voci al massimo in un elenco remoto, per tutti i provider: oltre, `TOO_LARGE` (un server ostile non riempie la
/// memoria).
pub(crate) const MAX_ENTRIES: usize = 200_000;

/// Attesa massima per collegarsi, per SFTP, FTP e HTTP.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Attesa massima di una risposta dopo il collegamento: con la VPN caduta si ha `TIMEOUT`, non un'attesa infinita.
pub(crate) const IO_TIMEOUT_MS: u32 = 30_000;

pub(crate) fn tcp(host: &str, port: u16) -> Result<TcpStream, AppError> {
    let fail = |code, e: io::Error| AppError { detail: Some(e.to_string()), ..AppError::new(code, None) };
    let addrs = (host, port).to_socket_addrs().map_err(|e| fail(ErrorCode::ConnectionFailed, e))?;
    let mut last = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    let code = if last.kind() == io::ErrorKind::TimedOut { ErrorCode::Timeout } else { ErrorCode::ConnectionFailed };
    Err(fail(code, last))
}

/// Lascia cadere `v` su un thread a parte. Con la rete bloccata (VPN caduta) la chiusura di una connessione aspetta il
/// timeout: chi lascia cadere il provider, spesso il thread di una richiesta dopo un `TIMEOUT`, non deve aspettare. Se
/// il thread non parte, si chiude qui.
pub(crate) fn drop_in_background<T: Send + 'static>(v: T) {
    let _ = std::thread::Builder::new().name("net-close".into()).spawn(move || drop(v));
}

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

/// I siti salvati di WinSCP, per l'anteprima (non si salva niente).
#[tauri::command]
pub async fn winscp_candidates() -> Result<Vec<ImportCandidate>, AppError> {
    blocking(winscp::from_system).await?
}

#[tauri::command]
pub async fn import_connections(net: Net<'_>, items: Vec<ImportCandidate>) -> Result<Vec<ConnectionView>, AppError> {
    let s = net.inner().clone();
    blocking(move || s.import(items)).await?
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

/// `false`: connessi, ma la password chiesta con `remember` non si è potuta salvare.
#[tauri::command]
pub async fn connect(net: Net<'_>, id: String, password: Option<String>, remember: bool) -> Result<bool, AppError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_happens_off_the_calling_thread() {
        use std::sync::mpsc::{Sender, channel};
        use std::time::Duration;
        // Chiusura lenta che aspetta il via del test: eseguita sul thread del chiamante, il via arriverebbe tardi.
        struct Slow(std::sync::mpsc::Receiver<()>, Sender<(std::thread::ThreadId, bool)>);
        impl Drop for Slow {
            fn drop(&mut self) {
                let go = self.0.recv_timeout(Duration::from_secs(2)).is_ok();
                self.1.send((std::thread::current().id(), go)).unwrap();
            }
        }
        let (go_tx, go_rx) = channel();
        let (done_tx, done_rx) = channel();
        drop_in_background(Slow(go_rx, done_tx));
        go_tx.send(()).unwrap();
        let (id, go) = done_rx.recv().unwrap();
        assert!(go);
        assert_ne!(id, std::thread::current().id());
    }
}
