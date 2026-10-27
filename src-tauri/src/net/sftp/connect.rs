//! Connessione SSH con `russh`: algoritmi, impronta dell'host con il suo tipo, autenticazione con la password e un
//! canale SFTP grezzo. Prende uno stream qualsiasi: il TCP in `open_in`, un `tokio::io::duplex` nei test.

use super::errors::{sftp_error, ssh_error};
use super::{fingerprint, rt};
use crate::error::{AppError, ErrorCode};
use crate::net::connections::Connection;
use crate::net::trust::{self, Trust};
use crate::net::{CONNECT_TIMEOUT, IO_TIMEOUT_MS};
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse as Kbd, Prompt};
use russh::keys::{Algorithm, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect, Preferred, cipher, kex, mac};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::extensions::LIMITS;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub(super) const POSIX_RENAME: &str = "posix-rename@openssh.com";
/// Blocco di lettura e scrittura: 32 KiB come in russh-sftp, o meno se il server lo limita.
const CHUNK: u32 = 32 * 1024;
/// Attesa massima della chiusura in `Drop`.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
pub(super) struct Options {
    /// Connessione, handshake, autenticazione e apertura del canale.
    pub connect_timeout: Duration,
    /// Ogni richiesta SFTP.
    pub io_timeout: Duration,
}

impl Options {
    pub(super) const DEFAULT: Options =
        Options { connect_timeout: CONNECT_TIMEOUT, io_timeout: Duration::from_millis(IO_TIMEOUT_MS as u64) };
}

/// Sessione SSH e canale SFTP. `Drop` li chiude sul runtime, senza aspettare.
pub(super) struct Link {
    pub handle: ManuallyDrop<Handle<HostKeyCheck>>,
    pub sftp: Arc<RawSftpSession>,
}

impl Drop for Link {
    fn drop(&mut self) {
        // SAFETY: il campo si prende una volta sola, qui, e dopo `drop` non si usa più.
        let handle = unsafe { ManuallyDrop::take(&mut self.handle) };
        let sftp = self.sftp.clone();
        rt::spawn(async move {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                let _ = sftp.close_session();
                let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
            })
            .await;
        });
    }
}

pub(super) struct Connected {
    pub link: Arc<Link>,
    /// Il server ha `posix-rename@openssh.com`.
    pub atomic: bool,
    /// Byte per richiesta di lettura o scrittura.
    pub chunk: u32,
}

/// Gli algoritmi di `Preferred::DEFAULT` con il tipo di chiave salvato in testa (`ssh-rsa`: le tre varianti RSA, che
/// hanno lo stesso blob) e in coda quelli datati che libssh2 accettava, per i server vecchi.
pub(super) fn preferred(saved_key_type: Option<&str>) -> Preferred {
    let d = Preferred::DEFAULT;
    let saved = |a: &Algorithm| match saved_key_type {
        Some("ssh-rsa") => matches!(a, Algorithm::Rsa { .. }),
        Some(t) => a.as_str() == t,
        None => false,
    };
    let (mut key, rest): (Vec<_>, Vec<_>) = d.key.iter().cloned().partition(saved);
    key.extend(rest);
    let mut kexes = Vec::new();
    for k in d.kex.iter() {
        kexes.push(*k);
        // Prima dei nomi-segnale (`ext-info-*`, `kex-strict-*`), che seguono.
        if *k == kex::DH_G14_SHA256 {
            kexes.extend([kex::DH_G14_SHA1, kex::DH_GEX_SHA1]);
        }
    }
    let legacy_ciphers = [cipher::AES_256_CBC, cipher::AES_192_CBC, cipher::AES_128_CBC, cipher::TRIPLE_DES_CBC];
    Preferred {
        kex: kexes.into(),
        key: key.into(),
        cipher: [&d.cipher[..], &legacy_ciphers].concat().into(),
        mac: [&d.mac[..], &[mac::HMAC_SHA1_ETM, mac::HMAC_SHA1]].concat().into(),
        ..d
    }
}

/// Verifica l'impronta dell'host in `known_hosts`. Una chiave non nota si rifiuta; il motivo (`HOST_KEY_UNKNOWN` o
/// `HOST_KEY_CHANGED` con l'impronta in `detail`, o l'errore di `known_hosts`) resta in `refused` per `connect`.
pub(super) struct HostKeyCheck {
    dir: PathBuf,
    host: String,
    port: u16,
    refused: Arc<Mutex<Option<AppError>>>,
}

