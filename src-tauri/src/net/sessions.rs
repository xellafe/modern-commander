//! Sessioni verso i server delle connessioni salvate: il `Connector` di `Vfs`.

use super::connections::{self, Connection, ConnectionView};
use super::credentials::Secrets;
use super::trust;
use super::winscp::ImportCandidate;
use crate::error::{AppError, ErrorCode};
use crate::vfs::loc::Scheme;
use crate::vfs::provider::{Caps, Connector, Provider, Purpose, Stat};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

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
    /// Sessioni dei trasferimenti libere, per connessione, con l'istante in cui sono tornate.
    pool: Mutex<HashMap<String, Vec<Idle>>>,
    /// Cresce a ogni `disconnect`: una sessione prestata prima non torna nella riserva.
    epoch: AtomicU64,
    idle_ttl: Duration,
}

/// Sessione di trasferimento libera e l'istante in cui è tornata.
type Idle = (Arc<dyn Provider>, Instant);

/// Sessioni dei trasferimenti tenute libere per connessione, e per quanto.
const IDLE_MAX: usize = 2;
const IDLE_TTL: Duration = Duration::from_secs(60);

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

    /// Una sessione libera della connessione, la più recente; quelle scadute si chiudono.
    fn take_idle(&self, id: &str) -> Option<Arc<dyn Provider>> {
        let (p, _expired) = {
            let mut pool = lock(&self.pool);
            let idle = pool.get_mut(id)?;
            let expired: Vec<_> = idle.extract_if(.., |(_, t)| t.elapsed() >= self.idle_ttl).collect();
            (idle.pop().map(|(p, _)| p), expired)
        };
        p
    }

    /// Rimette `p` tra le libere, se nel frattempo la connessione non è stata chiusa; oltre `IDLE_MAX` si chiude la
    /// più vecchia.
    fn give_back(&self, id: &str, p: Arc<dyn Provider>, epoch: u64) {
        let _old = {
            let mut pool = lock(&self.pool);
            if self.epoch.load(Ordering::SeqCst) != epoch {
                vec![(p, Instant::now())]
            } else {
                let idle = pool.entry(id.to_owned()).or_default();
                idle.push((p, Instant::now()));
                let mut old: Vec<_> = idle.extract_if(.., |(_, t)| t.elapsed() >= self.idle_ttl).collect();
                let extra = idle.len().saturating_sub(IDLE_MAX);
                old.extend(idle.drain(..extra));
                old
            }
        };
    }

    /// Toglie `s` dalla cache se c'è ancora: una sessione nuova aperta nel frattempo resta.
    fn evict(&self, id: &str, s: &Arc<Session>) {
        let _old = lock(&self.live)
            .get_mut(id)
            .filter(|l| l.browse.as_ref().is_some_and(|b| Arc::ptr_eq(b, s)))
            .and_then(|l| l.browse.take());
    }
}

