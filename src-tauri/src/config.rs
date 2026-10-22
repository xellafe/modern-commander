//! Lettura e scrittura dei file di configurazione JSON (`settings`, `keybindings`, `hotlist`), più quelli interni
//! del backend (`connections`, `known_hosts`) che il frontend non legge né scrive.
//! Modalità portable: se accanto all'exe c'è un file `portable`, si usa `<exe>\config`,
//! altrimenti `%APPDATA%\ModernCommander`.

use crate::error::{AppError, ErrorCode};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub fn config_dir(exe_dir: &Path, appdata: &Path) -> PathBuf {
    if exe_dir.join("portable").exists() {
        exe_dir.join("config")
    } else {
        appdata.join("ModernCommander")
    }
}

fn validate_name(name: &str) -> Result<(), AppError> {
    match name {
        "settings" | "keybindings" | "hotlist" => Ok(()),
        _ => Err(AppError::new(ErrorCode::InvalidRequest, None)),
    }
}

/// File che solo il backend legge e scrive: mai raggiungibili da `read_config`/`write_config`.
fn validate_internal(name: &str) -> Result<(), AppError> {
    match name {
        "connections" | "known_hosts" => Ok(()),
        _ => Err(AppError::new(ErrorCode::InvalidRequest, None)),
    }
}

pub(crate) fn current_config_dir() -> Result<PathBuf, AppError> {
    let invalid = || AppError::new(ErrorCode::InvalidRequest, None);
    let exe = std::env::current_exe().map_err(|e| AppError::from_io(&e, Path::new("")))?;
    let exe_dir = exe.parent().ok_or_else(invalid)?;
    let appdata = std::env::var_os("APPDATA").ok_or_else(invalid)?;
    Ok(config_dir(exe_dir, Path::new(&appdata)))
}

fn read(name: &str) -> Result<Option<serde_json::Value>, AppError> {
    read_in(&current_config_dir()?, name)
}

fn read_in(dir: &Path, name: &str) -> Result<Option<serde_json::Value>, AppError> {
    validate_name(name)?;
    read_file(dir, name)
}

pub(crate) fn read_internal_in(dir: &Path, name: &str) -> Result<Option<serde_json::Value>, AppError> {
    validate_internal(name)?;
    read_file(dir, name)
}

fn read_file(dir: &Path, name: &str) -> Result<Option<serde_json::Value>, AppError> {
    let path = dir.join(format!("{name}.json"));
    let res = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| AppError {
            detail: Some(e.to_string()),
            ..AppError::new(ErrorCode::Io, Some(&path))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::from_io(&e, &path)),
    };
    if res.is_err() {
        lock(&READ_FAILED).insert(path);
    }
    res
}

/// Serializza le scritture: tutte usano lo stesso `<name>.json.tmp`.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// File che in questa sessione non si sono potuti leggere: il frontend è partito con i default.
static READ_FAILED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// I lock non proteggono invarianti: si ignora l'avvelenamento.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Prima di sostituire un file che non è JSON valido, o che in questa sessione non si è potuto
/// leggere, lo si copia in `<name>.json.bak` (resta il più recente): la prima modifica fatta
/// partendo dai default non deve far perdere il file dell'utente. Se la copia non riesce la
/// scrittura fallisce.
fn backup_if_unreadable(path: &Path) -> Result<(), AppError> {
    let mut failed = lock(&READ_FAILED);
    let unreadable = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes).is_err(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    };
    if unreadable || failed.contains(path) {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        match std::fs::copy(path, &bak) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::from_io(&e, path)),
        }
    }
    failed.remove(path);
    Ok(())
}

fn write_in(dir: &Path, name: &str, value: &serde_json::Value) -> Result<(), AppError> {
    validate_name(name)?;
    write_file(dir, name, value)
}

pub(crate) fn write_internal_in(dir: &Path, name: &str, value: &serde_json::Value) -> Result<(), AppError> {
    validate_internal(name)?;
    write_file(dir, name, value)
}