impl HostKeyCheck {
    fn verify(&self, key: &PublicKeyOrCertificate) -> Result<(), AppError> {
        let key = key.public_key();
        let blob = key.to_bytes().map_err(|e| AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) })?;
        let fp = fingerprint(&blob)?;
        let code = match trust::check_in(&self.dir, "ssh", &self.host, self.port, &fp)? {
            Trust::Known => return Ok(()),
            Trust::Unknown => ErrorCode::HostKeyUnknown,
            Trust::Changed => ErrorCode::HostKeyChanged,
        };
        trust::presented(&self.host, self.port, &fp, key.algorithm().as_str());
        Err(AppError { detail: Some(fp), ..AppError::new(code, None) })
    }
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let e = match self.verify(key) {
            Ok(()) => return Ok(true),
            Err(e) => e,
        };
        *self.refused.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
        Ok(false)
    }
}

#[derive(Debug, PartialEq)]
enum Auth {
    Password,
    /// Molti server con PAM chiedono la password solo così.
    KeyboardInteractive,
}

/// Come autenticarsi con la password, dati i metodi che il server offre (nomi separati da virgole).
fn auth_plan(methods: &str, has_password: bool) -> Result<Auth, ErrorCode> {
    if !has_password {
        return Err(ErrorCode::AuthRequired);
    }
    let offers = |m: &str| methods.split(',').any(|x| x == m);
    if offers("password") {
        Ok(Auth::Password)
    } else if offers("keyboard-interactive") {
        Ok(Auth::KeyboardInteractive)
    } else {
        Err(ErrorCode::AuthFailed)
    }
}

/// Keyboard-interactive: la password va solo alla prima richiesta con una sola domanda nascosta (la password).
/// Ogni altra (codice OTP, verifica, più domande) riceve risposte vuote, quindi il server rifiuta: `AUTH_FAILED`.
fn answer(prompts: &[Prompt], answered: &mut bool, password: &str) -> Vec<String> {
    match prompts {
        [p] if !p.echo && !*answered => {
            *answered = true;
            vec![password.to_owned()]
        }
        _ => vec![String::new(); prompts.len()],
    }
}

/// `detail` fisso: mai il messaggio del server, che potrebbe ripetere quanto inviato.
fn auth_failed() -> AppError {
    AppError { detail: Some("authentication rejected".into()), ..AppError::new(ErrorCode::AuthFailed, None) }
}

/// Prima `none` (se il server accetta si è già dentro), poi la password con il metodo che il server offre.
async fn authenticate(h: &mut Handle<HostKeyCheck>, user: &str, password: Option<&str>) -> Result<(), AppError> {
    let ssh = |e: russh::Error| ssh_error(&e);
    let methods = match h.authenticate_none(user).await.map_err(ssh)? {
        AuthResult::Success => return Ok(()),
        AuthResult::Failure { remaining_methods, .. } => {
            remaining_methods.iter().map(<&str>::from).collect::<Vec<_>>().join(",")
        }
    };
    let plan = auth_plan(&methods, password.is_some()).map_err(|code| AppError::new(code, None))?;
    let pw = password.unwrap_or_default();
    let ok = match plan {
        Auth::Password => h.authenticate_password(user, pw).await.map_err(ssh)?.success(),
        Auth::KeyboardInteractive => {
            let mut answered = false;
            let mut r = h.authenticate_keyboard_interactive_start(user, None::<String>).await.map_err(ssh)?;
            loop {
                r = match r {
                    Kbd::Success => break true,
                    Kbd::Failure { .. } => break false,
                    Kbd::InfoRequest { prompts, .. } => {
                        let answers = answer(&prompts, &mut answered, pw);
                        h.authenticate_keyboard_interactive_respond(answers).await.map_err(ssh)?
                    }
                }
            }
        }
    };
    if ok { Ok(()) } else { Err(auth_failed()) }
}

