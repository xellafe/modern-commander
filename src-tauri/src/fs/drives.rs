//! Unità logiche con etichetta, capacità e stato. Le unità di rete si interrogano in thread
//! separati con timeout di 1 s: una condivisione irraggiungibile non deve bloccare la sidebar.

use serde::Serialize;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SetThreadErrorMode, THREAD_ERROR_MODE,
};
use windows::core::HSTRING;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Cdrom,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// Radice con barra finale, per esempio `C:\`.
    pub root: String,
    pub label: String,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
    pub ready: bool,
}

/// Etichetta, capacità totale e spazio libero per l'utente; `None` se l'unità non è pronta.
fn volume_info(root: &str) -> Option<(String, u64, u64)> {
    // Niente finestre di sistema "inserire un disco" su unità senza supporto.
    let mut old = THREAD_ERROR_MODE(0);
    let _ = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut old)) };
    let root = HSTRING::from(root);
    let mut label = [0u16; 261];
    let (mut total, mut free) = (0u64, 0u64);
    let info = unsafe { GetVolumeInformationW(&root, Some(&mut label), None, None, None, None) }
        .and_then(|_| unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) });
    let _ = unsafe { SetThreadErrorMode(old, None) };
    info.ok()?;
    let len = label.iter().position(|&c| c == 0).unwrap_or(label.len());
    Some((String::from_utf16_lossy(&label[..len]), total, free))
}

fn drive(root: String, kind: DriveKind, info: Option<(String, u64, u64)>) -> Drive {
    let ready = info.is_some();
    let (label, total, free) = info.unwrap_or_default();
    Drive { root, label, kind, total, free, ready }
}

pub fn list_drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut local = Vec::new();
    // ponytail: un thread per unità di rete per chiamata; un thread bloccato dal sistema resta in
    // vita finché l'API non risponde, ma il numero è limitato dalle lettere (max 26), accettabile.
    let mut network = Vec::new();
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i) as char);
        let kind = match unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) } {
            1 => continue, // DRIVE_NO_ROOT_DIR
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::Cdrom,
            _ => DriveKind::Other,
        };
        if kind == DriveKind::Network {
            let (tx, rx) = mpsc::channel();
            let r = root.clone();
            std::thread::spawn(move || {
                let _ = tx.send(volume_info(&r));
            });
            network.push((root, rx));
        } else {
            let info = volume_info(&root);
            local.push((root, kind, info));
        }
    }
    let mut out: Vec<Drive> = local.into_iter().map(|(r, k, i)| drive(r, k, i)).collect();
    // Scadenza condivisa: N condivisioni morte costano 1 s in totale, non N secondi.
    let deadline = Instant::now() + NETWORK_TIMEOUT;
    for (root, rx) in network {
        let info = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok().flatten();
        out.push(drive(root, DriveKind::Network, info));
    }
    out.sort_by(|a, b| a.root.cmp(&b.root));
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_include_system_drive_and_never_hang() {
        let t = std::time::Instant::now();
        let v = list_drives();
        assert!(v.iter().any(|d| d.root.eq_ignore_ascii_case(r"C:\") && d.ready && d.total > 0));
        assert!(t.elapsed().as_secs() < 3);
    }
}
