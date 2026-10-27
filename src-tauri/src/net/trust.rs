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
    /// Solo `ssh`: nome SSH della chiave (`ssh-ed25519`, `ecdsa-sha2-nistp256|384|521`, `ssh-rsa`). Assente nelle voci
    /// di libssh2, che negoziava solo RSA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_type: Option<String>,
}

const FILE: &str = "known_hosts";

/// Serializza lettura-modifica-scrittura del file.
static LOCK: Mutex<()> = Mutex::new(());

/// Chiavi presentate e rifiutate (host, porta, impronta, tipo): `accept_in` ne ricava il tipo da salvare.
static PRESENTED: Mutex<Vec<(String, u16, String, String)>> = Mutex::new(Vec::new());
const PRESENTED_MAX: usize = 16;

/// Ricorda in memoria una chiave host presentata e non ancora accettata; le più vecchie escono oltre 16 voci.
pub(crate) fn presented(host: &str, port: u16, fingerprint: &str, key_type: &str) {
    let mut p = PRESENTED.lock().unwrap_or_else(|e| e.into_inner());
    p.retain(|(h, po, f, _)| !(h.eq_ignore_ascii_case(host) && *po == port && f == fingerprint));
    p.push((host.into(), port, fingerprint.into(), key_type.into()));
    if p.len() > PRESENTED_MAX {
        p.remove(0);
    }
}

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

/// Tipo di chiave della voce `ssh` di host e porta: `ssh-rsa` se manca (voce di libssh2), `None` senza voce.
pub(crate) fn saved_ssh_key_type_in(dir: &Path, host: &str, port: u16) -> Result<Option<String>, AppError> {
    Ok(load(dir)?
        .into_iter()
        .find(|e| e.kind == "ssh" && same(e, host, port))
        .map(|e| e.key_type.unwrap_or_else(|| "ssh-rsa".into())))
}

/// Sostituisce l'impronta già accettata per lo stesso tipo, host e porta (un certificato TLS rinnovato).
/// Per `ssh` il tipo di chiave è quello presentato con la stessa impronta: senza, `INVALID_REQUEST`.
pub(crate) fn accept_in(dir: &Path, kind: &str, host: &str, port: u16, fingerprint: &str) -> Result<(), AppError> {
    let key_type = if kind == "ssh" {
        let p = PRESENTED.lock().unwrap_or_else(|e| e.into_inner());
        let found = p.iter().find(|(h, po, f, _)| h.eq_ignore_ascii_case(host) && *po == port && f == fingerprint);
        Some(found.ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, None))?.3.clone())
    } else {
        None
    };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load(dir)?;
    entries.retain(|e| !(e.kind == kind && same(e, host, port)));
    entries.push(Entry { kind: kind.into(), host: host.into(), port, fingerprint: fingerprint.into(), key_type });
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
        presented("h", 22, "SHA256:a", "ssh-ed25519");
        accept_in(d, "ssh", "h", 22, "SHA256:a").unwrap();
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Known);
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:b").unwrap(), Trust::Changed);
        // Tipo e porta fanno parte della chiave: un certificato o un'altra porta non sono quell'impronta.
        assert_eq!(check_in(d, "tls", "h", 22, "SHA256:a").unwrap(), Trust::Unknown);
        assert_eq!(check_in(d, "ssh", "h", 2222, "SHA256:a").unwrap(), Trust::Unknown);
        forget_in(d, "h", 22).unwrap();
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Unknown);
    }

    #[test]
    fn legacy_entry_reads_as_rsa() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let v = serde_json::json!([{"kind":"ssh","host":"h","port":22,"fingerprint":"SHA256:a"}]);
        config::write_internal_in(d, FILE, &v).unwrap();
        assert_eq!(saved_ssh_key_type_in(d, "h", 22).unwrap().as_deref(), Some("ssh-rsa"));
        assert_eq!(check_in(d, "ssh", "h", 22, "SHA256:a").unwrap(), Trust::Known);
    }

    #[test]
    fn accept_ssh_records_presented_key_type() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        presented("h", 22, "SHA256:b", "ssh-ed25519");
        accept_in(d, "ssh", "h", 22, "SHA256:b").unwrap();
        accept_in(d, "tls", "h", 22, "SHA256:c").unwrap();
        assert_eq!(saved_ssh_key_type_in(d, "h", 22).unwrap().as_deref(), Some("ssh-ed25519"));
        let json = config::read_internal_in(d, FILE).unwrap().unwrap();
        let tls = json.as_array().unwrap().iter().find(|e| e["kind"] == "tls").unwrap();
        assert!(tls.get("key_type").is_none());
    }

    #[test]
    fn accept_ssh_without_presented_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(accept_in(d, "ssh", "h2", 22, "SHA256:z").unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(check_in(d, "ssh", "h2", 22, "SHA256:z").unwrap(), Trust::Unknown);
    }

    #[test]
    fn saved_key_type_matches_host_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        presented("Example.ORG", 22, "SHA256:d", "ecdsa-sha2-nistp256");
        accept_in(d, "ssh", "Example.ORG", 22, "SHA256:d").unwrap();
        assert_eq!(saved_ssh_key_type_in(d, "example.org", 22).unwrap().as_deref(), Some("ecdsa-sha2-nistp256"));
        assert_eq!(saved_ssh_key_type_in(d, "example.org", 2222).unwrap(), None);
    }
}
