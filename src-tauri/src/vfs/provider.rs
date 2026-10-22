//! Provider di cartella: l'interfaccia comune alle sorgenti remote (e al locale, solo per i trasferimenti).

use super::loc::Scheme;
use crate::error::AppError;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

/// Metadati di un elemento. `is_dir` vale anche per un collegamento a cartella, che ha in più `dir_link`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub name: String,
    pub is_dir: bool,
    pub dir_link: bool,
    pub size: u64,
    /// Millisecondi dall'epoca Unix, se il server la dà.
    pub modified_ms: Option<i64>,
    /// Permessi POSIX, dove ci sono.
    pub mode: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caps {
    pub writable: bool,
    /// `rename(replace = true)` sostituisce un file esistente in un colpo solo.
    pub atomic_replace: bool,
    /// Date al secondo o meglio (FTP senza `MDTM`: al minuto o all'anno).
    pub precise_mtime: bool,
    pub ranged_read: bool,
}

/// Tutti i `path` sono percorsi POSIX del provider (il locale usa il percorso Windows).
/// Gli errori degli stream di `reader`/`writer` portano l'`AppError` dentro `io::Error::other`: vedi `stream_error`.
pub trait Provider: Send + Sync {
    fn caps(&self) -> Caps;
    fn stat(&self, path: &str) -> Result<Stat, AppError>;
    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError>;
    /// Fino a `len` byte da `offset`: più corto solo a fine file.
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError>;
    fn reader(&self, path: &str) -> Result<Box<dyn std::io::Read + Send>, AppError>;
    /// Con `create_new` fallisce (`ALREADY_EXISTS`) se il file c'è; senza, lo crea o lo tronca. `flush` alla fine
    /// conclude la scrittura e ne dà l'esito (in FTP la risposta finale del server); dopo non si scrive più.
    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn std::io::Write + Send>, AppError>;
    /// Senza `replace` una destinazione esistente dà `ALREADY_EXISTS`.
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError>;
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError>;
    fn mkdir(&self, path: &str) -> Result<(), AppError>;
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError>;
}

/// Navigazione e trasferimenti usano sessioni diverse: la lista non aspetta un upload in corso.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Browse,
    Transfer,
}

pub trait Connector: Send + Sync {
    fn provider(&self, scheme: Scheme, conn: &str, purpose: Purpose) -> Result<Arc<dyn Provider>, AppError>;
}

/// Errore di uno stream di `reader`/`writer`: l'`AppError` del provider se c'è, altrimenti la traduzione di `from_io`.
pub fn stream_error(e: std::io::Error, path: &str) -> AppError {
    e.downcast::<AppError>().unwrap_or_else(|e| AppError::from_io(&e, Path::new(path)))
}
