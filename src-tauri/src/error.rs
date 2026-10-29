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
    /// File aperto o bloccato da un altro programma (violazione di condivisione o di blocco).
    FileInUse,
    /// Elemento che l'operazione non sa trattare (per esempio una giunzione in Copy, detail `reparse_point`).
    Unsupported,
    Cancelled,
    InvalidRequest,
    /// File oltre il limite che l'operazione accetta (lettura intera, ...).
    TooLarge,
    /// Il file è cambiato su disco dopo l'apertura: va riletto prima di salvare.
    ChangedOnDisk,
    /// Il testo contiene caratteri che la codifica scelta non sa rappresentare.
    UnmappableChars,
    /// Archivio danneggiato o non leggibile.
    CorruptArchive,
    /// 7z i cui decoder chiederebbero troppa memoria (dizionario oltre 256 MiB, vedi `archive::sevenz_guard`).
    ArchiveTooDemanding,
    /// La sorgente remota chiede una password che non è salvata.
    AuthRequired,
    /// Credenziali rifiutate dal server.
    AuthFailed,
    /// Connessione non riuscita o caduta.
    ConnectionFailed,
    Timeout,
    /// Impronta dell'host SSH mai vista: `detail` porta `SHA256:<base64>`.
    HostKeyUnknown,
    /// Impronta dell'host SSH diversa da quella accettata: `detail` porta `SHA256:<base64>`.
    HostKeyChanged,
    /// Il server offre solo tipi di chiave dell'host che l'app non gestisce: `detail` porta quelli offerti.
    HostKeyUnsupported,
    /// Certificato TLS che Windows non accetta: `detail` porta `SHA256:<base64>`.
    CertUntrusted,
    /// Il server non permette la scrittura sicura (temporaneo e rinomina): serve la conferma dell'utente.
    UnsafeWrite,
    /// Sorgente in sola lettura (HTTP).
    ReadOnlySource,
    /// Il server non supporta le letture a intervalli (`Range`).
    RangeUnsupported,
    /// Il server SSH non ha in comune con l'app lo scambio di chiavi, il cifrario o il MAC: `detail` porta il messaggio
    /// della libreria. Per la chiave dell'host c'è `HostKeyUnsupported`.
    AlgorithmUnsupported,
    Io,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub path: Option<String>,
    pub detail: Option<String>,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Serve per far viaggiare un `AppError` dentro un `std::io::Error` (gli stream dei provider).
impl std::error::Error for AppError {}

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
            Some(32 | 33 | 1175) => ErrorCode::FileInUse,
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
        assert_eq!(e(32), ErrorCode::FileInUse); // ERROR_SHARING_VIOLATION
        assert_eq!(e(33), ErrorCode::FileInUse); // ERROR_LOCK_VIOLATION
        assert_eq!(e(1175), ErrorCode::FileInUse); // ERROR_UNABLE_TO_REMOVE_REPLACED (ReplaceFileW)
        assert_eq!(serde_json::to_value(ErrorCode::FileInUse).unwrap(), "FILE_IN_USE");
        assert_eq!(e(9999), ErrorCode::Io);
    }

    #[test]
    fn new_codes_serialize_screaming_snake() {
        let s = |c| serde_json::to_value(c).unwrap();
        assert_eq!(s(ErrorCode::TooLarge), "TOO_LARGE");
        assert_eq!(s(ErrorCode::ChangedOnDisk), "CHANGED_ON_DISK");
        assert_eq!(s(ErrorCode::UnmappableChars), "UNMAPPABLE_CHARS");
        assert_eq!(s(ErrorCode::CorruptArchive), "CORRUPT_ARCHIVE");
        assert_eq!(s(ErrorCode::ArchiveTooDemanding), "ARCHIVE_TOO_DEMANDING");
    }

    #[test]
    fn network_codes_serialize_screaming_snake() {
        let s = |c| serde_json::to_value(c).unwrap();
        assert_eq!(s(ErrorCode::AuthRequired), "AUTH_REQUIRED");
        assert_eq!(s(ErrorCode::AuthFailed), "AUTH_FAILED");
        assert_eq!(s(ErrorCode::ConnectionFailed), "CONNECTION_FAILED");
        assert_eq!(s(ErrorCode::Timeout), "TIMEOUT");
        assert_eq!(s(ErrorCode::HostKeyUnknown), "HOST_KEY_UNKNOWN");
        assert_eq!(s(ErrorCode::HostKeyChanged), "HOST_KEY_CHANGED");
        assert_eq!(s(ErrorCode::HostKeyUnsupported), "HOST_KEY_UNSUPPORTED");
        assert_eq!(s(ErrorCode::CertUntrusted), "CERT_UNTRUSTED");
        assert_eq!(s(ErrorCode::UnsafeWrite), "UNSAFE_WRITE");
        assert_eq!(s(ErrorCode::ReadOnlySource), "READ_ONLY_SOURCE");
        assert_eq!(s(ErrorCode::RangeUnsupported), "RANGE_UNSUPPORTED");
        assert_eq!(s(ErrorCode::AlgorithmUnsupported), "ALGORITHM_UNSUPPORTED");
    }

    #[test]
    fn serializes_camel_case_and_screaming_codes() {
        let e = AppError::new(ErrorCode::AccessDenied, Some(Path::new("x")));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, serde_json::json!({"code": "ACCESS_DENIED", "path": "x", "detail": null}));
        assert_eq!(serde_json::from_value::<AppError>(v).unwrap(), e);
    }
}
