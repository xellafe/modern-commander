//! Registro delle richieste annullabili, unico per tutta l'app (dimensione, hash, scansione, archivi):
//! il frontend assegna l'id, il lavoro in corso controlla il flag e `cancel_request` lo alza.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;

/// Quanti annullamenti arrivati prima della registrazione si ricordano (i più vecchi si dimenticano).
const EARLY_MAX: usize = 64;

/// Flag di annullamento delle richieste in corso, per id; poi gli id annullati prima di registrarsi: un comando
/// asincrono registra la richiesta solo quando parte, e l'annullamento del frontend può arrivare prima. Gli id non
/// si ripetono (anche dopo un ricaricamento della pagina, vedi `nextRequestId`), quindi ricordarli non tocca altri.
#[derive(Default, Clone)]
pub struct Requests(Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>, Arc<Mutex<VecDeque<u64>>>);

/// Toglie la richiesta dal registro al drop, anche se il thread va in panic.
pub struct RequestGuard {
    requests: Requests,
    id: u64,
    flag: Arc<AtomicBool>,
}

impl Requests {
    /// Parte già annullata se `cancel(id)` è arrivato prima.
    pub fn register(&self, id: u64) -> RequestGuard {
        // Sempre prima il registro e poi gli annullamenti anticipati, qui e in `cancel`: nessun annullamento si perde.
        let mut live = self.0.lock().unwrap();
        let mut early = self.1.lock().unwrap();
        let cancelled = early.iter().position(|&e| e == id).and_then(|i| early.remove(i)).is_some();
        let flag = Arc::new(AtomicBool::new(cancelled));
        live.insert(id, flag.clone());
        RequestGuard { requests: self.clone(), id, flag }
    }

    pub fn cancel(&self, id: u64) {
        let live = self.0.lock().unwrap();
        match live.get(&id) {
            Some(f) => f.store(true, Ordering::Relaxed),
            None => {
                let mut early = self.1.lock().unwrap();
                if early.len() == EARLY_MAX {
                    early.pop_front();
                }
                early.push_back(id);
            }
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
    fn a_cancel_that_arrives_before_the_register_is_kept() {
        let r = Requests::default();
        r.cancel(5); // il comando asincrono non ha ancora registrato la richiesta
        assert!(r.register(5).flag().load(Ordering::Relaxed));
        // Consumato dalla registrazione: lo stesso id registrato di nuovo parte vivo.
        assert!(!r.register(5).flag().load(Ordering::Relaxed));
        // Memoria limitata: i più vecchi si dimenticano.
        let last = 100 + 2 * EARLY_MAX as u64;
        for id in 100..last {
            r.cancel(id);
        }
        assert!(!r.register(100).flag().load(Ordering::Relaxed));
        assert!(r.register(last - 1).flag().load(Ordering::Relaxed));
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