/// Una sessione di navigazione per connessione, riusata; i trasferimenti prendono in prestito sessioni a parte, una per
/// utilizzatore, che poi tornano tra le libere (vedi `Lease`).
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
        Self(Arc::new(Shared {
            factories,
            dir,
            creds,
            live: Mutex::default(),
            pool: Mutex::default(),
            epoch: AtomicU64::new(0),
            idle_ttl: IDLE_TTL,
        }))
    }

    /// Senza password usa quella salvata; con `remember` salva quella data, solo se la connessione riesce. Ogni errore
    /// è della connessione; `Ok(false)`: sessione aperta, ma la password non si è potuta salvare.
    pub fn connect(&self, id: &str, password: Option<String>, remember: bool) -> Result<bool, AppError> {
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
        Ok(to_store.is_none_or(|pw| s.creds.store(id, &user, &pw).is_ok()))
    }

    /// Le sessioni dei trasferimenti in corso restano finché il trasferimento le usa.
    /// Chiude anche le sessioni libere dei trasferimenti; quelle in prestito si chiudono quando tornano.
    pub fn disconnect(&self, id: &str) {
        let _old = lock(&self.0.live).remove(id);
        let _idle = {
            let mut pool = lock(&self.0.pool);
            self.0.epoch.fetch_add(1, Ordering::SeqCst);
            pool.remove(id)
        };
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

    /// Importa i siti scelti come connessioni nuove (id nuovi, anche con un nome già in uso), senza password; quelli
    /// da saltare non si salvano. Un errore interrompe: i già salvati restano.
    pub fn import(&self, items: Vec<ImportCandidate>) -> Result<Vec<ConnectionView>, AppError> {
        let dir = self.0.dir()?;
        items
            .into_iter()
            .filter_map(ImportCandidate::into_connection)
            .map(|c| connections::save(dir, &*self.0.creds, c, None, false))
            .collect()
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
            // L'epoca prima dei dati di accesso: un `save` o un `disconnect` durante l'apertura scarta la sessione nuova.
            let epoch = self.0.epoch.load(Ordering::SeqCst);
            // Lo schema si verifica anche quando c'è una sessione libera.
            let (conn, password) = self.0.login(scheme, id)?;
            let p = match self.0.take_idle(id) {
                Some(p) => p,
                None => self.0.open(&conn, password.as_deref())?,
            };
            let fell = AtomicBool::new(false);
            let lease = Lease { shared: self.0.clone(), id: id.to_owned(), epoch, p: Some(p), fell };
            return Ok(Arc::new(Lent(Arc::new(lease))));
        }
        let session = self.0.browse(scheme, id)?;
        Ok(Arc::new(Watched { shared: self.0.clone(), scheme, id: id.to_owned(), cur: Mutex::new(session) }))
    }
}

/// Prestito di una sessione di trasferimento, sempre a un solo utilizzatore. Lo tengono il provider restituito
/// (`Lent`) e i suoi stream (`Seen`): quando cadono tutti, la sessione torna tra le libere della connessione, a meno
/// che una chiamata o uno stream abbia dato `CONNECTION_FAILED` o `TIMEOUT`, o che il provider si sappia rotto (allora
/// si chiude, su un altro thread per i provider che lo richiedono). Così una coda non apre una connessione per ogni
/// operazione.
/// ponytail: una sessione chiusa dal server mentre è libera si scopre solo al primo comando del prestito successivo,
/// che dà `CONNECTION_FAILED`; un `NOOP` alla presa se diventa un problema.
struct Lease {
    shared: Arc<Shared>,
    id: String,
    epoch: u64,
    p: Option<Arc<dyn Provider>>,
    fell: AtomicBool,
}

impl Lease {
    fn p(&self) -> &dyn Provider {
        self.p.as_deref().expect("presente fino al Drop")
    }

    fn seen<T>(&self, r: Result<T, AppError>) -> Result<T, AppError> {
        if let Err(e) = &r
            && dropped(e)
        {
            self.fell.store(true, Ordering::SeqCst);
        }
        r
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(p) = self.p.take()
            && !self.fell.load(Ordering::SeqCst)
            && !p.is_broken()
        {
            self.shared.give_back(&self.id, p, self.epoch);
        }
    }
}

struct Lent(Arc<Lease>);

/// Stream di una sessione in prestito: tiene il prestito finché è aperto, e una caduta a metà lo esclude dal riuso.
struct Seen<S> {
    inner: S,
    lease: Arc<Lease>,
}

impl<S> Seen<S> {
    fn seen<T>(&self, r: std::io::Result<T>) -> std::io::Result<T> {
        if let Err(e) = &r
            && e.get_ref().and_then(|x| x.downcast_ref::<AppError>()).is_none_or(dropped)
        {
            self.lease.fell.store(true, Ordering::SeqCst);
        }
        r
    }
}

impl<S: Read> Read for Seen<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let r = self.inner.read(buf);
        self.seen(r)
    }
}

impl<S: Write> Write for Seen<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let r = self.inner.write(buf);
        self.seen(r)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let r = self.inner.flush();
        self.seen(r)
    }
}

