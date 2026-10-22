//! Cartella temporanea dell'app per i file remoti che il viewer mostra come locali (immagini, PDF, media):
//! `%TEMP%\ModernCommander\<pid>`, una sottocartella per download. Si svuota all'uscita e, per i processi non più
//! vivi (chiusi male), all'avvio successivo.

use crate::error::{AppError, ErrorCode};
use crate::paths::invalid_windows_name;
use crate::vfs::loc::Loc;
use crate::vfs::router::Vfs;
use crate::vfs::transfer;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject};

fn base() -> PathBuf {
    std::env::temp_dir().join("ModernCommander")
}

/// `%TEMP%\ModernCommander\<pid>`.
pub fn dir() -> PathBuf {
    base().join(std::process::id().to_string())
}

/// All'avvio: le cartelle dei processi non più vivi.
pub fn cleanup_stale() {
    cleanup_stale_in(&base());
}

/// All'uscita: la cartella di questo processo.
pub fn cleanup_own() {
    let _ = std::fs::remove_dir_all(dir());
}

/// Elimina in `base` solo le cartelle col nome di un pid che non è più un processo vivo; il resto non si tocca.
fn cleanup_stale_in(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for e in entries.flatten() {
        let pid = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok());
        if pid.is_some_and(|pid| !alive(pid)) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Nel dubbio (processo protetto, di un altro utente) vivo: si elimina solo ciò che è certamente morto. Un pid
/// riusato da un altro processo tiene la cartella fino a un avvio successivo.
fn alive(pid: u32) -> bool {
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(h) => {
            let running = unsafe { WaitForSingleObject(h, 0) } == WAIT_TIMEOUT;
            let _ = unsafe { CloseHandle(h) };
            running
        }
        Err(e) => e.code() != ERROR_INVALID_PARAMETER.to_hresult(),
    }
}

/// Scarica `loc` in `base/<uuid>/<nome>` e ne restituisce il percorso. `progress(byte)` dopo ogni blocco; con
/// `cancel` alzato `CANCELLED` e non resta niente, neanche la cartella.
pub fn fetch_in(vfs: &Vfs, loc: &Loc, base: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, AppError> {
    if !loc.is_remote() {
        return Err(AppError::new(ErrorCode::InvalidRequest, None));
    }
    // Il nome viene dal server: con `\`, `:` o `..` unito alla cartella porterebbe altrove (un disco, una share SMB).
    let name = loc.file_name().filter(|n| !invalid_windows_name(n)).ok_or_else(|| loc.error(ErrorCode::InvalidName))?;
    let dir = base.join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| AppError::from_io(&e, &dir))?;
    let dst = dir.join(name);
    let r = transfer::copy(vfs, loc, &Loc::Local(dst.clone()), false, &mut |_| false, &mut |n| {
        progress(n);
        !cancel.load(Ordering::Relaxed)
    });
    match r {
        Ok(()) => Ok(dst),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::vfs::loc::Loc;
    use crate::vfs::mem::{MemProvider, vfs};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn fetch_temp_lands_in_own_dir() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/d/foto.jpg", b"jpeg");
        let v = vfs(&[("m", &m)]);
        let loc = Loc::parse("sftp://m/d/foto.jpg").unwrap();
        let base = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let mut seen = 0;
        let p = fetch_in(&v, &loc, base.path(), &cancel, &mut |n| seen = n).unwrap();
        assert_eq!(p.file_name().unwrap(), "foto.jpg");
        assert_eq!(p.parent().unwrap().parent().unwrap(), base.path());
        assert_eq!((std::fs::read(&p).unwrap(), seen), (b"jpeg".to_vec(), 4));
        // Ogni download ha la sua cartella: due copie dello stesso nome non si pestano.
        let q = fetch_in(&v, &loc, base.path(), &cancel, &mut |_| {}).unwrap();
        assert_ne!(p.parent(), q.parent());
        // Annullato: né il file né la sua cartella.
        cancel.store(true, Ordering::Relaxed);
        let e = fetch_in(&v, &loc, base.path(), &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 2);
    }

    #[test]
    fn fetch_temp_refuses_names_that_leave_its_dir() {
        let m = Arc::new(MemProvider::new());
        let v = vfs(&[("m", &m)]);
        let base = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        // Un server FTP può elencare nomi con `\` o `:`: uniti alla cartella porterebbero altrove (anche su una share SMB).
        for name in [r"\x", r"C:\x", r"\\h\s\x", "..", "a:b", "x."] {
            m.add_file(&format!("/d/{name}"), b"x");
            let loc = Loc::Remote { scheme: crate::vfs::loc::Scheme::Ftp, conn: "m".into(), path: format!("/d/{name}") };
            let e = fetch_in(&v, &loc, base.path(), &cancel, &mut |_| {}).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidName, "{name}");
        }
        assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
    }

    #[test]
    fn cleanup_stale_keeps_live_pids() {
        use std::process::{Command, Stdio};
        let base = tempfile::tempdir().unwrap();
        let mk = |name: &str| {
            let d = base.path().join(name);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("f"), b"x").unwrap();
            d
        };
        // Un pid che non può esistere (oltre la tabella dei pid di Windows): quello di un processo finito potrebbe
        // essere già riusato.
        let dead = mk(&0xFFFF_FFFCu32.to_string());
        let mut running = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        let live = mk(&running.id().to_string());
        let own = mk(&std::process::id().to_string());
        let other = mk("non-un-pid");
        cleanup_stale_in(base.path());
        let _ = running.kill();
        assert!(!dead.exists());
        assert!(live.exists() && own.exists() && other.exists());
    }
}
