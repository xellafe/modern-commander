//! Sessioni verso i server delle connessioni salvate: il `Connector` di `Vfs`.

use super::connections::{self, Connection, ConnectionView};
use super::credentials::Secrets;
use super::trust;
use crate::error::{AppError, ErrorCode};
use crate::vfs::loc::Scheme;
use crate::vfs::provider::{Caps, Connector, Provider, Purpose, Stat};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Apre una sessione verso il server di `conn`. Senza password: `AUTH_REQUIRED` se il server ne chiede una.
pub trait ProviderFactory: Send + Sync {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError>;
}

/// Connessione aperta: la password usata (anche se non ricordata) serve per le sessioni dei trasferimenti e per
/// riconnettere. Niente `Debug`: contiene la password.
struct Live {
    conn: Connection,
    password: Option<String>,
    /// `None` dopo una caduta: la prossima richiesta di navigazione riapre.
    browse: Option<Arc<dyn Provider>>,
}

type LiveMap = Arc<Mutex<HashMap<String, Live>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Una sessione di navigazione per connessione, riusata; ogni trasferimento ne apre una sua.
pub struct Sessions {
    factories: HashMap<Scheme, Arc<dyn ProviderFactory>>,
    /// La cartella di configurazione; se non si trova, ogni comando di rete dà quell'errore.
    dir: Result<PathBuf, AppError>,
    creds: Arc<dyn Secrets>,
    live: LiveMap,
}

impl Sessions {
    pub fn new(factories: HashMap<Scheme, Arc<dyn ProviderFactory>>) -> Self {
        Self::with(factories, crate::config::current_config_dir(), Arc::new(super::credentials::WindowsCredentials))
    }

    pub(crate) fn with(
        factories: HashMap<Scheme, Arc<dyn ProviderFactory>>,
        dir: Result<PathBuf, AppError>,
        creds: Arc<dyn Secrets>,
    ) -> Self {
        Self { factories, dir, creds, live: LiveMap::default() }
    }

    fn dir(&self) -> Result<&Path, AppError> {
        self.dir.as_deref().map_err(Clone::clone)
    }

    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        let factory = self.factories.get(&conn.scheme).ok_or_else(|| AppError::new(ErrorCode::Unsupported, None))?;
        factory.open(conn, password)
    }

    /// Senza password usa quella salvata; con `remember` salva quella data, solo se la connessione riesce.
    pub fn connect(&self, id: &str, password: Option<String>, remember: bool) -> Result<(), AppError> {
        let conn = connections::find(self.dir()?, id)?;
        let given = password.is_some();
        let password = match password {
            Some(p) => Some(p),
            None => self.creds.load(id)?,
        };
        let browse = self.open(&conn, password.as_deref())?;
        if remember && given && let Some(pw) = &password {
            self.creds.store(id, &conn.user, pw)?;
        }
        lock(&self.live).insert(id.to_owned(), Live { conn, password, browse: Some(browse) });
        Ok(())
    }

    /// Le sessioni dei trasferimenti in corso restano finché il trasferimento le usa.
    pub fn disconnect(&self, id: &str) {
        lock(&self.live).remove(id);
    }

    pub fn list(&self) -> Result<Vec<ConnectionView>, AppError> {
        connections::list(self.dir()?, &*self.creds)
    }

    /// Una connessione modificata si riapre alla prossima richiesta, con i dati nuovi.
    pub fn save(&self, conn: Connection, password: Option<String>, remember: bool) -> Result<ConnectionView, AppError> {
        let view = connections::save(self.dir()?, &*self.creds, conn, password, remember)?;
        self.disconnect(&view.conn.id);
        Ok(view)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        self.disconnect(id);
        connections::delete(self.dir()?, &*self.creds, id)
    }

    pub fn forget_password(&self, id: &str) -> Result<(), AppError> {
        self.creds.delete(id)
    }

    /// Accetta l'impronta per host e porta della connessione: SSH per SFTP, TLS per FTPS e HTTPS.
    pub fn trust_host(&self, id: &str, fingerprint: &str) -> Result<(), AppError> {
        let dir = self.dir()?;
        let conn = connections::find(dir, id)?;
        let kind = match conn.scheme {
            Scheme::Sftp => "ssh",
            Scheme::Ftps | Scheme::Https => "tls",
            Scheme::Ftp | Scheme::Http => return Err(AppError::new(ErrorCode::InvalidRequest, None)),
        };
        trust::accept_in(dir, kind, &conn.host, conn.port, fingerprint)
    }

    pub fn forget_host(&self, id: &str) -> Result<(), AppError> {
        let dir = self.dir()?;
        let conn = connections::find(dir, id)?;
        trust::forget_in(dir, &conn.host, conn.port)
    }
}

impl Connector for Sessions {
    /// Una connessione non ancora aperta si apre con la password salvata (le posizioni della hotlist).
    fn provider(&self, scheme: Scheme, id: &str, purpose: Purpose) -> Result<Arc<dyn Provider>, AppError> {
        let not_found = || AppError::new(ErrorCode::NotFound, None);
        let cached = match lock(&self.live).get(id) {
            Some(l) if l.conn.scheme != scheme => return Err(not_found()),
            Some(Live { browse: Some(b), .. }) if purpose == Purpose::Browse => return Ok(self.watch(id, b.clone())),
            Some(l) => Some((l.conn.clone(), l.password.clone())),
            None => None,
        };
        let (conn, password) = match cached {
            Some(c) => c,
            None => (connections::find(self.dir()?, id)?, self.creds.load(id)?),
        };
        if conn.scheme != scheme {
            return Err(not_found());
        }
        let p = self.open(&conn, password.as_deref())?;
        if purpose == Purpose::Transfer {
            return Ok(p);
        }
        lock(&self.live).insert(id.to_owned(), Live { conn, password, browse: Some(p.clone()) });
        Ok(self.watch(id, p))
    }
}

