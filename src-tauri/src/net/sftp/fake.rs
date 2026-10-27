//! Server SSH/SFTP finto per i test: `russh::server` su un `tokio::io::duplex`, mai socket (i test di loopback TCP
//! falliscono con la VPN). Il sottosistema `sftp` è `russh_sftp::server` sopra un file system in memoria.

use super::connect::{Connected, Options, connect};
use super::rt;
use crate::net::connections::Connection;
use crate::net::trust;
use crate::vfs::loc::Scheme;
use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use russh::keys::ssh_key::private::RsaKeypair;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Response, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet, Preferred, cipher, kex, mac};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, FileMode, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::DuplexStream;
use tokio::runtime::{Builder, Runtime};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum FakeAuth {
    Password,
    Keyboard,
    Both,
    /// Accetta l'autenticazione `none`.
    NoneAccepted,
}

/// Niente `Debug`: contiene la password.
pub(super) struct FakeOptions {
    pub password: String,
    pub auth: FakeAuth,
    pub keys: Vec<Algorithm>,
    pub posix_rename: bool,
    /// Risposte di lettura di al massimo 1 000 byte.
    pub short_reads: bool,
    /// Solo `diffie-hellman-group14-sha1`, `aes128-cbc`, `hmac-sha1`.
    pub legacy_only: bool,
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self {
            password: "pw".into(),
            auth: FakeAuth::Password,
            keys: vec![Algorithm::Ed25519],
            posix_rename: true,
            short_reads: false,
            legacy_only: false,
        }
    }
}

/// Runtime del finto, separato da `sftp-rt`: in debug le sue firme RSA e i DH occuperebbero i due worker di
/// `sftp-rt`, e i timer dei test (per esempio `block_times_out`) scadrebbero in ritardo.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_multi_thread().worker_threads(2).thread_name("sftp-fake").enable_all().build().expect("runtime del finto")
});

/// Generare RSA in debug è lento: una chiave da 2048 bit per processo.
static RSA: LazyLock<PrivateKey> =
    LazyLock::new(|| PrivateKey::from(RsaKeypair::random(&mut UnwrapErr(SysRng), 2048).expect("chiave RSA")));

fn host_key(alg: &Algorithm) -> PrivateKey {
    match alg {
        Algorithm::Rsa { .. } => RSA.clone(),
        a => PrivateKey::random(&mut UnwrapErr(SysRng), a.clone()).expect("chiave dell'host"),
    }
}

struct Shared {
    opts: FakeOptions,
    stalled: AtomicBool,
    /// Canali SFTP aperti, per `kill`.
    open: Mutex<Vec<(ChannelId, server::Handle)>>,
    fs: Mutex<MemFs>,
}

enum Node {
    File { data: Vec<u8>, mode: u32, mtime: u32 },
    Dir { mode: u32, mtime: u32 },
    Link { target: String },
}

/// File system del finto, comune alle sue connessioni: percorsi assoluti senza `/` finale. Esistono `/` e `/ro`, dove
/// le modifiche danno `PERMISSION_DENIED`.
struct MemFs(BTreeMap<String, Node>);

fn now() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32
}

fn norm(path: &str) -> String {
    match path.trim_end_matches('/') {
        "" => "/".into(),
        p => p.into(),
    }
}

fn parent(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((p, _)) => p,
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") }
}

