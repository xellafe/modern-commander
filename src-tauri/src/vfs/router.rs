//! Instradamento delle posizioni remote al provider della loro connessione.

use super::local::LocalProvider;
use super::loc::Loc;
use super::provider::{Connector, Provider, Purpose};
use crate::error::{AppError, ErrorCode};
use std::sync::Arc;

pub struct Vfs {
    connector: Arc<dyn Connector>,
}

impl Vfs {
    pub fn new(connector: Arc<dyn Connector>) -> Self {
        Self { connector }
    }

    /// Provider e percorso POSIX di una posizione remota; un percorso locale dà `INVALID_REQUEST`.
    pub fn remote(&self, loc: &Loc, purpose: Purpose) -> Result<(Arc<dyn Provider>, String), AppError> {
        match loc {
            Loc::Remote { scheme, conn, path } => Ok((self.connector.provider(*scheme, conn, purpose)?, path.clone())),
            Loc::Local(_) => Err(AppError::new(ErrorCode::InvalidRequest, None)),
            // Ancora nessun provider d'archivio: lo sostituisce il Task 4.
            Loc::Archive { .. } => Err(AppError::new(ErrorCode::Unsupported, None)),
        }
    }

    /// Come `remote`, ma un percorso locale dà `LocalProvider` con il percorso Windows: solo per i trasferimenti
    /// tra provider diversi, il resto del locale non passa da qui.
    pub fn endpoint(&self, loc: &Loc, purpose: Purpose) -> Result<(Arc<dyn Provider>, String), AppError> {
        match loc {
            Loc::Local(p) => Ok((Arc::new(LocalProvider), p.to_string_lossy().into_owned())),
            Loc::Remote { .. } | Loc::Archive { .. } => self.remote(loc, purpose),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::mem::{MemConnector, MemProvider};
    use std::collections::HashMap;

    fn vfs_with(m: &Arc<MemProvider>) -> Vfs {
        Vfs::new(Arc::new(MemConnector(HashMap::from([("m".to_owned(), m.clone())]))))
    }

    #[test]
    fn local_is_not_routed_to_providers() {
        let vfs = vfs_with(&Arc::new(MemProvider::new()));
        let local = Loc::parse("C:\\a").unwrap();
        assert_eq!(vfs.remote(&local, Purpose::Browse).err().unwrap().code, ErrorCode::InvalidRequest);
        // `endpoint` (solo per i trasferimenti) dà il provider locale con il percorso Windows.
        assert_eq!(vfs.endpoint(&local, Purpose::Transfer).unwrap().1, "C:\\a");
    }

    #[test]
    fn remote_reaches_its_provider() {
        let m = Arc::new(MemProvider::new());
        let vfs = vfs_with(&m);
        let (p, path) = vfs.remote(&Loc::parse("sftp://m/a").unwrap(), Purpose::Browse).unwrap();
        assert_eq!(path, "/a");
        assert!(std::ptr::addr_eq(Arc::as_ptr(&p), Arc::as_ptr(&m)));
        let other = Loc::parse("sftp://n/a").unwrap();
        assert_eq!(vfs.remote(&other, Purpose::Browse).err().unwrap().code, ErrorCode::NotFound);
    }
}