/// Handshake, impronta, autenticazione e canale `sftp` entro `connect_timeout`; poi `init` (e `limits@openssh.com`,
/// se dichiarata) con il limite delle richieste di russh-sftp. Il limite fine è quello di `rt::block` di chi chiama.
pub(super) async fn connect<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    dir: &Path,
    conn: &Connection,
    password: Option<&str>,
    stream: S,
    opts: &Options,
) -> Result<Connected, AppError> {
    let saved = trust::saved_ssh_key_type_in(dir, &conn.host, conn.port)?;
    let config = Arc::new(client::Config { preferred: preferred(saved.as_deref()), ..Default::default() });
    let refused = Arc::new(Mutex::new(None));
    let check = HostKeyCheck { dir: dir.to_owned(), host: conn.host.clone(), port: conn.port, refused: refused.clone() };
    let ssh = async {
        let mut handle = client::connect_stream(config, stream, check).await.map_err(|e| {
            let refused = refused.lock().unwrap_or_else(|p| p.into_inner()).take();
            refused.unwrap_or_else(|| ssh_error(&e))
        })?;
        authenticate(&mut handle, &conn.user, password).await?;
        let mut channel = handle.channel_open_session().await.map_err(|e| ssh_error(&e))?;
        channel.request_subsystem(true, "sftp").await.map_err(|e| ssh_error(&e))?;
        // La risposta: un rifiuto non deve aspettare `init` fino a `io_timeout`.
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) | None => {
                    let detail = Some("subsystem sftp rejected".into());
                    return Err(AppError { detail, ..AppError::new(ErrorCode::ConnectionFailed, None) });
                }
                Some(_) => {}
            }
        }
        Ok::<_, AppError>((handle, channel))
    };
    let (handle, channel) = tokio::time::timeout(opts.connect_timeout, ssh)
        .await
        .map_err(|_| AppError::new(ErrorCode::Timeout, None))??;

    // russh-sftp conta in secondi interi: per eccesso, almeno 1.
    let request_timeout_secs = opts.io_timeout.as_millis().div_ceil(1000).max(1) as u64;
    let cfg = russh_sftp::client::Config { request_timeout_secs, ..Default::default() };
    let mut sftp = RawSftpSession::new_with_config(channel.into_stream(), cfg);
    let err = |e| sftp_error(e, None);
    let version = sftp.init().await.map_err(err)?;
    let atomic = version.extensions.get(POSIX_RENAME).is_some_and(|v| v == "1");
    let mut chunk = CHUNK;
    if version.extensions.contains_key(LIMITS) {
        let limits = Limits::from(sftp.limits().await.map_err(err)?);
        chunk = [limits.read_len, limits.write_len].into_iter().flatten().fold(chunk as u64, u64::min) as u32;
        sftp.set_limits(limits);
    }
    let link = Arc::new(Link { handle: ManuallyDrop::new(handle), sftp: Arc::new(sftp) });
    Ok(Connected { link, atomic, chunk })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::sftp::fake::{self, FakeAuth, FakeOptions, conn, trusted};
    use crate::net::sftp::errors::sftp_error;
    use russh::keys::{Algorithm, EcdsaCurve, HashAlg};
    use russh::{Preferred, cipher, kex, mac};
    use tokio::io::DuplexStream;

    fn open_with(dir: &Path, host: &str, stream: DuplexStream, password: Option<&str>, o: &Options) -> Result<Connected, AppError> {
        rt::block(o.connect_timeout + o.io_timeout, connect(dir, &conn(host), password, stream, o))
    }

    fn open(dir: &Path, host: &str, stream: DuplexStream, password: Option<&str>) -> Result<Connected, AppError> {
        open_with(dir, host, stream, password, &Options::DEFAULT)
    }

    fn code(r: Result<Connected, AppError>) -> Option<ErrorCode> {
        r.err().map(|e| e.code)
    }

    #[test]
    fn unknown_then_known_then_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "changed.test");
        let (fake, s) = fake::start(FakeOptions::default());
        let e = open(d, h, s, Some("pw")).err().expect("impronta sconosciuta");
        assert_eq!(e.code, ErrorCode::HostKeyUnknown);
        let fp = e.detail.unwrap();
        assert_eq!(fp, fake.fingerprint(Algorithm::Ed25519));
        trust::accept_in(d, "ssh", h, 22, &fp).unwrap();
        assert!(open(d, h, fake.stream(), Some("pw")).is_ok());

        let (other, s) = fake::start(FakeOptions::default());
        let e = open(d, h, s, Some("pw")).err().expect("impronta cambiata");
        assert_eq!(e.code, ErrorCode::HostKeyChanged);
        assert_eq!(e.detail.unwrap(), other.fingerprint(Algorithm::Ed25519));
    }

    #[test]
    fn fingerprint_matches_ssh_key() {
        let dir = tempfile::tempdir().unwrap();
        let rsa = Algorithm::Rsa { hash: None };
        let (fake, s) = fake::start(FakeOptions { keys: vec![rsa.clone()], ..Default::default() });
        let e = open(dir.path(), "fingerprint.test", s, Some("pw")).err().expect("impronta sconosciuta");
        assert_eq!(e.detail.unwrap(), fake.fingerprint(rsa));
    }

    #[test]
    fn legacy_entry_negotiates_rsa() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "legacy-entry.test");
        let (fake, s) = fake::start(FakeOptions { keys: vec![Algorithm::Ed25519, Algorithm::Rsa { hash: None }], ..Default::default() });
        // Voce scritta da libssh2: impronta RSA, nessun `key_type`.
        let fp = fake.fingerprint(Algorithm::Rsa { hash: None });
        let v = serde_json::json!([{"kind": "ssh", "host": h, "port": 22, "fingerprint": fp}]);
        crate::config::write_internal_in(d, "known_hosts", &v).unwrap();
        assert!(open(d, h, s, Some("pw")).is_ok());
    }

    #[test]
    fn saved_ed25519_negotiates_ed25519() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "saved-ed25519.test");
        let (fake, s) = fake::start(FakeOptions { keys: vec![Algorithm::Ed25519, Algorithm::Rsa { hash: None }], ..Default::default() });
        let e = open(d, h, s, Some("pw")).err().expect("impronta sconosciuta");
        assert_eq!(e.detail.as_deref(), Some(fake.fingerprint(Algorithm::Ed25519).as_str()));
        trust::accept_in(d, "ssh", h, 22, &e.detail.unwrap()).unwrap();
        assert_eq!(trust::saved_ssh_key_type_in(d, h, 22).unwrap().as_deref(), Some("ssh-ed25519"));
        assert!(open(d, h, fake.stream(), Some("pw")).is_ok());
    }

    #[test]
    fn preferred_puts_saved_type_first_and_legacy_last() {
        let d = Preferred::DEFAULT;
        let rsa = |hash| Algorithm::Rsa { hash };
        let p = preferred(Some("ssh-rsa"));
        assert_eq!(p.key[..4], [rsa(Some(HashAlg::Sha512)), rsa(Some(HashAlg::Sha256)), rsa(None), Algorithm::Ed25519]);
        assert_eq!(p.key.len(), d.key.len());
        assert_eq!(preferred(Some("ecdsa-sha2-nistp384")).key[0], Algorithm::Ecdsa { curve: EcdsaCurve::NistP384 });

        let p = preferred(None);
        assert_eq!(p.key[..], d.key[..]);
        let at = p.kex.iter().position(|k| *k == kex::DH_G14_SHA256).unwrap();
        assert_eq!(p.kex[at + 1..at + 4], [kex::DH_G14_SHA1, kex::DH_GEX_SHA1, kex::EXTENSION_SUPPORT_AS_CLIENT]);
        assert_eq!(p.kex.len(), d.kex.len() + 2);
        assert_eq!(p.cipher[..d.cipher.len()], d.cipher[..]);
        let legacy = [cipher::AES_256_CBC, cipher::AES_192_CBC, cipher::AES_128_CBC, cipher::TRIPLE_DES_CBC];
        assert_eq!(p.cipher[d.cipher.len()..], legacy);
        assert_eq!(p.mac[..d.mac.len()], d.mac[..]);
        assert_eq!(p.mac[d.mac.len()..], [mac::HMAC_SHA1_ETM, mac::HMAC_SHA1]);
    }

    #[test]
    fn legacy_only_server_connects() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "legacy-only.test");
        let (_fake, s) = trusted(d, h, FakeOptions { legacy_only: true, ..Default::default() });
        assert!(open(d, h, s, Some("pw")).is_ok());
    }

    #[test]
    fn auth_required_and_failed() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "auth.test");
        let (fake, s) = trusted(d, h, FakeOptions::default());
        assert_eq!(code(open(d, h, s, None)), Some(ErrorCode::AuthRequired));
        assert_eq!(code(open(d, h, fake.stream(), Some("sbagliata"))), Some(ErrorCode::AuthFailed));
        assert!(open(d, h, fake.stream(), Some("pw")).is_ok());

        let (_none, s) = trusted(d, h, FakeOptions { auth: FakeAuth::NoneAccepted, ..Default::default() });
        assert!(open(d, h, s, None).is_ok());
        let (_both, s) = trusted(d, h, FakeOptions { auth: FakeAuth::Both, ..Default::default() });
        assert!(open(d, h, s, Some("pw")).is_ok());
    }

    #[test]
    fn keyboard_interactive_only() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "keyboard.test");
        let (fake, s) = trusted(d, h, FakeOptions { auth: FakeAuth::Keyboard, ..Default::default() });
        assert!(open(d, h, s, Some("pw")).is_ok());
        assert_eq!(code(open(d, h, fake.stream(), Some("sbagliata"))), Some(ErrorCode::AuthFailed));
    }

    #[test]
    fn keyboard_interactive_sends_the_password_once_to_a_single_hidden_prompt() {
        let ask = |answered: &mut bool, prompts: &[(&str, bool)]| {
            let prompts: Vec<Prompt> = prompts.iter().map(|&(t, echo)| Prompt { prompt: t.into(), echo }).collect();
            answer(&prompts, answered, "pw")
        };
        let mut answered = false;
        assert_eq!(ask(&mut answered, &[("Nome:", true)]), [""]);
        assert_eq!(ask(&mut answered, &[("Password:", false), ("Codice:", false)]), ["", ""]);
        assert!(ask(&mut answered, &[]).is_empty());
        assert_eq!(ask(&mut answered, &[("Password:", false)]), ["pw"]);
        // Una seconda domanda nascosta (per esempio un codice OTP) non riceve la password.
        assert_eq!(ask(&mut answered, &[("Codice di verifica:", false)]), [""]);
    }

    #[test]
    fn auth_failure_never_leaks_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "leak.test");
        for auth in [FakeAuth::Password, FakeAuth::Keyboard] {
            let (_fake, s) = trusted(d, h, FakeOptions { auth, ..Default::default() });
            let e = open(d, h, s, Some("pässwörd:ü")).err().expect("password rifiutata");
            assert_eq!(e.code, ErrorCode::AuthFailed);
            assert!(!e.detail.as_deref().unwrap_or_default().contains("pässwörd"));
            assert!(!format!("{e:?}").contains("pässwörd"));
        }
    }

    #[test]
    fn posix_rename_detected() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "posix-rename.test");
        let (_fake, s) = trusted(d, h, FakeOptions::default());
        let c = open(d, h, s, Some("pw")).ok().unwrap();
        assert!(c.atomic);
        assert_eq!(c.chunk, 32 * 1024);
        let (_fake, s) = trusted(d, h, FakeOptions { posix_rename: false, ..Default::default() });
        assert!(!open(d, h, s, Some("pw")).ok().unwrap().atomic);
    }

    #[test]
    fn auth_without_password_is_required() {
        assert_eq!(auth_plan("publickey,password", false), Err(ErrorCode::AuthRequired));
        assert_eq!(auth_plan("publickey,password", true), Ok(Auth::Password));
        assert_eq!(auth_plan("publickey,keyboard-interactive", true), Ok(Auth::KeyboardInteractive));
        assert_eq!(auth_plan("publickey", true), Err(ErrorCode::AuthFailed));
    }

    /// Il finto che non risponde a `init`: `TIMEOUT` dal limite delle richieste di russh-sftp.
    #[test]
    fn stalled_server_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "stalled-init.test");
        let (fake, s) = trusted(d, h, FakeOptions::default());
        fake.stall();
        let o = Options { io_timeout: Duration::from_secs(1), ..Options::DEFAULT };
        assert_eq!(code(open_with(d, h, s, Some("pw"), &o)), Some(ErrorCode::Timeout));
    }

    /// Sottosistema `sftp` rifiutato: `CONNECTION_FAILED` subito, senza aspettare `init` fino a `io_timeout`.
    #[test]
    fn rejected_subsystem_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "no-subsystem.test");
        let (_fake, s) = trusted(d, h, FakeOptions { reject_subsystem: true, ..Default::default() });
        let t = std::time::Instant::now();
        let e = open(d, h, s, Some("pw")).err().expect("sottosistema rifiutato");
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::ConnectionFailed, Some("subsystem sftp rejected")));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }

    /// Canale e sessione chiusi dal finto: la richiesta successiva dà `CONNECTION_FAILED` (russh-sftp:
    /// `sender dropped` o `session closed`).
    #[test]
    fn killed_server_fails_the_next_request() {
        let dir = tempfile::tempdir().unwrap();
        let (d, h) = (dir.path(), "killed.test");
        let (fake, s) = trusted(d, h, FakeOptions::default());
        let c = open(d, h, s, Some("pw")).ok().unwrap();
        fake.kill();
        let sftp = c.link.sftp.clone();
        let e = rt::block(Duration::from_secs(5), async move { sftp.realpath("/").await.map_err(|e| sftp_error(e, None)) })
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::ConnectionFailed);
    }
}
