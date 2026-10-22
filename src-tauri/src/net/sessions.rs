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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Apre una sessione verso il server di `conn`. Senza password: `AUTH_REQUIRED` se il server ne chiede una.
pub trait ProviderFactory: Send + Sync {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError>;
}

/// Sessione di navigazione in cache. `used`: ha già risposto almeno una volta senza cadere (vedi `dropped`), quindi
/// una sua caduta è "vecchia" e vale la pena riaprire e riprovare.
struct Session {
    p: Arc<dyn Provider>,
    used: AtomicBool,
}

/// Connessione aperta: la password usata (anche se non ricordata) serve per le sessioni dei trasferimenti e per
/// riconnettere. Niente `Debug`: contiene la password.
/// ponytail: la password in memoria non si azzera (lo fanno solo i buffer verso il Credential Manager); un tipo
/// segreto che si azzera al `Drop` se serve.
struct Live {
    conn: Connection,
    password: Option<String>,
    /// `None` dopo una caduta: la prossima richiesta di navigazione riapre.
    browse: Option<Arc<Session>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sessione caduta, o bloccata: dopo un `TIMEOUT` lo stato del trasporto SSH può essere incoerente, quindi la
/// sessione non si riusa.
fn dropped(e: &AppError) -> bool {
    matches!(e.code, ErrorCode::ConnectionFailed | ErrorCode::Timeout)
}

fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound, None)
}

/// Stato condiviso con le sessioni di navigazione restituite, che lo usano per riaprire dopo una caduta.
/// I valori tolti dalla mappa si lasciano cadere dopo aver rilasciato il lock (`let _old = ...`): chiudere una
/// sessione può fare I/O di rete.
struct Shared {
    factories: HashMap<Scheme, Arc<dyn ProviderFactory>>,
    /// La cartella di configurazione; se non si trova, ogni comando di rete dà quell'errore.
    dir: Result<PathBuf, AppError>,
    creds: Arc<dyn Secrets>,
    live: Mutex<HashMap<String, Live>>,
}

impl Shared {
    fn dir(&self) -> Result<&Path, AppError> {
        self.dir.as_deref().map_err(Clone::clone)
    }

    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        let factory = self.factories.get(&conn.scheme).ok_or_else(|| AppError::new(ErrorCode::Unsupported, None))?;
        factory.open(conn, password)
    }

    /// Connessione e password: quelle della connessione aperta, altrimenti quelle salvate (le posizioni della
    /// hotlist si aprono così). Lo schema deve essere quello della connessione.
    fn login(&self, scheme: Scheme, id: &str) -> Result<(Connection, Option<String>), AppError> {
        let cached = lock(&self.live).get(id).map(|l| (l.conn.clone(), l.password.clone()));
        let (conn, password) = match cached {
            Some(c) => c,
            None => (connections::find(self.dir()?, id)?, self.creds.load(id)?),
        };
        if conn.scheme != scheme {
            return Err(not_found());
        }
        Ok((conn, password))
    }

    /// La sessione di navigazione in cache, o una nuova.
    /// ponytail: si apre senza lock, quindi due pannelli possono aprire insieme due sessioni (resta l'ultima) e un
    /// `disconnect`/`save`/`delete` durante l'apertura viene superato dal reinserimento; un'apertura unica per id,
    /// con attesa, se diventa un problema.
    fn browse(&self, scheme: Scheme, id: &str) -> Result<Arc<Session>, AppError> {
        if let Some(l) = lock(&self.live).get(id)
            && l.conn.scheme == scheme
            && let Some(b) = &l.browse
        {
            return Ok(b.clone());
        }
        let (conn, password) = self.login(scheme, id)?;
        let session = Arc::new(Session { p: self.open(&conn, password.as_deref())?, used: AtomicBool::new(false) });
        let _old = lock(&self.live).insert(id.to_owned(), Live { conn, password, browse: Some(session.clone()) });
        Ok(session)
    }

    /// Toglie `s` dalla cache se c'è ancora: una sessione nuova aperta nel frattempo resta.
    fn evict(&self, id: &str, s: &Arc<Session>) {
        let _old = lock(&self.live)
            .get_mut(id)
            .filter(|l| l.browse.as_ref().is_some_and(|b| Arc::ptr_eq(b, s)))
            .and_then(|l| l.browse.take());
    }
}

