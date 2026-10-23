//! Condivisioni disco di un server SMB (`\\server`), come le mostra Explorer.

use crate::error::AppError;
use crate::fs::listing::Entry;
use std::path::Path;
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, HANDLE, NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::NetManagement::{MAX_PREFERRED_LENGTH, NetApiBufferFree};
use windows::Win32::NetworkManagement::WNet::{
    NETRESOURCEW, RESOURCE_GLOBALNET, RESOURCETYPE_DISK, RESOURCEUSAGE_CONTAINER, RESOURCEUSAGE_NONE, WNetCloseEnum,
    WNetEnumResourceW, WNetOpenEnumW,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE,
};
use windows::core::{HSTRING, PWSTR};

/// Bit di tipo di `shi1_type` (`STYPE_MASK`): sopra ci sono flag come `STYPE_SPECIAL` delle condivisioni `C$`.
const STYPE_MASK: u32 = 0xFF;
/// "Indirizzo di rete non valido" (`RPC_S_INVALID_NET_ADDR`, in `Win32_System_Rpc`): la risposta di SMB per `\\wsl.localhost`.
const RPC_S_INVALID_NET_ADDR: WIN32_ERROR = WIN32_ERROR(1707);

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
            // Server non SMB (`\\wsl.localhost`, `\\wsl$`): SMB risponde "indirizzo non valido", i contenitori li dà
            // il provider di rete come in Explorer. Solo su quell'errore, per non raddoppiare l'attesa di un server irraggiungibile.
            if WIN32_ERROR(rc) == RPC_S_INVALID_NET_ADDR
                && let Ok(found) = list_containers(&unc)
                && !found.is_empty()
            {
                return Ok(found);
            }
            return Err(AppError::from_io(&std::io::Error::from_raw_os_error(rc as i32), Path::new(&unc)));
        }
        if !buf.is_null() {
            let infos = unsafe { std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize) };
            out.extend(
                infos
                    .iter()
                    .filter(|i| i.shi1_type.0 & STYPE_MASK == STYPE_DISKTREE.0)
                    .map(|i| dir_entry(String::from_utf16_lossy(unsafe { i.shi1_netname.as_wide() }))),
            );
            unsafe { NetApiBufferFree(Some(buf as _)) };
        }
        if !more {
            return Ok(out);
        }
    }
}

/// Contenitori disco di `unc` secondo i provider di rete (WNet), con il solo ultimo segmento come nome.
fn list_containers(unc: &str) -> Result<Vec<Entry>, WIN32_ERROR> {
    let mut remote: Vec<u16> = unc.encode_utf16().chain([0]).collect();
    let nr = NETRESOURCEW { dwScope: RESOURCE_GLOBALNET, dwType: RESOURCETYPE_DISK, dwUsage: RESOURCEUSAGE_CONTAINER.0, lpRemoteName: PWSTR(remote.as_mut_ptr()), ..Default::default() };
    let mut h = HANDLE::default();
    let rc = unsafe { WNetOpenEnumW(RESOURCE_GLOBALNET, RESOURCETYPE_DISK, RESOURCEUSAGE_NONE, Some(&nr), &mut h) };
    if rc != NO_ERROR {
        return Err(rc);
    }
    // u64: buffer allineato per i NETRESOURCEW; 16 KB bastano per molte voci a giro (le stringhe stanno in coda al buffer).
    let mut buf = vec![0u64; 2048];
    let mut out = Vec::new();
    let rc = loop {
        let (mut count, mut size) = (u32::MAX, (buf.len() * 8) as u32);
        let rc = unsafe { WNetEnumResourceW(h, &mut count, buf.as_mut_ptr().cast(), &mut size) };
        if rc != NO_ERROR {
            break rc;
        }
        let items = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const NETRESOURCEW, count as usize) };
        out.extend(
            items
                .iter()
                .filter_map(|r| unsafe { r.lpRemoteName.to_string() }.ok())
                .filter_map(|n| n.rsplit('\\').next().filter(|s| !s.is_empty()).map(str::to_owned))
                .map(dir_entry),
        );
    };
    let _ = unsafe { WNetCloseEnum(h) };
    if rc == ERROR_NO_MORE_ITEMS { Ok(out) } else { Err(rc) }
}

/// Una condivisione come cartella; quelle che finiscono con `$` sono nascoste.
fn dir_entry(name: String) -> Entry {
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

    /// `\\wsl.localhost` non è SMB: le distribuzioni arrivano dal provider Plan 9. Serve WSL con almeno una distribuzione.
    #[test]
    #[ignore]
    fn lists_wsl_distros() {
        for server in ["wsl.localhost", "wsl$"] {
            let distros = list_shares(server).unwrap();
            assert!(!distros.is_empty() && distros.iter().all(|e| e.is_dir && !e.name.contains('\\')), "{distros:?}");
        }
    }
}