impl Sessions {
    fn watch(&self, id: &str, inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
        Arc::new(Watched { id: id.to_owned(), inner, live: self.live.clone() })
    }
}

/// Sessione di navigazione che, quando una chiamata dà `CONNECTION_FAILED`, si toglie dalla cache: la richiesta
/// successiva riconnette. Se nel frattempo c'è già una sessione nuova, resta quella.
/// ponytail: l'operazione che scopre la caduta fallisce (nessun nuovo tentativo automatico) e gli errori a metà
/// degli stream di `reader`/`writer` non si guardano; un nuovo tentativo per le letture se la caduta si vede spesso.
struct Watched {
    id: String,
    inner: Arc<dyn Provider>,
    live: LiveMap,
}

impl Watched {
    fn check<T>(&self, r: Result<T, AppError>) -> Result<T, AppError> {
        if matches!(&r, Err(e) if e.code == ErrorCode::ConnectionFailed)
            && let Some(l) = lock(&self.live).get_mut(&self.id)
            && l.browse.as_ref().is_some_and(|b| std::ptr::addr_eq(Arc::as_ptr(b), Arc::as_ptr(&self.inner)))
        {
            l.browse = None;
        }
        r
    }
}

impl Provider for Watched {
    fn caps(&self) -> Caps {
        self.inner.caps()
    }
    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        self.check(self.inner.stat(path))
    }
    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        self.check(self.inner.list(path))
    }
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        self.check(self.inner.read_at(path, offset, len))
    }
    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        self.check(self.inner.reader(path))
    }
    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        self.check(self.inner.writer(path, create_new))
    }
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        self.check(self.inner.rename(from, to, replace))
    }
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        self.check(self.inner.delete(path, is_dir))
    }
    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.check(self.inner.mkdir(path))
    }
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        self.check(self.inner.set_meta(path, modified_ms, mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::connections::sample;
    use crate::net::credentials::MemSecrets;
    use crate::vfs::mem::MemProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Server finto che chiede la password `pw`; `offline` dà sessioni che non elencano (`CONNECTION_FAILED`).
    #[derive(Default)]
    struct Fake {
        opens: AtomicUsize,
        offline: bool,
    }

    impl ProviderFactory for Fake {
        fn open(&self, _: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
            match password {
                None => Err(AppError::new(ErrorCode::AuthRequired, None)),
                Some("pw") => {
                    self.opens.fetch_add(1, Ordering::SeqCst);
                    let mut m = MemProvider::new();
                    m.offline = self.offline;
                    Ok(Arc::new(m))
                }
                Some(_) => Err(AppError::new(ErrorCode::AuthFailed, None)),
            }
        }
    }

    fn setup(fake: Arc<Fake>) -> (tempfile::TempDir, Arc<MemSecrets>, Sessions, String) {
        let dir = tempfile::tempdir().unwrap();
        let creds = Arc::new(MemSecrets::default());
        let id = connections::save(dir.path(), &*creds, sample(Scheme::Sftp), None, false).unwrap().conn.id;
        let factories = HashMap::from([(Scheme::Sftp, fake as Arc<dyn ProviderFactory>)]);
        let s = Sessions::with(factories, Ok(dir.path().to_owned()), creds.clone());
        (dir, creds, s, id)
    }

    fn code(r: Result<(), AppError>) -> ErrorCode {
        r.unwrap_err().code
    }

    #[test]
    fn missing_credential_asks_password_again() {
        let (_dir, creds, s, id) = setup(Arc::default());
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        // Una password rifiutata non si ricorda.
        assert_eq!(code(s.connect(&id, Some("no".into()), true)), ErrorCode::AuthFailed);
        assert_eq!(creds.load(&id).unwrap(), None);
        s.connect(&id, Some("pw".into()), true).unwrap();
        s.disconnect(&id);
        s.connect(&id, None, false).unwrap();
        s.disconnect(&id);
        creds.delete(&id).unwrap();
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        assert_eq!(code(s.connect("sconosciuta", None, false)), ErrorCode::NotFound);
    }

    #[test]
    fn browse_session_is_reused_and_reopened_after_a_drop() {
        let fake = Arc::new(Fake { offline: true, ..Fake::default() });
        let (_dir, _creds, s, id) = setup(fake.clone());
        // Password non ricordata: la sessione la tiene per riaprire.
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        assert_eq!(fake.opens.load(Ordering::SeqCst), 1);
        // Ogni trasferimento ha la sua sessione.
        s.provider(Scheme::Sftp, &id, Purpose::Transfer).unwrap();
        assert_eq!(fake.opens.load(Ordering::SeqCst), 2);
        // Dopo una caduta la prossima richiesta riconnette.
        assert_eq!(p.list("/").unwrap_err().code, ErrorCode::ConnectionFailed);
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        assert_eq!(fake.opens.load(Ordering::SeqCst), 3);
        // Lo schema deve essere quello della connessione; uno schema senza factory non è supportato.
        assert_eq!(s.provider(Scheme::Ftp, &id, Purpose::Browse).err().unwrap().code, ErrorCode::NotFound);
        let dir = tempfile::tempdir().unwrap();
        let creds = MemSecrets::default();
        let ftp = connections::save(dir.path(), &creds, sample(Scheme::Ftp), Some("pw".into()), true).unwrap().conn.id;
        let s = Sessions::with(HashMap::new(), Ok(dir.path().to_owned()), Arc::new(creds));
        assert_eq!(code(s.connect(&ftp, None, false)), ErrorCode::Unsupported);
    }
}
