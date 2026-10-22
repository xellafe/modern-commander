//! Impronte accettate (chiavi degli host SSH e certificati TLS), nel file interno `known_hosts`.

use crate::config;
use crate::error::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, PartialEq)]
pub enum Trust {
    Known,
    Unknown,
    Changed,
}

/// `kind`: `"ssh"` o `"tls"`; `fingerprint`: `SHA256:<base64>`.
#[derive(Serialize, Deserialize)]
struct Entry {
    kind: String,
    host: String,
    port: u16,
    fingerprint: String,
}

const FILE: &str = "known_hosts";

/// Serializza lettura-modifica-scrittura del file.
static LOCK: Mutex<()> = Mutex::new(());

fn load(dir: &Path) -> Result<Vec<Entry>, AppError> {
    match config::read_internal_in(dir, FILE)? {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v).map_err(|e| AppError {
            detail: Some(e.to_string()),
            ..AppError::new(ErrorCode::Io, None)
        }),
    }
}

fn save(dir: &Path, entries: &[Entry]) -> Result<(), AppError> {
    let value = serde_json::to_value(entries).map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, None)
    })?;
    config::write_internal_in(dir, FILE, &value)
}

/// I nomi DNS non distinguono maiuscole e minuscole.
fn same(e: &Entry, host: &str, port: u16) -> bool {
    e.host.eq_ignore_ascii_case(host) && e.port == port
}

pub fn check(kind: &str, host: &str, port: u16, fingerprint: &str) -> Result<Trust, AppError> {
    check_in(&config::current_config_dir()?, kind, host, port, fingerprint)
}

pub fn accept(kind: &str, host: &str, port: u16, fingerprint: &str) -> Result<(), AppError> {
    accept_in(&config::current_config_dir()?, kind, host, port, fingerprint)
}

pub fn forget(host: &str, port: u16) -> Result<(), AppError> {
    forget_in(&config::current_config_dir()?, host, port)
}

pub(crate) fn check_in(dir: &Path, kind: &str, host: &str, port: u16, fingerprint: &str) -> Result<Trust, AppError> {
    Ok(match load(dir)?.iter().find(|e| e.kind == kind && same(e, host, port)) {
        None => Trust::Unknown,
        Some(e) if e.fingerprint == fingerprint => Trust::Known,
        Some(_) => Trust::Changed,
    })
}

/// Sostituisce l'impronta già accettata per lo stesso tipo, host e porta (un certificato TLS rinnovato).
pub(crate) fn accept_in(dir: &Path, kind: &str, host: &str, port: u16, fingerprint: &str) -> Result<(), AppError> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load(dir)?;
    entries.retain(|e| !(e.kind == kind && same(e, host, port)));
    entries.push(Entry { kind: kind.into(), host: host.into(), port, fingerprint: fingerprint.into() });
    save(dir, &entries)
}

/// Dimentica le impronte di ogni tipo per host e porta.
pub(crate) fn forget_in(dir: &Path, host: &str, port: u16) -> Result<(), AppError> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load(dir)?;
    let before = entries.len();
    entries.retain(|e| !same(e, host, port));
    if entries.len() == before {
        return Ok(());
    }
    save(dir, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_unknown_changed_known() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Unknown);
        accept_in(d, "ssh", "h", 22, "SHA256:a").unwrap();
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Known);
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:b").unwrap(), Trust::Changed);
        // Tipo e porta fanno parte della chiave: un certificato o un'altra porta non sono quell'impronta.
        assert_eq!(check_in(d, "tls", "h", 22, "SHA256:a").unwrap(), Trust::Unknown);
        assert_eq!(check_in(d, "ssh", "h", 2222, "SHA256:a").unwrap(), Trust::Unknown);
        forget_in(d, "h", 22).unwrap();
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Unknown);
    }
}
