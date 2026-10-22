//! Watcher per tab: ogni tab osserva la propria cartella (non ricorsivo) e il frontend
//! ricarica l'elenco all'evento `fs-changed`. Le raffiche di eventi sono compresse in una
//! sola notifica, 150 ms dopo l'ultimo evento (debounce trailing) e al massimo 1 s dopo il primo.

use crate::error::{AppError, ErrorCode};
use crate::ops::exec::drive_type;
use notify::{ErrorKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_WAIT: Duration = Duration::from_secs(1);

/// Un watcher per chiave (id del tab). Eliminare il watcher chiude il canale e
/// termina il suo thread di debounce.
#[derive(Default, Clone)]
pub struct Watchers(Arc<Mutex<HashMap<String, RecommendedWatcher>>>);

impl Watchers {
    /// Osserva `path` per conto di `key`, sostituendo l'eventuale watcher precedente.
    pub fn watch(
        &self,
        key: String,
        path: PathBuf,
        on_change: impl Fn(String) + Send + 'static,
    ) -> Result<(), AppError> {
        // notify su Windows appiattisce gli errori in `Generic`: il controllo preliminare
        // dà il codice corretto (NOT_FOUND, ACCESS_DENIED...) per i casi comuni.
        std::fs::metadata(&path).map_err(|e| AppError::from_io(&e, &path))?;
        // ponytail: l'handle di ReadDirectoryChangesW impedirebbe la rimozione sicura di una
        // chiavetta (ed espellere un CD): lì niente watcher, la lista si aggiorna con Ctrl+R e al
        // ritorno del focus. Un disco USB che si presenta come fisso resta osservato: servirebbe
        // ascoltare WM_DEVICECHANGE (DBT_DEVICEQUERYREMOVE) e chiudere il watcher.
        if !watchable(drive_type(&path)) {
            self.unwatch(&key);
            return Ok(());
        }
        let (tx, rx) = mpsc::channel::<()>();
        let watcher = notify::recommended_watcher(move |_| {
            let _ = tx.send(());
        })
        .and_then(|mut w| w.watch(&path, RecursiveMode::NonRecursive).map(|()| w))
        .map_err(|e| to_app_error(e, &path))?;
        // Il thread vive finché esiste il watcher (che possiede il Sender): quando il
        // watcher viene eliminato, recv() fallisce e il thread termina.
        let k = key.clone();
        std::thread::spawn(move || debounce(&rx, DEBOUNCE, MAX_WAIT, || on_change(k.clone())));
        self.0.lock().unwrap().insert(key, watcher);
        Ok(())
    }

    pub fn unwatch(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

/// Unità rimovibili (2) e CD/DVD (5) non si osservano: vedi `Watchers::watch`.
fn watchable(drive_type: u32) -> bool {
    !matches!(drive_type, 2 | 5)
}

/// Una notifica per raffica, `quiet` dopo l'ultimo evento ma al massimo `max_wait` dopo il primo:
/// un flusso continuo (una copia lunga nella cartella) aggiorna comunque la lista. Torna quando il
/// canale si chiude.
fn debounce(rx: &mpsc::Receiver<()>, quiet: Duration, max_wait: Duration, mut notify: impl FnMut()) {
    while rx.recv().is_ok() {
        let first = Instant::now();
        loop {
            match rx.recv_timeout(quiet.min(max_wait.saturating_sub(first.elapsed()))) {
                Ok(()) if first.elapsed() < max_wait => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                _ => break notify(),
            }
        }
    }
}

fn to_app_error(e: notify::Error, path: &Path) -> AppError {
    match e.kind {
        ErrorKind::PathNotFound => AppError::new(ErrorCode::NotFound, Some(path)),
        ErrorKind::Io(ref io) => AppError::from_io(io, path),
        _ => AppError {
            detail: Some(e.to_string()),
            ..AppError::new(ErrorCode::Io, Some(path))
        },
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsChanged {
    tab_id: String,
}

#[tauri::command]
pub async fn watch_dir(
    app: AppHandle,
    watchers: State<'_, Watchers>,
    tab_id: String,
    path: String,
) -> Result<(), AppError> {
    // metadata, add_watch e spawn possono bloccarsi (UNC lenti, unità rimovibili): thread dedicato.
    let path = super::local_only(&path)?;
    let watchers = watchers.inner().clone();
    super::blocking(move || {
        watchers.watch(tab_id, path, move |tab_id| {
            let _ = app.emit("fs-changed", FsChanged { tab_id });
        })
    })
    .await?
}

#[tauri::command]
pub fn unwatch_dir(watchers: State<'_, Watchers>, tab_id: String) {
    watchers.unwatch(&tab_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn notifies_once_per_burst() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let w = Watchers::default();
        w.watch("t1".into(), d.path().into(), move |k| { tx.send(k).unwrap(); }).unwrap();
        for i in 0..20 { std::fs::write(d.path().join(format!("f{i}")), b"").unwrap(); }
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "t1");
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err()); // debounce
    }

    #[test]
    fn a_continuous_stream_still_notifies_within_max_wait() {
        let (tx, rx) = mpsc::channel();
        let feeder = std::thread::spawn(move || {
            let t = std::time::Instant::now();
            while t.elapsed() < Duration::from_millis(700) {
                tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let t = std::time::Instant::now();
        let mut at = Vec::new();
        // Il canale si chiude a fine flusso: `debounce` torna.
        debounce(&rx, Duration::from_millis(100), Duration::from_millis(200), || at.push(t.elapsed()));
        feeder.join().unwrap();
        assert!(at.len() >= 3, "{at:?}");
        assert!(at[0] < Duration::from_millis(400), "{at:?}");
    }

    #[test]
    fn removable_and_optical_drives_are_not_watched() {
        // GetDriveTypeW: 0 sconosciuto, 1 nessuna radice, 2 rimovibile, 3 fisso, 4 rete, 5 CD, 6 RAM disk.
        let watched: Vec<u32> = (0..=6).filter(|&t| watchable(t)).collect();
        assert_eq!(watched, vec![0, 1, 3, 4, 6]);
    }

    #[test]
    fn missing_path_is_not_found() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("non-esiste");
        let e = Watchers::default().watch("t".into(), gone.clone(), |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert_eq!(e.path.as_deref(), Some(gone.to_string_lossy().as_ref()));
    }

    #[test]
    fn unwatch_and_replace_stop_the_debounce_thread() {
        let d = tempfile::tempdir().unwrap();
        let w = Watchers::default();
        // Il thread di debounce possiede il callback: quando termina, l'Arc torna a 1.
        let probe = Arc::new(());
        let (p1, p2) = (probe.clone(), probe.clone());
        w.watch("t".into(), d.path().into(), move |_| { let _ = &p1; }).unwrap();
        w.watch("t".into(), d.path().into(), move |_| { let _ = &p2; }).unwrap(); // sostituisce il primo
        let wait_for = |n| {
            for _ in 0..50 {
                if Arc::strong_count(&probe) == n { return true; }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(wait_for(2), "il thread del watcher sostituito deve terminare");
        w.unwatch("t");
        assert!(wait_for(1), "il thread del watcher rimosso deve terminare");
    }
}