/// Una sessione di navigazione per connessione, riusata; ogni trasferimento ne apre una sua.
pub struct Sessions(Arc<Shared>);

impl Sessions {
    pub fn new(factories: HashMap<Scheme, Arc<dyn ProviderFactory>>) -> Self {
        Self::with(factories, crate::config::current_config_dir(), Arc::new(super::credentials::WindowsCredentials))
    }

    pub(crate) fn with(
        factories: HashMap<Scheme, Arc<dyn ProviderFactory>>,
        dir: Result<PathBuf, AppError>,
        creds: Arc<dyn Secrets>,
    ) -> Self {
        Self(Arc::new(Shared { factories, dir, creds, live: Mutex::default() }))
    }

    /// Senza password usa quella salvata; con `remember` salva quella data, solo se la connessione riesce. Se poi il
    /// salvataggio fallisce la sessione resta aperta: l'errore dice solo che la password non è stata ricordata.
    pub fn connect(&self, id: &str, password: Option<String>, remember: bool) -> Result<(), AppError> {
        let s = &self.0;
        let conn = connections::find(s.dir()?, id)?;
        let given = password.is_some();
        let password = match password {
            Some(p) => Some(p),
            None => s.creds.load(id)?,
        };
        let session = Arc::new(Session { p: s.open(&conn, password.as_deref())?, used: AtomicBool::new(false) });
        let user = conn.user.clone();
        let to_store = if remember && given { password.clone() } else { None };
        let _old = lock(&s.live).insert(id.to_owned(), Live { conn, password, browse: Some(session) });
        match to_store {
            Some(pw) => s.creds.store(id, &user, &pw),
            None => Ok(()),
        }
    }

    /// Le sessioni dei trasferimenti in corso restano finché il trasferimento le usa.
    pub fn disconnect(&self, id: &str) {
        let _old = lock(&self.0.live).remove(id);
    }

    pub fn list(&self) -> Result<Vec<ConnectionView>, AppError> {
        connections::list(self.0.dir()?, &*self.0.creds)
    }

    /// Una connessione modificata si riapre alla prossima richiesta, con i dati nuovi.
    pub fn save(&self, conn: Connection, password: Option<String>, remember: bool) -> Result<ConnectionView, AppError> {
        let view = connections::save(self.0.dir()?, &*self.0.creds, conn, password, remember)?;
        self.disconnect(&view.conn.id);
        Ok(view)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        self.disconnect(id);
        connections::delete(self.0.dir()?, &*self.0.creds, id)
    }

    pub fn forget_password(&self, id: &str) -> Result<(), AppError> {
        self.0.creds.delete(id)
    }

    /// Accetta l'impronta per host e porta della connessione: SSH per SFTP, TLS per FTPS e HTTPS.
    pub fn trust_host(&self, id: &str, fingerprint: &str) -> Result<(), AppError> {
        let dir = self.0.dir()?;
        let conn = connections::find(dir, id)?;
        let kind = match conn.scheme {
            Scheme::Sftp => "ssh",
            Scheme::Ftps | Scheme::Https => "tls",
            Scheme::Ftp | Scheme::Http => return Err(AppError::new(ErrorCode::InvalidRequest, None)),
        };
        trust::accept_in(dir, kind, &conn.host, conn.port, fingerprint)
    }

    pub fn forget_host(&self, id: &str) -> Result<(), AppError> {
        let dir = self.0.dir()?;
        let conn = connections::find(dir, id)?;
        trust::forget_in(dir, &conn.host, conn.port)
    }
}

