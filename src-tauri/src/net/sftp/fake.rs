//! Server SSH/SFTP finto per i test: `russh::server` su un `tokio::io::duplex`, mai socket (i test di loopback TCP
//! falliscono con la VPN). Il sottosistema `sftp` è `russh_sftp::server` con l'handler del finto.

use super::rt;
use crate::net::connections::Connection;
use crate::vfs::loc::Scheme;
use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use russh::keys::ssh_key::private::RsaKeypair;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Response, Session};
use russh::{Channel, ChannelId, Disconnect, MethodKind, MethodSet, Preferred, cipher, kex, mac};
use russh_sftp::protocol::{StatusCode, Version};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
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
    #[allow(dead_code)] // Task 5: risposte di lettura più corte del richiesto.
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
    let shared = Arc::new(Shared { opts, stalled: AtomicBool::new(false), open: Mutex::default() });
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
}

pub(super) fn conn() -> Connection {
    Connection {
        id: "fake".into(),
        name: "fake".into(),
        scheme: Scheme::Sftp,
        host: "fake.test".into(),
        port: 22,
        user: "u".into(),
        start_dir: "/".into(),
        ftps_implicit: false,
    }
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
                russh_sftp::server::run(channel.into_stream(), Sftp { shared: self.shared.clone() }).await;
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

struct Sftp {
    shared: Arc<Shared>,
}

impl Sftp {
    /// Dopo `stall` la richiesta resta in sospeso per sempre.
    async fn gate(&self) {
        if self.shared.stalled.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
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
            v.extensions.insert("posix-rename@openssh.com".into(), "1".into());
        }
        Ok(v)
    }
}
