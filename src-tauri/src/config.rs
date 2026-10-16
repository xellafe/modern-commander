//! Lettura e scrittura dei file di configurazione JSON (`settings`, `keybindings`, `hotlist`).
//! Modalità portable: se accanto all'exe c'è un file `portable`, si usa `<exe>\config`,
//! altrimenti `%APPDATA%\ModernCommander`.

use crate::error::{AppError, ErrorCode};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

fn current_config_dir() -> Result<PathBuf, AppError> {
    let invalid = || AppError::new(ErrorCode::InvalidRequest, None);
    let exe = std::env::current_exe().map_err(|e| AppError::from_io(&e, Path::new("")))?;
    let exe_dir = exe.parent().ok_or_else(invalid)?;
    let appdata = std::env::var_os("APPDATA").ok_or_else(invalid)?;
    Ok(config_dir(exe_dir, Path::new(&appdata)))
}

fn read(name: &str) -> Result<Option<serde_json::Value>, AppError> {
    validate_name(name)?;
    let path = current_config_dir()?.join(format!("{name}.json"));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AppError::from_io(&e, &path)),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, Some(&path))
    })
}

/// Serializza le scritture: tutte usano lo stesso `<name>.json.tmp`.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn write_in(dir: &Path, name: &str, value: &serde_json::Value) -> Result<(), AppError> {
    validate_name(name)?;
    let path = dir.join(format!("{name}.json"));
    let tmp = dir.join(format!("{name}.json.tmp"));
    let text = serde_json::to_string_pretty(value).map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, Some(&path))
    })?;
    // Il lock non protegge dati: si ignora l'avvelenamento.
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(dir).map_err(|e| AppError::from_io(&e, dir))?;
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