impl Connector for Sessions {
    fn provider(&self, scheme: Scheme, id: &str, purpose: Purpose) -> Result<Arc<dyn Provider>, AppError> {
        if purpose == Purpose::Transfer {
            let (conn, password) = self.0.login(scheme, id)?;
            return self.0.open(&conn, password.as_deref());
        }
        let session = self.0.browse(scheme, id)?;
        Ok(Arc::new(Watched { shared: self.0.clone(), scheme, id: id.to_owned(), cur: Mutex::new(session) }))
    }
}

/// Sessione di navigazione che sopravvive alle cadute. Una chiamata che dà `CONNECTION_FAILED` o `TIMEOUT` toglie la
/// sessione dalla cache; se la sessione aveva già risposto (caduta "vecchia") e l'operazione è idempotente (`stat`,
/// `list`, `read_at`, `reader`), si riapre e si riprova una volta. Le altre restituiscono l'errore e la richiesta
/// successiva riconnette. Una sessione appena aperta che fallisce non si riapre: il server è giù, inutile aspettare
/// due volte. La sessione tolta si chiude su un altro thread (vedi `Drop` di `SftpProvider`).
/// ponytail: gli errori a metà degli stream di `reader`/`writer` non si guardano.
struct Watched {
    shared: Arc<Shared>,
    scheme: Scheme,
    id: String,
    cur: Mutex<Arc<Session>>,
}

impl Watched {
    fn call<T>(&self, idempotent: bool, f: impl Fn(&dyn Provider) -> Result<T, AppError>) -> Result<T, AppError> {
        let s = lock(&self.cur).clone();
        let r = self.attempt(&s, &f);
        let fell = matches!(&r, Err(e) if dropped(e));
        if !fell || !idempotent || !s.used.load(Ordering::SeqCst) {
            return r;
        }
        let fresh = self.shared.browse(self.scheme, &self.id)?;
        let _old = std::mem::replace(&mut *lock(&self.cur), fresh.clone());
        self.attempt(&fresh, &f)
    }

    fn attempt<T>(&self, s: &Arc<Session>, f: &impl Fn(&dyn Provider) -> Result<T, AppError>) -> Result<T, AppError> {
        let r = f(&*s.p);
        match &r {
            Err(e) if dropped(e) => self.shared.evict(&self.id, s),
            _ => s.used.store(true, Ordering::SeqCst),
        }
        r
    }
}

