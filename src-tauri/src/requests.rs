//! Registro delle richieste annullabili, unico per tutta l'app (dimensione, hash, scansione, archivi):
//! il frontend assegna l'id, il lavoro in corso controlla il flag e `cancel_request` lo alza.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;

/// Flag di annullamento delle richieste in corso, per id.
#[derive(Default, Clone)]
pub struct Requests(Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>);

/// Toglie la richiesta dal registro al drop, anche se il thread va in panic.
pub struct RequestGuard {
    requests: Requests,
    id: u64,
    flag: Arc<AtomicBool>,
}

impl Requests {
    pub fn register(&self, id: u64) -> RequestGuard {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap().insert(id, flag.clone());
        RequestGuard { requests: self.clone(), id, flag }
    }

    pub fn cancel(&self, id: u64) {
        if let Some(f) = self.0.lock().unwrap().get(&id) {
            f.store(true, Ordering::Relaxed);
        }
    }
}

impl RequestGuard {
    pub fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.requests.0.lock().unwrap().remove(&self.id);
    }
}

#[tauri::command]
pub fn cancel_request(requests: State<'_, Requests>, request_id: u64) {
    requests.cancel(request_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn guard_unregisters_and_cancel_sets_flag() {
        let r = Requests::default();
        let g = r.register(7);
        r.cancel(7);
        assert!(g.flag().load(Ordering::Relaxed));
        drop(g);
        r.cancel(7); // id non più registrato: nessun effetto, nessun panic
        assert!(r.0.lock().unwrap().is_empty());
    }

    #[test]
    fn cancel_sets_only_the_flag_of_that_id() {
        let r = Requests::default();
        let (a, b) = (r.register(1), r.register(2));
        r.cancel(1);
        assert!(a.flag().load(Ordering::Relaxed) && !b.flag().load(Ordering::Relaxed));
        r.cancel(99); // id sconosciuto: nessun effetto
        assert!(!b.flag().load(Ordering::Relaxed));
    }

    #[test]
    fn registry_entry_is_removed_even_on_panic() {
        let r = Requests::default();
        let r2 = r.clone();
        let res = std::panic::catch_unwind(move || {
            let _g = r2.register(7);
            panic!("simulato");
        });
        assert!(res.is_err());
        assert!(r.0.lock().unwrap().is_empty());
    }
}
