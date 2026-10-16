//! Unico tipo di errore restituito al frontend: solo codice e dati, mai testo da mostrare.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    AccessDenied,
    AlreadyExists,
    DiskFull,
    NotReady,
    SameFile,
    IntoItself,
    /// Spostamento tra unità diverse: va fatto come copia + cancellazione.
    NotSameDevice,
    /// Il volume non ha Cestino (rete, rimovibile, ...): serve la cancellazione definitiva.
    NoRecycleBin,
    InvalidName,
    Cancelled,
    InvalidRequest,
    Io,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub path: Option<String>,
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode, path: Option<&Path>) -> Self {
        Self {
            code,
            path: path.map(|p| p.to_string_lossy().into_owned()),
            detail: None,
        }
    }

    /// Mappa i codici di errore Win32 più comuni; il testo dell'errore originale va in `detail`.
    pub fn from_io(err: &std::io::Error, path: &Path) -> Self {
        let code = match err.raw_os_error() {
            Some(2 | 3) => ErrorCode::NotFound,
            Some(5) => ErrorCode::AccessDenied,
            Some(80 | 183) => ErrorCode::AlreadyExists,
            Some(112) => ErrorCode::DiskFull,
            Some(21) => ErrorCode::NotReady,
            Some(123) => ErrorCode::InvalidName,
            Some(17) => ErrorCode::NotSameDevice,
            Some(1235) => ErrorCode::Cancelled,
            _ => ErrorCode::Io,
        };
        Self {
            detail: Some(err.to_string()),
            ..Self::new(code, Some(path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_codes_map() {
        let e = |n| AppError::from_io(&std::io::Error::from_raw_os_error(n), Path::new("x")).code;
        assert_eq!(e(2), ErrorCode::NotFound);
        assert_eq!(e(3), ErrorCode::NotFound);
        assert_eq!(e(5), ErrorCode::AccessDenied);
        assert_eq!(e(80), ErrorCode::AlreadyExists);
        assert_eq!(e(183), ErrorCode::AlreadyExists);
        assert_eq!(e(112), ErrorCode::DiskFull);
        assert_eq!(e(21), ErrorCode::NotReady);
        assert_eq!(e(123), ErrorCode::InvalidName);
        assert_eq!(e(17), ErrorCode::NotSameDevice);
        assert_eq!(e(1235), ErrorCode::Cancelled);
        assert_eq!(e(9999), ErrorCode::Io);
    }

    #[test]
    fn serializes_camel_case_and_screaming_codes() {
        let e = AppError::new(ErrorCode::AccessDenied, Some(Path::new("x")));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, serde_json::json!({"code": "ACCESS_DENIED", "path": "x", "detail": null}));
        assert_eq!(serde_json::from_value::<AppError>(v).unwrap(), e);
    }
}