impl Provider for Watched {
    fn caps(&self) -> Caps {
        lock(&self.cur).p.caps()
    }
    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        self.call(true, |p| p.stat(path))
    }
    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        self.call(true, |p| p.list(path))
    }
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        self.call(true, |p| p.read_at(path, offset, len))
    }
    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        self.call(true, |p| p.reader(path))
    }
    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        self.call(false, |p| p.writer(path, create_new))
    }
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        self.call(false, |p| p.rename(from, to, replace))
    }
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        self.call(false, |p| p.delete(path, is_dir))
    }
    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.call(false, |p| p.mkdir(path))
    }
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        self.call(false, |p| p.set_meta(path, modified_ms, mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::connections::sample;
    use crate::net::credentials::MemSecrets;
    use crate::vfs::mem::MemProvider;

    /// Server finto che chiede la password `pw`. `offline`: le sessioni nascono cadute; `drop_all` fa cadere quelle
    /// aperte (`list` e `delete` danno `CONNECTION_FAILED`), `stall_all` le blocca (`TIMEOUT`).
    #[derive(Default)]
    struct Fake {
        opened: Mutex<Vec<Arc<MemProvider>>>,
        offline: bool,
    }

    impl Fake {
        fn opens(&self) -> usize {
            self.opened.lock().unwrap().len()
        }
        fn drop_all(&self) {
            for m in self.opened.lock().unwrap().iter() {
                m.offline.store(true, Ordering::SeqCst);
            }
        }
        fn stall_all(&self) {
            for m in self.opened.lock().unwrap().iter() {
                m.stalled.store(true, Ordering::SeqCst);
            }
        }
    }

    impl ProviderFactory for Fake {
        fn open(&self, _: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
            match password {
                None => Err(AppError::new(ErrorCode::AuthRequired, None)),
                Some("pw") => {
                    let m = Arc::new(MemProvider::new());
                    m.offline.store(self.offline, Ordering::SeqCst);
                    self.opened.lock().unwrap().push(m.clone());
                    Ok(m)
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

    // Niente `assert_eq!` sulle password: in caso di errore finirebbero nell'output dei test.
    #[test]
    fn missing_credential_asks_password_again() {
        let (_dir, creds, s, id) = setup(Arc::default());
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        // Una password rifiutata non si ricorda.
        assert_eq!(code(s.connect(&id, Some("no".into()), true)), ErrorCode::AuthFailed);
        assert!(creds.load(&id).unwrap().is_none());
        s.connect(&id, Some("pw".into()), true).unwrap();
        s.disconnect(&id);
        s.connect(&id, None, false).unwrap();
        s.disconnect(&id);
        creds.delete(&id).unwrap();
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        assert_eq!(code(s.connect("sconosciuta", None, false)), ErrorCode::NotFound);
    }

    #[test]
    fn browse_session_is_reused_and_a_fresh_failure_is_not_retried() {
        let fake = Arc::new(Fake { offline: true, ..Fake::default() });
        let (_dir, _creds, s, id) = setup(fake.clone());
        // Password non ricordata: la sessione la tiene per riaprire.
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        assert_eq!(fake.opens(), 1);
        // Ogni trasferimento ha la sua sessione.
        s.provider(Scheme::Sftp, &id, Purpose::Transfer).unwrap();
        assert_eq!(fake.opens(), 2);
        // Una sessione appena aperta che cade non si riapre subito; la richiesta successiva sì.
        assert_eq!(p.list("/").unwrap_err().code, ErrorCode::ConnectionFailed);
        assert_eq!(fake.opens(), 2);
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        assert_eq!(fake.opens(), 3);
        // Lo schema deve essere quello della connessione; uno schema senza factory non è supportato.
        assert_eq!(s.provider(Scheme::Ftp, &id, Purpose::Browse).err().unwrap().code, ErrorCode::NotFound);
        let dir = tempfile::tempdir().unwrap();
        let creds = MemSecrets::default();
        let ftp = connections::save(dir.path(), &creds, sample(Scheme::Ftp), Some("pw".into()), true).unwrap().conn.id;
        let s = Sessions::with(HashMap::new(), Ok(dir.path().to_owned()), Arc::new(creds));
        assert_eq!(code(s.connect(&ftp, None, false)), ErrorCode::Unsupported);
    }

    #[test]
    fn first_operation_after_a_drop_reconnects() {
        let fake = Arc::new(Fake::default());
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        p.list("/").unwrap();
        // La lettura dopo la caduta riapre e riprova.
        fake.drop_all();
        p.list("/").unwrap();
        assert_eq!(fake.opens(), 2);
        // Un'eliminazione no: restituisce l'errore, e l'operazione successiva riconnette.
        fake.drop_all();
        assert_eq!(p.delete("/x", false).unwrap_err().code, ErrorCode::ConnectionFailed);
        assert_eq!(fake.opens(), 2);
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap().list("/").unwrap();
        assert_eq!(fake.opens(), 3);
    }

    #[test]
    fn a_timed_out_session_is_not_reused() {
        let fake = Arc::new(Fake::default());
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        p.list("/").unwrap();
        // Come una caduta: la lettura riapre e riprova; un'eliminazione restituisce l'errore e non riusa la sessione.
        fake.stall_all();
        p.list("/").unwrap();
        assert_eq!(fake.opens(), 2);
        fake.stall_all();
        assert_eq!(p.delete("/x", false).unwrap_err().code, ErrorCode::Timeout);
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap().list("/").unwrap();
        assert_eq!(fake.opens(), 3);
    }
}
