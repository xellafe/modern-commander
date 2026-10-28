//! Cache degli archivi aperti: indice, archivio del crate, copia locale e cursore di ogni contenitore restano in
//! memoria finché il contenitore non cambia (dimensione e data, ricontrollate al più ogni `RECHECK`). Al più
//! `CAPACITY` archivi: esce il meno usato di recente.

use super::provider::ArchiveProvider;
use crate::error::{AppError, ErrorCode};
use crate::vfs::loc::Loc;
use crate::vfs::local::LocalProvider;
use crate::vfs::provider::{Connector, Provider, Purpose};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CAPACITY: usize = 8;
const RECHECK: Duration = Duration::from_secs(2);

/// Dimensione e data del contenitore.
type Stamp = (u64, Option<i64>);

struct Held {
    key: String,
    provider: Arc<ArchiveProvider>,
    stamp: Stamp,
    checked: Instant,
}

#[derive(Default)]
pub struct ArchiveCache {
    /// Dal meno al più usato di recente.
    held: Mutex<Vec<Held>>,
}

/// Chiave del contenitore: quella dei `Loc::Archive` che ci stanno dentro.
fn key(container: &Loc) -> String {
    Loc::archive(container.clone(), "/").volume_key()
}

fn stamp(connector: &Arc<dyn Connector>, container: &Loc) -> Result<Stamp, AppError> {
    let s = match container {
        Loc::Local(p) => LocalProvider.stat(&p.to_string_lossy())?,
        Loc::Remote { scheme, conn, path } => connector.provider(*scheme, conn, Purpose::Browse)?.stat(path)?,
        Loc::Archive { .. } => return Err(container.error(ErrorCode::InvalidRequest)),
    };
    Ok((s.size, s.modified_ms))
}

impl ArchiveCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Provider dell'archivio `container` (locale o remoto).
    pub fn provider(&self, connector: &Arc<dyn Connector>, container: &Loc) -> Result<Arc<dyn Provider>, AppError> {
        Ok(self.open(connector, container, &AtomicBool::new(false))?)
    }

    /// Come `provider`; `cancel` interrompe la lettura dell'indice (e il download della copia locale).
    pub(crate) fn open(&self, connector: &Arc<dyn Connector>, container: &Loc, cancel: &AtomicBool) -> Result<Arc<ArchiveProvider>, AppError> {
        if matches!(container, Loc::Archive { .. }) {
            return Err(container.error(ErrorCode::InvalidRequest));
        }
        let key = key(container);
        if let Some(p) = self.fresh(&key, connector, container)? {
            return Ok(p);
        }
        // La data prima di leggere: un cambiamento durante la lettura si vede al controllo successivo.
        // ponytail: due richieste insieme sullo stesso archivio lo leggono due volte; vince l'ultima.
        let stamp = stamp(connector, container)?;
        let provider = Arc::new(ArchiveProvider::open(connector, container, cancel)?);
        let mut held = self.held.lock().unwrap();
        held.retain(|h| h.key != key);
        held.push(Held { key, provider: provider.clone(), stamp, checked: Instant::now() });
        if held.len() > CAPACITY {
            held.remove(0);
        }
        Ok(provider)
    }

    /// La voce in cache se il contenitore non è cambiato; una voce non più valida esce.
    fn fresh(&self, key: &str, connector: &Arc<dyn Connector>, container: &Loc) -> Result<Option<Arc<ArchiveProvider>>, AppError> {
        let old = {
            let mut held = self.held.lock().unwrap();
            let Some(i) = held.iter().position(|h| h.key == key) else { return Ok(None) };
            let h = held.remove(i);
            let (recent, provider, stamp) = (h.checked.elapsed() < RECHECK, h.provider.clone(), h.stamp);
            held.push(h);
            if recent {
                return Ok(Some(provider));
            }
            stamp
        };
        // Fuori dal lucchetto: lo `stat` remoto può essere lento.
        let now = stamp(connector, container);
        let mut held = self.held.lock().unwrap();
        let Some(i) = held.iter().position(|h| h.key == key) else { return now.map(|_| None) };
        match now {
            Ok(s) if s == old => {
                held[i].checked = Instant::now();
                Ok(Some(held[i].provider.clone()))
            }
            r => {
                held.remove(i);
                r.map(|_| None)
            }
        }
    }

    /// Dimentica l'archivio: la prossima richiesta rilegge l'indice (dopo una modifica fatta dall'app).
    pub fn invalidate(&self, container: &Loc) {
        if matches!(container, Loc::Archive { .. }) {
            return;
        }
        let key = key(container);
        self.held.lock().unwrap().retain(|h| h.key != key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::mem::MemConnector;
    use std::collections::HashMap;
    use std::io::Write;

    #[test]
    fn least_recently_used_leaves_first() {
        let d = tempfile::tempdir().unwrap();
        let connector: Arc<dyn Connector> = Arc::new(MemConnector(HashMap::new()));
        let cache = ArchiveCache::new();
        let zips: Vec<Loc> = (0..=CAPACITY)
            .map(|i| {
                let p = d.path().join(format!("{i}.zip"));
                let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
                w.start_file("f", zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(b"x").unwrap();
                w.finish().unwrap();
                Loc::Local(p)
            })
            .collect();
        let first = cache.open(&connector, &zips[0], &AtomicBool::new(false)).unwrap();
        for z in &zips[1..CAPACITY] {
            cache.open(&connector, z, &AtomicBool::new(false)).unwrap();
        }
        // Il primo torna il più recente: esce il secondo.
        assert!(Arc::ptr_eq(&first, &cache.open(&connector, &zips[0], &AtomicBool::new(false)).unwrap()));
        cache.open(&connector, &zips[CAPACITY], &AtomicBool::new(false)).unwrap();
        let keys: Vec<String> = cache.held.lock().unwrap().iter().map(|h| h.key.clone()).collect();
        assert_eq!(keys.len(), CAPACITY);
        assert!(keys.contains(&key(&zips[0])) && !keys.contains(&key(&zips[1])));
        // Una forma diversa dello stesso file è lo stesso archivio.
        let upper = Loc::Local(d.path().join("0.ZIP"));
        assert!(Arc::ptr_eq(&first, &cache.open(&connector, &upper, &AtomicBool::new(false)).unwrap()));
    }
}
