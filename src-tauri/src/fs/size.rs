//! Dimensione ricorsiva di una cartella (Spazio / Alt+Maiusc+Invio). Visita iterativa, annullabile,
//! senza seguire i reparse point; le sottocartelle illeggibili si contano a parte.

use super::listing::list_dir;
use crate::error::{AppError, ErrorCode};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    let cancelled = || AppError::new(ErrorCode::Cancelled, Some(path));
    let mut total = DirSize::default();
    let mut stack = vec![path.to_path_buf()];
    let mut is_root = true;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let entries = match list_dir(&dir) {
            Ok(v) => v,
            Err(e) if is_root => return Err(e),
            Err(_) => {
                total.skipped += 1;
                continue;
            }
        };
        is_root = false;
        for (i, e) in entries.iter().enumerate() {
            if i % CANCEL_EVERY == CANCEL_EVERY - 1 && cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
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
        }
    }
    Ok(total)
}

/// Flag di annullamento dei calcoli in corso, per id di richiesta.
#[derive(Default, Clone)]
pub struct SizeJobs(Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>);

impl SizeJobs {
    fn start(&self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap().insert(id, flag.clone());
        flag
    }

    fn finish(&self, id: u64) {
        self.0.lock().unwrap().remove(&id);
    }

    fn cancel(&self, id: u64) {
        if let Some(f) = self.0.lock().unwrap().get(&id) {
            f.store(true, Ordering::Relaxed);
        }
    }
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
pub fn calc_dir_size(app: AppHandle, jobs: State<'_, SizeJobs>, request_id: u64, path: String) {
    let jobs = jobs.inner().clone();
    let cancel = jobs.start(request_id);
    tauri::async_runtime::spawn_blocking(move || {
        let r = dir_size(&PathBuf::from(&path), &cancel);
        jobs.finish(request_id);
        let (result, error) = match r {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        let _ = app.emit("dir-size", DirSizeEvent { request_id, path, result, error });
    });
}

#[tauri::command]
pub fn cancel_dir_size(jobs: State<'_, SizeJobs>, request_id: u64) {
    jobs.cancel(request_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    #[test]
    fn jobs_cancel_sets_only_the_flag_of_that_id() {
        let j = SizeJobs::default();
        let (a, b) = (j.start(1), j.start(2));
        j.cancel(1);
        assert!(a.load(Ordering::Relaxed) && !b.load(Ordering::Relaxed));
        j.finish(1);
        j.cancel(1); // id sconosciuto: nessun effetto
        assert!(!b.load(Ordering::Relaxed));
    }
}
