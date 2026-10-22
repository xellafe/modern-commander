//! Condivisioni disco di un server SMB (`\\server`), come le mostra Explorer.

use crate::error::AppError;
use crate::fs::listing::Entry;
use std::path::Path;
use windows::Win32::Foundation::{ERROR_MORE_DATA, WIN32_ERROR};
use windows::Win32::NetworkManagement::NetManagement::{MAX_PREFERRED_LENGTH, NetApiBufferFree};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE,
};
use windows::core::HSTRING;

/// Bit di tipo di `shi1_type` (`STYPE_MASK`): sopra ci sono flag come `STYPE_SPECIAL` delle condivisioni `C$`.
const STYPE_MASK: u32 = 0xFF;

/// Il nome del server se `path` è `\\server` o `\\server\`, senza condivisione.
pub fn is_server_root(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let name = s.strip_prefix("\\\\")?;
    let name = name.strip_suffix('\\').unwrap_or(name);
    // `\\?\` e `\\.\` sono prefissi di percorso, non server.
    (!name.is_empty() && !name.contains('\\') && !name.starts_with(['?', '.'])).then(|| name.to_owned())
}

/// Condivisioni disco di `server` come cartelle; quelle amministrative (`C$`) sono nascoste.
// ponytail: nessun timeout nostro, vale quello di sistema di SMB (gira fuori dal thread principale); se serve, thread con attesa limitata.
pub fn list_shares(server: &str) -> Result<Vec<Entry>, AppError> {
    let unc = format!("\\\\{server}");
    let host = HSTRING::from(&unc);
    let mut out = Vec::new();
    let mut resume = 0u32;
    loop {
        let (mut buf, mut read, mut total) = (std::ptr::null_mut::<u8>(), 0u32, 0u32);
        let rc = unsafe { NetShareEnum(&host, 1, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total, Some(&mut resume)) };
        let more = WIN32_ERROR(rc) == ERROR_MORE_DATA;
        if rc != 0 && !more {
            if !buf.is_null() {
                unsafe { NetApiBufferFree(Some(buf as _)) };
            }
            return Err(AppError::from_io(&std::io::Error::from_raw_os_error(rc as i32), Path::new(&unc)));
        }
        if !buf.is_null() {
            let infos = unsafe { std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize) };
            out.extend(infos.iter().filter(|i| i.shi1_type.0 & STYPE_MASK == STYPE_DISKTREE.0).map(|i| {
                let name = String::from_utf16_lossy(unsafe { i.shi1_netname.as_wide() });
                Entry {
                    is_dir: true,
                    size: 0,
                    modified_ms: 0,
                    created_ms: 0,
                    accessed_ms: 0,
                    attributes: if name.ends_with('$') { FILE_ATTRIBUTE_HIDDEN.0 } else { 0 },
                    is_symlink: false,
                    mode: None,
                    name,
                }
            }));
            unsafe { NetApiBufferFree(Some(buf as _)) };
        }
        if !more {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_root_detection() {
        let r = |s: &str| is_server_root(Path::new(s));
        assert_eq!(r("\\\\srv"), Some("srv".into()));
        assert_eq!(r("\\\\srv\\"), Some("srv".into()));
        assert_eq!(r("\\\\srv\\share"), None);
        assert_eq!(r("C:\\"), None);
        assert_eq!(r("\\\\?\\C:"), None);
    }

    /// Legge le condivisioni del PC stesso: serve che ci siano quelle amministrative.
    #[test]
    #[ignore]
    fn lists_shares_of_localhost() {
        let shares = list_shares("localhost").unwrap();
        assert!(shares.iter().any(|e| e.name.ends_with('$') && e.attributes == FILE_ATTRIBUTE_HIDDEN.0), "{shares:?}");
    }
}