fn read_only(path: &str) -> bool {
    path == "/ro" || path.starts_with("/ro/")
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

/// Due stringhe SSH (lunghezza `u32` big-endian e byte), come i dati di `posix-rename@openssh.com`.
fn two_strings(mut b: &[u8]) -> Option<(String, String)> {
    let mut next = || {
        let n = u32::from_be_bytes(b.get(..4)?.try_into().ok()?) as usize;
        let s = String::from_utf8(b.get(4..4 + n)?.to_vec()).ok()?;
        b = &b[4 + n..];
        Some(s)
    };
    Some((next()?, next()?))
}

impl MemFs {
    fn new() -> Self {
        let dir = |mode| Node::Dir { mode, mtime: now() };
        Self(BTreeMap::from([("/".into(), dir(0o755)), ("/ro".into(), dir(0o555))]))
    }

    /// Il percorso dopo i collegamenti (al massimo 8).
    fn resolve(&self, path: &str) -> Result<String, StatusCode> {
        let mut p = path.to_owned();
        for _ in 0..8 {
            match self.0.get(&p) {
                Some(Node::Link { target }) if target.starts_with('/') => p = norm(target),
                Some(Node::Link { target }) => p = join(parent(&p), target),
                Some(_) => return Ok(p),
                None => return Err(StatusCode::NoSuchFile),
            }
        }
        Err(StatusCode::Failure)
    }

    fn attrs(&self, path: &str) -> Result<FileAttributes, StatusCode> {
        let (size, perm, mtime) = match self.0.get(path).ok_or(StatusCode::NoSuchFile)? {
            Node::File { data, mode, mtime } => (data.len() as u64, FileMode::REG.bits() | mode, *mtime),
            Node::Dir { mode, mtime } => (0, FileMode::DIR.bits() | mode, *mtime),
            Node::Link { target } => (target.len() as u64, FileMode::LNK.bits() | 0o777, 0),
        };
        let t = Some(mtime);
        Ok(FileAttributes { size: Some(size), uid: Some(0), gid: Some(0), permissions: Some(perm), atime: t, mtime: t, ..Default::default() })
    }

    /// `path` non c'è e la sua cartella sì.
    fn vacant(&self, path: &str) -> Result<(), StatusCode> {
        if self.0.contains_key(path) {
            return Err(StatusCode::Failure);
        }
        match self.0.get(parent(path)) {
            Some(Node::Dir { .. }) => Ok(()),
            _ => Err(StatusCode::NoSuchFile),
        }
    }

    fn children(&self, dir: &str) -> Vec<String> {
        self.0.keys().filter(|k| *k != dir && parent(k) == dir).cloned().collect()
    }

    /// Sposta `from` e quanto contiene.
    fn move_node(&mut self, from: &str, to: &str) {
        let under = format!("{from}/");
        let keys: Vec<String> = self.0.keys().filter(|k| *k == from || k.starts_with(&under)).cloned().collect();
        for k in keys {
            let node = self.0.remove(&k).unwrap();
            self.0.insert(format!("{to}{}", &k[from.len()..]), node);
        }
    }
}

pub(super) struct FakeServer {
    shared: Arc<Shared>,
    config: Arc<server::Config>,
}

/// Avvia il finto e restituisce il capo del client della prima connessione.
pub(super) fn start(opts: FakeOptions) -> (FakeServer, DuplexStream) {
    let methods: &[MethodKind] = match opts.auth {
        FakeAuth::Password | FakeAuth::NoneAccepted => &[MethodKind::Password],
        FakeAuth::Keyboard => &[MethodKind::KeyboardInteractive],
        FakeAuth::Both => &[MethodKind::Password, MethodKind::KeyboardInteractive],
    };
    let mut config = server::Config {
        keys: opts.keys.iter().map(host_key).collect(),
        methods: MethodSet::from(methods),
        auth_rejection_time: Duration::ZERO,
        inactivity_timeout: None,
        ..Default::default()
    };
    if opts.legacy_only {
        config.preferred = Preferred {
            kex: Cow::Borrowed(&[kex::DH_G14_SHA1]),
            cipher: Cow::Borrowed(&[cipher::AES_128_CBC]),
            mac: Cow::Borrowed(&[mac::HMAC_SHA1]),
            ..Preferred::DEFAULT
        };
    }
    let shared = Arc::new(Shared { opts, stalled: AtomicBool::new(false), open: Mutex::default(), fs: Mutex::new(MemFs::new()) });
    let fake = FakeServer { shared, config: Arc::new(config) };
    let stream = fake.stream();
    (fake, stream)
}

impl FakeServer {
    /// Un'altra connessione allo stesso finto (stesse chiavi e opzioni): il capo del client.
    pub(super) fn stream(&self) -> DuplexStream {
        let (client, server_end) = tokio::io::duplex(1 << 20);
        let (config, shared) = (self.config.clone(), self.shared.clone());
        RUNTIME.spawn(async move {
            let _ = server::run_stream(config, server_end, Ssh { shared, channels: HashMap::new() }).await;
        });
        client
    }

    /// Impronta della chiave `alg` del finto, calcolata da `ssh-key` (RSA: `Algorithm::Rsa { hash: None }`).
    pub(super) fn fingerprint(&self, alg: Algorithm) -> String {
        let key = self.config.keys.iter().find(|k| k.algorithm() == alg).expect("chiave del finto");
        key.public_key().fingerprint(HashAlg::Sha256).to_string()
    }

    /// Chiude i canali SFTP aperti e le loro sessioni.
    pub(super) fn kill(&self) {
        let open = std::mem::take(&mut *self.shared.open.lock().unwrap());
        let _ = rt::block(Duration::from_secs(5), async move {
            for (id, h) in open {
                let _ = h.close(id).await;
                let _ = h.disconnect(Disconnect::ByApplication, String::new(), String::new()).await;
            }
            Ok(())
        });
    }

    /// L'handler SFTP smette di rispondere.
    pub(super) fn stall(&self) {
        self.shared.stalled.store(true, Ordering::SeqCst);
    }

    /// Crea o sostituisce il file `path` (0o644); la cartella non si controlla.
    pub(super) fn put(&self, path: &str, data: &[u8]) {
        let file = Node::File { data: data.to_vec(), mode: 0o644, mtime: now() };
        self.shared.fs.lock().unwrap().0.insert(path.into(), file);
    }

    /// Il contenuto del file `path`, se c'è.
    pub(super) fn get(&self, path: &str) -> Option<Vec<u8>> {
        match self.shared.fs.lock().unwrap().0.get(path) {
            Some(Node::File { data, .. }) => Some(data.clone()),
            _ => None,
        }
    }
}

/// `host` diverso per ogni test: `trust::presented` tiene poche voci per tutto il processo e i test girano in
/// parallelo.
pub(super) fn conn(host: &str) -> Connection {
    Connection {
        id: "fake".into(),
        name: "fake".into(),
        scheme: Scheme::Sftp,
        host: host.into(),
        port: 22,
        user: "u".into(),
        start_dir: "/".into(),
        ftps_implicit: false,
    }
}

/// Un finto con la chiave ed25519 già accettata in `dir` per `host`, come dopo il dialogo dell'impronta.
pub(super) fn trusted(dir: &Path, host: &str, opts: FakeOptions) -> (FakeServer, DuplexStream) {
    let (fake, s) = start(opts);
    let fp = fake.fingerprint(Algorithm::Ed25519);
    trust::presented(host, 22, &fp, "ssh-ed25519");
    trust::accept_in(dir, "ssh", host, 22, &fp).unwrap();
    (fake, s)
}

/// Un finto e una connessione aperta: impronta accettata, password giusta, `Options::DEFAULT`.
pub(super) fn connected(host: &str, opts: FakeOptions) -> (FakeServer, Connected) {
    let dir = tempfile::tempdir().unwrap();
    let password = opts.password.clone();
    let (fake, s) = trusted(dir.path(), host, opts);
    let o = Options::DEFAULT;
    let c = rt::block(o.connect_timeout + o.io_timeout, connect(dir.path(), &conn(host), Some(&password), s, &o));
    (fake, c.ok().expect("connessione al finto"))
}

struct Ssh {
    shared: Arc<Shared>,
    /// Canali aperti in attesa del sottosistema.
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        Ok(if self.shared.opts.auth == FakeAuth::NoneAccepted { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == self.shared.opts.password { Auth::Accept } else { Auth::reject() })
    }

    /// Una sola domanda nascosta, la password.
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        Ok(match response {
            None => Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: Cow::Owned(vec![("Password: ".into(), false)]),
            },
            Some(mut r) => match r.next() {
                Some(a) if a.as_ref() == self.shared.opts.password.as_bytes() => Auth::Accept,
                _ => Auth::reject(),
            },
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        match self.channels.remove(&id) {
            Some(channel) if name == "sftp" => {
                session.channel_success(id)?;
                self.shared.open.lock().unwrap().push((id, session.handle()));
                let sftp = Sftp { shared: self.shared.clone(), handles: HashMap::new(), next: 0 };
                russh_sftp::server::run(channel.into_stream(), sftp).await;
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

enum Open {
    File(String),
    /// Percorso e se i nomi sono già stati mandati.
    Dir(String, bool),
}

const POSIX_RENAME: &str = "posix-rename@openssh.com";

struct Sftp {
    shared: Arc<Shared>,
    /// Handle aperti su questa connessione.
    handles: HashMap<String, Open>,
    next: u32,
}

impl Sftp {
    /// Dopo `stall` la richiesta resta in sospeso per sempre.
    async fn gate(&self) {
        if self.shared.stalled.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    }

    fn fs(&self) -> MutexGuard<'_, MemFs> {
        self.shared.fs.lock().unwrap()
    }

    fn handle(&mut self, id: u32, open: Open) -> Handle {
        self.next += 1;
        self.handles.insert(self.next.to_string(), open);
        Handle { id, handle: self.next.to_string() }
    }

    fn file(&self, handle: &str) -> Result<String, StatusCode> {
        match self.handles.get(handle) {
            Some(Open::File(p)) => Ok(p.clone()),
            _ => Err(StatusCode::Failure),
        }
    }
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        self.gate().await;
        let mut v = Version::new();
        if self.shared.opts.posix_rename {
            v.extensions.insert(POSIX_RENAME.into(), "1".into());
        }
        Ok(v)
    }

    /// Come OpenSSH con SFTP v3: `CREATE | EXCLUDE` su un file che c'è dà `FAILURE`. Sotto `/ro` l'apertura riesce.
    async fn open(&mut self, id: u32, name: String, flags: OpenFlags, attrs: FileAttributes) -> Result<Handle, StatusCode> {
        self.gate().await;
        let path = {
            let mut fs = self.fs();
            let path = norm(&name);
            match fs.resolve(&path) {
                Ok(_) if flags.contains(OpenFlags::CREATE | OpenFlags::EXCLUDE) => return Err(StatusCode::Failure),
                Ok(p) => match fs.0.get_mut(&p) {
                    Some(Node::File { data, .. }) if flags.contains(OpenFlags::TRUNCATE) => data.clear(),
                    Some(Node::File { .. }) => {}
                    _ => return Err(StatusCode::Failure),
                },
                Err(StatusCode::NoSuchFile) if flags.contains(OpenFlags::CREATE) => {
                    fs.vacant(&path)?;
                    let mode = attrs.permissions.unwrap_or(0o644) & 0o7777;
                    fs.0.insert(path.clone(), Node::File { data: Vec::new(), mode, mtime: now() });
                }
                Err(e) => return Err(e),
            }
            fs.resolve(&path)?
        };
        Ok(self.handle(id, Open::File(path)))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.gate().await;
        self.handles.remove(&handle).map(|_| ok(id)).ok_or(StatusCode::Failure)
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        self.gate().await;
        let path = self.file(&handle)?;
        let max = (if self.shared.opts.short_reads { len.min(1000) } else { len }) as usize;
        let fs = self.fs();
        let Some(Node::File { data, .. }) = fs.0.get(&path) else { return Err(StatusCode::Failure) };
        let start = offset.min(data.len() as u64) as usize;
        if start == data.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data { id, data: data[start..data.len().min(start + max)].to_vec() })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, bytes: Vec<u8>) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = self.file(&handle)?;
        if read_only(&path) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        let Some(Node::File { data, mtime, .. }) = fs.0.get_mut(&path) else { return Err(StatusCode::Failure) };
        let (start, end) = (offset as usize, offset as usize + bytes.len());
        if data.len() < end {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(&bytes);
        *mtime = now();
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.gate().await;
        Ok(Attrs { id, attrs: self.fs().attrs(&norm(&path))? })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.gate().await;
        let fs = self.fs();
        Ok(Attrs { id, attrs: fs.attrs(&fs.resolve(&norm(&path))?)? })
    }

    /// Solo permessi e `mtime`.
    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = norm(&path);
        if read_only(&path) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        let p = fs.resolve(&path)?;
        let Some(Node::File { mode, mtime, .. } | Node::Dir { mode, mtime }) = fs.0.get_mut(&p) else {
            return Err(StatusCode::Failure);
        };
        if let Some(m) = attrs.permissions {
            *mode = m & 0o7777;
        }
        if let Some(t) = attrs.mtime {
            *mtime = t;
        }
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        self.gate().await;
        let path = {
            let fs = self.fs();
            let p = fs.resolve(&norm(&path))?;
            if !matches!(fs.0.get(&p), Some(Node::Dir { .. })) {
                return Err(StatusCode::Failure);
            }
            p
        };
        Ok(self.handle(id, Open::Dir(path, false)))
    }

    /// Tutti i nomi (con `.` e `..`) in una pagina, poi `EOF`.
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        self.gate().await;
        let Some(Open::Dir(path, sent)) = self.handles.get_mut(&handle) else { return Err(StatusCode::Failure) };
        if std::mem::replace(sent, true) {
            return Err(StatusCode::Eof);
        }
        let path = path.clone();
        let fs = self.fs();
        let mut files = vec![File::new(".", fs.attrs(&path)?), File::new("..", fs.attrs(parent(&path))?)];
        for child in fs.children(&path) {
            let name = child.rsplit('/').next().unwrap_or_default();
            files.push(File::new(name, fs.attrs(&child)?));
        }
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, name: String) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = norm(&name);
        if read_only(&path) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        match fs.0.get(&path) {
            None => Err(StatusCode::NoSuchFile),
            Some(Node::Dir { .. }) => Err(StatusCode::Failure),
            Some(_) => {
                fs.0.remove(&path);
                Ok(ok(id))
            }
        }
    }

    async fn mkdir(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = norm(&path);
        if read_only(&path) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        fs.vacant(&path)?;
        fs.0.insert(path, Node::Dir { mode: attrs.permissions.unwrap_or(0o755) & 0o7777, mtime: now() });
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = norm(&path);
        let mut fs = self.fs();
        match fs.0.get(&path) {
            None => Err(StatusCode::NoSuchFile),
            Some(Node::Dir { .. }) if fs.children(&path).is_empty() => {
                fs.0.remove(&path);
                Ok(ok(id))
            }
            Some(_) => Err(StatusCode::Failure),
        }
    }

    /// SFTP v3: la destinazione non deve esistere.
    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        self.gate().await;
        let (from, to) = (norm(&from), norm(&to));
        if read_only(&from) || read_only(&to) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        if !fs.0.contains_key(&from) {
            return Err(StatusCode::NoSuchFile);
        }
        fs.vacant(&to)?;
        fs.move_node(&from, &to);
        Ok(ok(id))
    }

    /// Ordine della specifica: prima il collegamento, poi la destinazione.
    async fn symlink(&mut self, id: u32, linkpath: String, targetpath: String) -> Result<Status, StatusCode> {
        self.gate().await;
        let path = norm(&linkpath);
        let mut fs = self.fs();
        fs.vacant(&path)?;
        fs.0.insert(path, Node::Link { target: targetpath });
        Ok(ok(id))
    }

    /// `posix-rename@openssh.com` (se dichiarata): sostituisce la destinazione.
    async fn extended(&mut self, id: u32, request: String, data: Vec<u8>) -> Result<Packet, StatusCode> {
        self.gate().await;
        if request != POSIX_RENAME || !self.shared.opts.posix_rename {
            return Err(StatusCode::OpUnsupported);
        }
        let (from, to) = two_strings(&data).ok_or(StatusCode::BadMessage)?;
        let (from, to) = (norm(&from), norm(&to));
        if read_only(&from) || read_only(&to) {
            return Err(StatusCode::PermissionDenied);
        }
        let mut fs = self.fs();
        if !fs.0.contains_key(&from) {
            return Err(StatusCode::NoSuchFile);
        }
        if !matches!(fs.0.get(parent(&to)), Some(Node::Dir { .. })) {
            return Err(StatusCode::NoSuchFile);
        }
        fs.0.remove(&to);
        fs.move_node(&from, &to);
        Ok(Packet::Status(ok(id)))
    }
}