impl Provider for Lent {
    fn caps(&self) -> Caps {
        self.0.p().caps()
    }
    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        self.0.seen(self.0.p().stat(path))
    }
    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        self.0.seen(self.0.p().list(path))
    }
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        self.0.seen(self.0.p().read_at(path, offset, len))
    }
    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let inner = self.0.seen(self.0.p().reader(path))?;
        Ok(Box::new(Seen { inner, lease: self.0.clone() }))
    }
    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let inner = self.0.seen(self.0.p().writer(path, create_new))?;
        Ok(Box::new(Seen { inner, lease: self.0.clone() }))
    }
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        self.0.seen(self.0.p().rename(from, to, replace))
    }
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        self.0.seen(self.0.p().delete(path, is_dir))
    }
    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.0.seen(self.0.p().mkdir(path))
    }
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        self.0.seen(self.0.p().set_meta(path, modified_ms, mode))
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
    use crate::net::credentials::{MemSecrets, Refuses};
    use crate::vfs::mem::MemProvider;

    /// Server finto che chiede la password `pw`. `offline`: le sessioni nascono cadute; `drop_all` fa cadere quelle
    /// aperte (`list` e `delete` danno `CONNECTION_FAILED`), `stall_all` le blocca (`TIMEOUT`).
    #[derive(Default)]
    struct Fake {
        opened: Mutex<Vec<Arc<MemProvider>>>,
        offline: bool,
        /// Gli stream di lettura cadono dopo tanti byte.
        fail_reads: Option<u64>,
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
                    let mut m = MemProvider::new();
                    m.fail_read_after = self.fail_reads;
                    let m = Arc::new(m);
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

    fn code<T: std::fmt::Debug>(r: Result<T, AppError>) -> ErrorCode {
        r.unwrap_err().code
    }

    #[test]
    fn import_saves_new_connections_without_password_and_skips_the_rest() {
        let (_dir, _creds, s, id) = setup(Arc::default());
        let sites = crate::net::winscp::from_ini(include_str!("fixtures/winscp.ini"));
        let views = s.import(sites).unwrap();
        // Dei 10 siti della fixture 3 si saltano (WebDAV, S3, senza host).
        assert_eq!(views.len(), 7);
        assert!(views.iter().all(|v| !v.has_password && !v.conn.id.is_empty() && v.conn.id != id));
        // Un nome già in uso si importa lo stesso, con un id nuovo.
        let again = s.import(crate::net::winscp::from_ini(include_str!("fixtures/winscp.ini"))).unwrap();
        assert_eq!(s.list().unwrap().len(), 1 + 7 + again.len());
        assert_ne!(views[0].conn.id, again[0].conn.id);
    }

    // Niente `assert_eq!` sulle password: in caso di errore finirebbero nell'output dei test.
    #[test]
    fn missing_credential_asks_password_again() {
        let (_dir, creds, s, id) = setup(Arc::default());
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        // Una password rifiutata non si ricorda.
        assert_eq!(code(s.connect(&id, Some("no".into()), true)), ErrorCode::AuthFailed);
        assert!(creds.load(&id).unwrap().is_none());
        assert!(s.connect(&id, Some("pw".into()), true).unwrap());
        s.disconnect(&id);
        assert!(s.connect(&id, None, false).unwrap());
        s.disconnect(&id);
        creds.delete(&id).unwrap();
        assert_eq!(code(s.connect(&id, None, false)), ErrorCode::AuthRequired);
        assert_eq!(code(s.connect("sconosciuta", None, false)), ErrorCode::NotFound);
    }

    #[test]
    fn password_that_cannot_be_stored_keeps_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let id = connections::save(dir.path(), &Refuses, sample(Scheme::Sftp), None, false).unwrap().conn.id;
        let fake = Arc::new(Fake::default());
        let factories = HashMap::from([(Scheme::Sftp, fake.clone() as Arc<dyn ProviderFactory>)]);
        let s = Sessions::with(factories, Ok(dir.path().to_owned()), Arc::new(Refuses));
        assert!(!s.connect(&id, Some("pw".into()), true).unwrap());
        s.provider(Scheme::Sftp, &id, Purpose::Browse).unwrap();
        assert_eq!(fake.opens(), 1, "la sessione aperta da connect resta");
        // Senza `remember` non si prova a salvare.
        assert!(s.connect(&id, Some("pw".into()), false).unwrap());
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

    fn transfer(s: &Sessions, id: &str) -> Arc<dyn Provider> {
        s.provider(Scheme::Sftp, id, Purpose::Transfer).unwrap()
    }

    #[test]
    fn transfer_sessions_are_lent_one_at_a_time_and_reused() {
        let fake = Arc::new(Fake::default());
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), false).unwrap();
        assert_eq!(fake.opens(), 1);
        // In sequenza: una sessione sola, riusata.
        for _ in 0..3 {
            transfer(&s, &id).list("/").unwrap();
        }
        assert_eq!(fake.opens(), 2);
        // Insieme (scrittura del temporaneo e lettura della sorgente sullo stesso server): due sessioni diverse.
        let (a, b) = (transfer(&s, &id), transfer(&s, &id));
        assert_eq!(fake.opens(), 3);
        a.mkdir("/a").unwrap();
        assert_eq!(b.stat("/a").unwrap_err().code, ErrorCode::NotFound, "b non è la sessione di a");
        drop((a, b));
        // Al massimo due libere: la terza in contemporanea si apre.
        let (_a, _b, _c) = (transfer(&s, &id), transfer(&s, &id), transfer(&s, &id));
        assert_eq!(fake.opens(), 4);
        // Lo schema si controlla anche con sessioni libere.
        assert_eq!(s.provider(Scheme::Ftp, &id, Purpose::Transfer).err().unwrap().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_fallen_transfer_session_is_not_reused() {
        let fake = Arc::new(Fake { fail_reads: Some(1), ..Fake::default() });
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = transfer(&s, &id);
        p.list("/").unwrap();
        drop(p);
        fake.drop_all();
        assert_eq!(transfer(&s, &id).list("/").unwrap_err().code, ErrorCode::ConnectionFailed);
        // La sessione caduta si è chiusa: la prossima è nuova.
        assert_eq!(fake.opens(), 2);
        let p = transfer(&s, &id);
        assert_eq!(fake.opens(), 3);
        // Anche una caduta a metà di uno stream la esclude.
        p.writer("/f", true).unwrap().write_all(b"abc").unwrap();
        let mut buf = Vec::new();
        assert!(p.reader("/f").unwrap().read_to_end(&mut buf).is_err());
        drop(p);
        transfer(&s, &id);
        assert_eq!(fake.opens(), 4);
    }

    #[test]
    fn a_session_returns_only_after_its_streams_close() {
        let fake = Arc::new(Fake::default());
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), false).unwrap();
        let p = transfer(&s, &id);
        let mut w = p.writer("/f", true).unwrap();
        w.write_all(b"abc").unwrap();
        w.flush().unwrap();
        drop(w);
        let r = p.reader("/f").unwrap();
        // Il provider cade prima dello stream: la sessione resta in prestito, il prossimo ne riceve un'altra.
        drop(p);
        let q = transfer(&s, &id);
        assert_eq!(fake.opens(), 3);
        assert_eq!(q.stat("/f").unwrap_err().code, ErrorCode::NotFound, "non è la sessione dello stream");
        drop(q);
        // Chiuso lo stream, torna libera ed è la prima a essere riusata.
        drop(r);
        assert!(transfer(&s, &id).stat("/f").is_ok());
        assert_eq!(fake.opens(), 3);
    }

    #[test]
    fn idle_transfer_sessions_expire_and_close_on_disconnect() {
        let fake = Arc::new(Fake::default());
        let (_dir, _creds, s, id) = setup(fake.clone());
        s.connect(&id, Some("pw".into()), true).unwrap();
        drop(transfer(&s, &id));
        // `disconnect` chiude le libere, e una sessione prestata prima non torna tra le libere.
        s.disconnect(&id);
        let lent = transfer(&s, &id);
        assert_eq!(fake.opens(), 3);
        s.disconnect(&id);
        drop(lent);
        drop(transfer(&s, &id));
        assert_eq!(fake.opens(), 4);
        // Scadute: si riapre.
        let mut s = s;
        Arc::get_mut(&mut s.0).unwrap().idle_ttl = Duration::ZERO;
        drop(transfer(&s, &id));
        assert_eq!(fake.opens(), 5);
    }
}