fn write_file(dir: &Path, name: &str, value: &serde_json::Value) -> Result<(), AppError> {
    let path = dir.join(format!("{name}.json"));
    let tmp = dir.join(format!("{name}.json.tmp"));
    let text = serde_json::to_string_pretty(value).map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, Some(&path))
    })?;
    let _guard = lock(&WRITE_LOCK);
    std::fs::create_dir_all(dir).map_err(|e| AppError::from_io(&e, dir))?;
    backup_if_unreadable(&path)?;
    // Scrittura su file temporaneo e rinomina: il file di destinazione non resta mai a metà.
    std::fs::write(&tmp, text).map_err(|e| AppError::from_io(&e, &tmp))?;
    std::fs::rename(&tmp, &path).map_err(|e| AppError::from_io(&e, &path))
}

fn write(name: &str, value: &serde_json::Value) -> Result<(), AppError> {
    write_in(&current_config_dir()?, name, value)
}
#[tauri::command]
pub async fn read_config(name: String) -> Result<Option<serde_json::Value>, AppError> {
    crate::fs::blocking(move || read(&name)).await?
}

#[tauri::command]
pub async fn write_config(name: String, value: serde_json::Value) -> Result<(), AppError> {
    crate::fs::blocking(move || write(&name, &value)).await?
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn portable_marker_switches_dir() {
        let exe = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        assert_eq!(config_dir(exe.path(), app.path()), app.path().join("ModernCommander"));
        std::fs::write(exe.path().join("portable"), b"").unwrap();
        assert_eq!(config_dir(exe.path(), app.path()), exe.path().join("config"));
    }

    #[test]
    fn rejects_unknown_config_names() {
        assert_eq!(validate_name("../evil").unwrap_err().code, ErrorCode::InvalidRequest);
        assert!(validate_name("settings").is_ok());
        // I file interni non passano dall'IPC, e le funzioni interne non leggono quelli del frontend.
        assert!(validate_name("connections").is_err() && validate_name("known_hosts").is_err());
        assert!(validate_internal("connections").is_ok() && validate_internal("settings").is_err());
    }

    #[test]
    fn malformed_file_is_backed_up_before_the_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let (path, bak) = (dir.path().join("hotlist.json"), dir.path().join("hotlist.json.bak"));
        std::fs::write(&path, "[{\"rotto\"").unwrap();
        assert!(read_in(dir.path(), "hotlist").is_err());
        write_in(dir.path(), "hotlist", &serde_json::json!([])).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "[{\"rotto\"");
        assert_eq!(read_in(dir.path(), "hotlist").unwrap(), Some(serde_json::json!([])));
        // Il file ora è il nostro: le scritture successive non toccano la copia dell'utente.
        write_in(dir.path(), "hotlist", &serde_json::json!([1])).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "[{\"rotto\"");
    }

    #[test]
    fn file_unreadable_at_startup_is_backed_up_even_if_valid_later() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (path, bak) = (dir.path().join("settings.json"), dir.path().join("settings.json.bak"));
        std::fs::write(&path, "{\"theme\":\"dark\"}").unwrap();
        // Un altro programma lo tiene aperto in esclusiva (antivirus, sincronizzazione): il frontend
        // parte con i default.
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert_eq!(read_in(dir.path(), "settings").unwrap_err().code, ErrorCode::FileInUse);
        drop(lock);
        write_in(dir.path(), "settings", &serde_json::json!({"theme": "light"})).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{\"theme\":\"dark\"}");
        // Un file valido letto senza errori non si copia.
        std::fs::remove_file(&bak).unwrap();
        assert!(read_in(dir.path(), "settings").unwrap().is_some());
        write_in(dir.path(), "settings", &serde_json::json!({})).unwrap();
        assert!(!bak.exists());
    }

    #[test]
    fn concurrent_writes_do_not_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let values: Vec<_> = (0..16).map(|i| serde_json::json!({ "n": i })).collect();
        let handles: Vec<_> = values
            .iter()
            .cloned()
            .map(|v| {
                let d = dir.path().to_path_buf();
                std::thread::spawn(move || write_in(&d, "settings", &v))
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        let got: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(values.contains(&got));
        assert!(!dir.path().join("settings.json.tmp").exists());
    }
}