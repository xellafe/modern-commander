//! Dimensione ricorsiva di una cartella (Spazio / Alt+Maiusc+Invio). Visita iterativa, annullabile,
//! senza seguire i reparse point; le sottocartelle illeggibili si contano a parte.

use super::listing::walk_dir;
use crate::error::{AppError, ErrorCode};
use crate::requests::Requests;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, State};

/// Ogni quante voci si controlla l'annullamento dentro una cartella.
const CANCEL_EVERY: usize = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
    /// Sottocartelle contate, radice esclusa (giunzioni e collegamenti simbolici contano 1, senza entrarci).
    pub dirs: u64,
    /// Sottocartelle non leggibili (accesso negato, sparite durante la visita...): saltate, non fatali.
    pub skipped: u64,
}

/// Somma file e cartelle sotto `path`. Solo la radice mancante o illeggibile è un errore.
pub fn dir_size(path: &Path, cancel: &AtomicBool) -> Result<DirSize, AppError> {
    dir_size_with(path, &mut || cancel.load(Ordering::Relaxed))
}

/// `cancelled` si interroga a ogni cartella e ogni `CANCEL_EVERY` voci, anche dentro una cartella enorme.
fn dir_size_with(path: &Path, cancelled: &mut dyn FnMut() -> bool) -> Result<DirSize, AppError> {
    let cancel_err = || AppError::new(ErrorCode::Cancelled, Some(path));
    let mut total = DirSize::default();
    let mut stack = vec![path.to_path_buf()];
    let mut is_root = true;
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        if cancelled() {
            return Err(cancel_err());
        }
        let walked = walk_dir(&dir, &mut |e| {
            seen += 1;
            if seen % CANCEL_EVERY == 0 && cancelled() {
                return false;
            }
            if !e.is_dir {
                total.files += 1;
                total.bytes += e.size;
            } else {
                total.dirs += 1;
                if !e.is_symlink {
                    stack.push(dir.join(&e.name));
                }
            }
            true
        });
        match walked {
            Ok(true) => {}
            Ok(false) => return Err(cancel_err()),
            Err(e) if is_root => return Err(e),
            // Illeggibile (anche a metà lettura): quanto già sommato resta, la cartella si segnala.
            Err(_) => total.skipped += 1,
        }
        is_root = false;
    }
    Ok(total)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirSizeEvent {
    request_id: u64,
    path: String,
    result: Option<DirSize>,
    error: Option<AppError>,
}

/// Risponde subito; il risultato arriva con l'evento `dir-size`. Una richiesta per thread dedicato:
/// il frontend limita la concorrenza.
#[tauri::command]
pub fn calc_dir_size(app: AppHandle, requests: State<'_, Requests>, request_id: u64, path: String) {
    let guard = requests.register(request_id);
    tauri::async_runtime::spawn_blocking(move || {
        let r = super::local_only(&path).and_then(|p| dir_size(&p, guard.flag()));
        drop(guard); // fuori dal registro prima di emettere il risultato
        let (result, error) = match r {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        let _ = app.emit("dir-size", DirSizeEvent { request_id, path, result, error });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn write(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![b'x'; n]).unwrap();
    }

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("s1").join("a"), 1);
        write(&d.path().join("s1").join("b"), 2);
        write(&d.path().join("s2").join("c"), 3);
        d
    }

    #[test]
    fn sums_files_and_counts_dirs() {
        let d = tree();
        let r = dir_size(d.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(r, DirSize { bytes: 6, files: 3, dirs: 2, skipped: 0 });
    }

    #[test]
    fn cancel_mid_directory_stops_the_enumeration() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..3500 {
            std::fs::write(d.path().join(format!("f{i}")), b"").unwrap();
        }
        // Chiamate: 1 = inizio cartella, 2 = alla 1000a voce (qui si annulla): le altre 2500 non si leggono
        // (che la lettura si fermi davvero lo verifica `walk_dir_stops_when_visitor_says_so`).
        let mut calls = 0;
        let e = dir_size_with(d.path(), &mut || {
            calls += 1;
            calls >= 2
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(calls, 2);
    }

    #[test]
    fn empty_dir_is_zero() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(dir_size(d.path(), &AtomicBool::new(false)).unwrap(), DirSize::default());
    }

    #[test]
    fn cancelled_flag_aborts() {
        let d = tree();
        let e = dir_size(d.path(), &AtomicBool::new(true)).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }

    #[test]
    fn missing_or_file_root_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let no = AtomicBool::new(false);
        assert_eq!(dir_size(&d.path().join("gone"), &no).unwrap_err().code, ErrorCode::NotFound);
        let f = d.path().join("f.txt");
        write(&f, 1);
        assert_eq!(dir_size(&f, &no).unwrap_err().code, ErrorCode::InvalidRequest);
    }

    /// Toglie il divieto di lettura al drop, anche in caso di panic: senza, la tempdir non si cancella.
    struct Unlock(PathBuf);

    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
    }

    #[test]
    fn unreadable_subfolder_is_skipped_not_fatal() {
        let d = tree();
        let locked = d.path().join("s2");
        let _unlock = Unlock(locked.clone()); // dichiarato dopo `d`: si esegue prima della sua cancellazione
        let ok = std::process::Command::new("icacls")
            .arg(&locked)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "icacls /deny non riuscito");
        let r = dir_size(d.path(), &AtomicBool::new(false)).unwrap();
        // s2 si conta come cartella ma non si legge: il suo file (3 byte) manca.
        assert_eq!(r, DirSize { bytes: 3, files: 2, dirs: 2, skipped: 1 });
    }

    #[test]
    fn junction_counts_as_one_dir_and_is_not_followed() {
        let d = tree();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("big"), 1000);
        let link = d.path().join("link");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        let r = dir_size(d.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(r, DirSize { bytes: 6, files: 3, dirs: 3, skipped: 0 });
        // Chiesta direttamente, la radice invece si apre.
        assert_eq!(dir_size(&link, &AtomicBool::new(false)).unwrap().bytes, 1000);
    }
}
