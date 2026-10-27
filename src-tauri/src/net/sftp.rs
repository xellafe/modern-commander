//! Provider SFTP (`ssh2`, libssh2 con WinCNG): password, impronta dell'host in `known_hosts`.
//!
//! `ssh2::Session`, `Sftp` e `File` sono `Send + Sync` e si sincronizzano da soli (un mutex sulla sessione): le
//! operazioni sulla stessa sessione si mettono in fila, i trasferimenti hanno comunque una sessione loro.
//! `posix-rename@openssh.com` e l'elenco delle cartelle passano da un canale SFTP grezzo: vedi `raw`.
//!
//! Nomi remoti non rappresentabili: l'elenco li mostra con la decodifica lossy (U+FFFD), e ogni operazione su un
//! percorso con U+FFFD dà `INVALID_NAME` senza inviare nulla (il nome vero non si ricostruisce).
//! ponytail: `ssh2` su Windows cambia `\` in `/` nei percorsi, quindi un nome remoto con `\` non si raggiunge: anche
//! quei percorsi danno `INVALID_NAME` (altrimenti, per esempio, si eliminerebbe `a/b` al posto di `a\b`). Si
//! sblocca spostando tutte le operazioni sul canale grezzo.

use super::connections::Connection;
use super::{CONNECT_TIMEOUT, IO_TIMEOUT_MS, drop_in_background, tcp};
use super::sessions::ProviderFactory;
use super::trust::{self, Trust};
use raw::RawSftp;
use crate::error::{AppError, ErrorCode};
use crate::fs::hash::sha256;
use crate::vfs::provider::{Caps, Provider, Stat};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use libssh2_sys as sys;
use ssh2::{FileStat, KeyboardInteractivePrompt, OpenFlags, OpenType, Prompt, RenameFlags, Session, Sftp};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::ManuallyDrop;
use std::path::Path;
use std::sync::Arc;

mod raw;
mod rt;

/// Lo stato di una risposta SFTP; quelli generici (`SSH_FX_FAILURE`, per esempio un file che c'è già in SFTP v3)
/// danno `IO`.
pub fn map_sftp_error(code: i32) -> ErrorCode {
    match code {
        sys::LIBSSH2_FX_NO_SUCH_FILE | sys::LIBSSH2_FX_NO_SUCH_PATH => ErrorCode::NotFound,
        sys::LIBSSH2_FX_PERMISSION_DENIED | sys::LIBSSH2_FX_WRITE_PROTECT => ErrorCode::AccessDenied,
        sys::LIBSSH2_FX_FILE_ALREADY_EXISTS => ErrorCode::AlreadyExists,
        sys::LIBSSH2_FX_NO_SPACE_ON_FILESYSTEM | sys::LIBSSH2_FX_QUOTA_EXCEEDED => ErrorCode::DiskFull,
        sys::LIBSSH2_FX_NO_CONNECTION | sys::LIBSSH2_FX_CONNECTION_LOST => ErrorCode::ConnectionFailed,
        sys::LIBSSH2_FX_OP_UNSUPPORTED => ErrorCode::Unsupported,
        _ => ErrorCode::Io,
    }
}

fn map_session_error(code: i32) -> ErrorCode {
    match code {
        sys::LIBSSH2_ERROR_TIMEOUT | sys::LIBSSH2_ERROR_SOCKET_TIMEOUT => ErrorCode::Timeout,
        sys::LIBSSH2_ERROR_BANNER_RECV
        | sys::LIBSSH2_ERROR_BANNER_SEND
        | sys::LIBSSH2_ERROR_SOCKET_SEND
        | sys::LIBSSH2_ERROR_SOCKET_DISCONNECT
        | sys::LIBSSH2_ERROR_CHANNEL_CLOSED
        | sys::LIBSSH2_ERROR_CHANNEL_EOF_SENT
        | sys::LIBSSH2_ERROR_SOCKET_RECV
        | sys::LIBSSH2_ERROR_BAD_SOCKET => ErrorCode::ConnectionFailed,
        sys::LIBSSH2_ERROR_AUTHENTICATION_FAILED | sys::LIBSSH2_ERROR_PASSWORD_EXPIRED => ErrorCode::AuthFailed,
        _ => ErrorCode::Io,
    }
}

fn map_code(code: ssh2::ErrorCode) -> ErrorCode {
    match code {
        ssh2::ErrorCode::SFTP(c) => map_sftp_error(c),
        ssh2::ErrorCode::Session(c) => map_session_error(c),
    }
}

/// I messaggi di libssh2 non contengono mai la password.
fn app_error(e: &ssh2::Error, path: Option<&str>) -> AppError {
    AppError { detail: Some(e.message().to_owned()), ..AppError::new(map_code(e.code()), path.map(Path::new)) }
}

/// Errore di lettura o scrittura su un `ssh2::File`: l'`io::Error` di `ssh2` perde il codice, che si riprende
/// dall'ultimo errore della sessione.
fn file_error(sess: &Session, e: io::Error, path: &str) -> AppError {
    let code = match e.kind() {
        io::ErrorKind::TimedOut => ErrorCode::Timeout,
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        _ => ssh2::Error::last_session_error(sess).map_or(ErrorCode::Io, |l| map_code(l.code())),
    };
    AppError { detail: Some(e.to_string()), ..AppError::new(code, Some(Path::new(path))) }
}

/// Impronta come `ssh-keygen -lf`: `SHA256:` e il base64 senza padding del blob della chiave.
pub fn fingerprint(key: &[u8]) -> Result<String, AppError> {
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(sha256(key)?)))
}

#[derive(Debug, PartialEq)]
enum Auth {
    Password,
    /// Molti server con PAM chiedono la password solo così.
    KeyboardInteractive,
}

/// Come autenticarsi con la password, dati i metodi che il server offre.
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
/// Niente `Debug`: contiene la password.
struct Answer<'a> {
    password: &'a str,
    answered: bool,
}

impl KeyboardInteractivePrompt for Answer<'_> {
    fn prompt<'b>(&mut self, _: &str, _: &str, prompts: &[Prompt<'b>]) -> Vec<String> {
        match prompts {
            [p] if !p.echo && !self.answered => {
                self.answered = true;
                vec![self.password.to_owned()]
            }
            _ => Vec::new(),
        }
    }
}

/// Apre una sessione SFTP verificando l'impronta in `dir/known_hosts`.
pub(crate) fn open_in(dir: &Path, conn: &Connection, password: Option<&str>) -> Result<SftpProvider, AppError> {
    let err = |e: ssh2::Error| app_error(&e, None);
    let mut sess = Session::new().map_err(err)?;
    sess.set_tcp_stream(tcp(&conn.host, conn.port)?);
    sess.set_timeout(CONNECT_TIMEOUT.as_millis() as u32);
    sess.handshake().map_err(err)?;
    let key = sess.host_key().map(|(k, _)| k.to_vec()).ok_or_else(|| AppError::new(ErrorCode::Io, None))?;
    let fp = fingerprint(&key)?;
    let refused = match trust::check_in(dir, "ssh", &conn.host, conn.port, &fp)? {
        Trust::Known => None,
        Trust::Unknown => Some(ErrorCode::HostKeyUnknown),
        Trust::Changed => Some(ErrorCode::HostKeyChanged),
    };
    if let Some(code) = refused {
        trust::presented(&conn.host, conn.port, &fp, "ssh-rsa");
        return Err(AppError { detail: Some(fp), ..AppError::new(code, None) });
    }
    // Chiede i metodi con l'autenticazione "none": se il server la accetta si è già dentro.
    let methods = sess.auth_methods(&conn.user).map_err(err)?.to_owned();
    if !sess.authenticated() {
        let plan = auth_plan(&methods, password.is_some()).map_err(|code| AppError::new(code, None))?;
        let pw = password.unwrap_or_default();
        match plan {
            Auth::Password => sess.userauth_password(&conn.user, pw),
            Auth::KeyboardInteractive => {
                sess.userauth_keyboard_interactive(&conn.user, &mut Answer { password: pw, answered: false })
            }
        }
        .map_err(err)?;
    }
    sess.set_timeout(IO_TIMEOUT_MS);
    let sftp = sess.sftp().map_err(err)?;
    // Senza il secondo canale (server con un solo sottosistema per connessione) non si elenca in sicurezza: `ssh2`
    // va in panico sui nomi non UTF-8. La connessione fallisce con l'errore del canale.
    let raw = RawSftp::new(&sess).map_err(|code| AppError::new(code, None))?;
    // Una volta per sessione: `rename(p, p)` non cambia nulla anche se `p` esiste, e senza l'estensione libssh2
    // risponde `UNSUPPORTED` senza inviare nulla. Ogni altro esito (di solito `NOT_FOUND`) vuol dire che c'è.
    let atomic = raw.posix_rename(PROBE, PROBE) != Err(ErrorCode::Unsupported);
    let (sess, sftp, raw) = (ManuallyDrop::new(sess), ManuallyDrop::new(sftp), ManuallyDrop::new(raw));
    Ok(SftpProvider { sess, sftp, raw, atomic })
}

pub struct SftpFactory;

impl ProviderFactory for SftpFactory {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        Ok(Arc::new(open_in(&crate::config::current_config_dir()?, conn, password)?))
    }
}

/// Percorso della prova di `posix-rename` (rinominato su sé stesso).
const PROBE: &str = "/.mc-posix-rename-probe";

/// Sessione e canali si chiudono in `Drop`, su un altro thread: vedi `drop_in_background`.
pub(crate) struct SftpProvider {
    sess: ManuallyDrop<Session>,
    sftp: ManuallyDrop<Sftp>,
    raw: ManuallyDrop<RawSftp>,
    /// Il server ha `posix-rename@openssh.com`.
    atomic: bool,
}

impl Drop for SftpProvider {
    fn drop(&mut self) {
        // SAFETY: ogni campo si prende una volta sola, qui, e dopo `drop` non si usa più.
        let parts = unsafe {
            (ManuallyDrop::take(&mut self.raw), ManuallyDrop::take(&mut self.sftp), ManuallyDrop::take(&mut self.sess))
        };
        drop_in_background(parts);
    }
}

/// Lettura o scrittura di un file remoto; gli errori portano l'`AppError` dentro `io::Error::other`.
struct Stream {
    file: ssh2::File,
    sess: Session,
    path: String,
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf).map_err(|e| io::Error::other(file_error(&self.sess, e, &self.path)))
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf).map_err(|e| io::Error::other(file_error(&self.sess, e, &self.path)))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or_default().to_owned()
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// La data del server (secondi) in millisecondi; fuori scala nessuna data.
fn mtime_ms(secs: Option<u64>) -> Option<i64> {
    i64::try_from(secs?).ok()?.checked_mul(1000)
}

/// Un percorso che non si può inviare così com'è (vedi il commento del modulo) dà `INVALID_NAME`.
fn checked(path: &str) -> Result<&Path, AppError> {
    match path.contains(['\u{FFFD}', '\\']) {
        true => Err(AppError::new(ErrorCode::InvalidName, Some(Path::new(path)))),
        false => Ok(Path::new(path)),
    }
}

impl SftpProvider {
    fn err<'a>(&self, path: &'a str) -> impl Fn(ssh2::Error) -> AppError + 'a {
        move |e| app_error(&e, Some(path))
    }

    /// In SFTP v3 "esiste già" arriva come `SSH_FX_FAILURE`: si controlla se `path` c'è.
    fn exists_err<'a>(&'a self, path: &'a str) -> impl Fn(ssh2::Error) -> AppError + 'a {
        move |e| {
            let mut a = app_error(&e, Some(path));
            if a.code == ErrorCode::Io && self.sftp.lstat(Path::new(path)).is_ok() {
                a.code = ErrorCode::AlreadyExists;
            }
            a
        }
    }

    /// Da `lstat`: un collegamento si risolve con `stat`. A cartella è una cartella con `dir_link`; a file è il file
    /// a cui punta. Un collegamento rotto, o con un nome che non si può inviare, resta un file (la sua copia dà
    /// `NOT_FOUND` o `INVALID_NAME`).
    fn to_stat(&self, name: String, path: &str, l: FileStat) -> Stat {
        let (t, link) = match l.file_type().is_symlink() && checked(path).is_ok() {
            true => self.sftp.stat(Path::new(path)).map_or((l, false), |t| (t, true)),
            false => (l, false),
        };
        let is_dir = t.is_dir();
        Stat {
            name,
            is_dir,
            dir_link: link && is_dir,
            size: if is_dir { 0 } else { t.size.unwrap_or(0) },
            modified_ms: mtime_ms(t.mtime),
            mode: t.perm.map(|m| m & 0o7777),
        }
    }
}

impl Provider for SftpProvider {
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: self.atomic, precise_mtime: true, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let l = self.sftp.lstat(checked(path)?).map_err(self.err(path))?;
        Ok(self.to_stat(name_of(path), path, l))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        checked(path)?;
        let entries = self.raw.readdir(path).map_err(|code| AppError::new(code, Some(Path::new(path))))?;
        Ok(entries
            .into_iter()
            .map(|(bytes, l)| {
                let name = String::from_utf8_lossy(&bytes).into_owned();
                let full = join(path, &name);
                self.to_stat(name, &full, l)
            })
            .collect())
    }

    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let mut file = self.sftp.open(checked(path)?).map_err(self.err(path))?;
        // Il seek di libssh2 è locale: nessun pacchetto verso il server.
        file.seek(SeekFrom::Start(offset)).map_err(|e| file_error(&self.sess, e, path))?;
        let mut buf = Vec::with_capacity(len as usize);
        file.take(len as u64).read_to_end(&mut buf).map_err(|e| file_error(&self.sess, e, path))?;
        Ok(buf)
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let file = self.sftp.open(checked(path)?).map_err(self.err(path))?;
        Ok(Box::new(Stream { file, sess: Session::clone(&self.sess), path: path.to_owned() }))
    }

    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let flags = OpenFlags::WRITE | if create_new { OpenFlags::EXCLUSIVE } else { OpenFlags::TRUNCATE };
        // Solo con `create_new` un `SSH_FX_FAILURE` su un percorso esistente vuol dire "esiste già".
        let file = self.sftp.open_mode(checked(path)?, flags, 0o644, OpenType::File).map_err(|e| match create_new {
            true => self.exists_err(path)(e),
            false => app_error(&e, Some(path)),
        })?;
        Ok(Box::new(Stream { file, sess: Session::clone(&self.sess), path: path.to_owned() }))
    }

    /// Con `replace` e `posix-rename` sostituisce in un colpo solo. Altrimenti non sostituisce mai: una destinazione
    /// esistente dà `ALREADY_EXISTS` e resta intatta (la `rename` di SFTP v3 non ha flag di sovrascrittura; chi
    /// chiama, visto `atomic_replace = false`, passa dal fallback confermato).
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        let (f, t) = (checked(from)?, checked(to)?);
        if from == to {
            return Ok(());
        }
        if replace && self.atomic {
            return self.raw.posix_rename(from, to).map_err(|code| AppError::new(code, Some(f)));
        }
        if self.sftp.lstat(t).is_ok() {
            return Err(AppError::new(ErrorCode::AlreadyExists, Some(t)));
        }
        self.sftp.rename(f, t, Some(RenameFlags::empty())).map_err(self.err(from))
    }

    /// Un collegamento a cartella si toglie con `unlink`: mai `rmdir` né ricorsione nella cartella a cui punta.
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let p = checked(path)?;
        let link = is_dir && self.sftp.lstat(p).map_err(self.err(path))?.file_type().is_symlink();
        match is_dir && !link {
            true => self.sftp.rmdir(p),
            false => self.sftp.unlink(p),
        }
        .map_err(self.err(path))
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.sftp.mkdir(checked(path)?, 0o755).map_err(self.exists_err(path))
    }

    /// SFTP imposta insieme accesso e modifica: l'accesso prende la stessa data.
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        if modified_ms.is_none() && mode.is_none() {
            return Ok(());
        }
        let secs = modified_ms.map(|ms| ms.div_euclid(1000).max(0) as u64);
        let stat = FileStat { size: None, uid: None, gid: None, perm: mode, atime: secs, mtime: secs };
        self.sftp.setstat(checked(path)?, stat).map_err(self.err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::loc::Scheme;
    use base64::engine::general_purpose::STANDARD;

    #[test]
    fn sftp_status_mapping() {
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_NO_SUCH_FILE), ErrorCode::NotFound);
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_PERMISSION_DENIED), ErrorCode::AccessDenied);
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_FAILURE), ErrorCode::Io);
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_FILE_ALREADY_EXISTS), ErrorCode::AlreadyExists);
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_CONNECTION_LOST), ErrorCode::ConnectionFailed);
        assert_eq!(map_sftp_error(sys::LIBSSH2_FX_OP_UNSUPPORTED), ErrorCode::Unsupported);
        assert_eq!(map_session_error(sys::LIBSSH2_ERROR_TIMEOUT), ErrorCode::Timeout);
        assert_eq!(map_session_error(sys::LIBSSH2_ERROR_SOCKET_RECV), ErrorCode::ConnectionFailed);
        assert_eq!(map_session_error(sys::LIBSSH2_ERROR_AUTHENTICATION_FAILED), ErrorCode::AuthFailed);
    }

    #[test]
    fn unrepresentable_names_are_never_sent() {
        // Un nome Latin-1 si mostra con U+FFFD; qualunque operazione su quel percorso dà `INVALID_NAME`.
        let name = String::from_utf8_lossy(b"caf\xe9.txt").into_owned();
        assert_eq!(name, "caf\u{FFFD}.txt");
        assert_eq!(checked(&join("/d", &name)).unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(checked("/d/a\\b").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(checked("/d/caf\u{e9}.txt").unwrap(), Path::new("/d/caf\u{e9}.txt"));
    }

    /// La data del server è un numero qualsiasi: fuori scala non c'è, mai un overflow.
    #[test]
    fn server_mtime_never_overflows() {
        assert_eq!(mtime_ms(Some(1_600_000_000)), Some(1_600_000_000_000));
        assert_eq!(mtime_ms(None), None);
        assert_eq!(mtime_ms(Some(u64::MAX)), None);
        assert_eq!(mtime_ms(Some(i64::MAX as u64)), None);
    }

    #[test]
    fn auth_without_password_is_required() {
        assert_eq!(auth_plan("publickey,password", false), Err(ErrorCode::AuthRequired));
        assert_eq!(auth_plan("publickey,password", true), Ok(Auth::Password));
        assert_eq!(auth_plan("publickey,keyboard-interactive", true), Ok(Auth::KeyboardInteractive));
        assert_eq!(auth_plan("publickey", true), Err(ErrorCode::AuthFailed));
    }

    #[test]
    fn keyboard_interactive_sends_the_password_once_to_a_single_hidden_prompt() {
        let ask = |a: &mut Answer, prompts: &[(&'static str, bool)]| {
            let prompts: Vec<Prompt> = prompts.iter().map(|&(t, echo)| Prompt { text: t.into(), echo }).collect();
            a.prompt("", "", &prompts)
        };
        let mut a = Answer { password: "pw", answered: false };
        assert!(ask(&mut a, &[("Nome:", true)]).is_empty());
        assert!(ask(&mut a, &[("Password:", false), ("Codice:", false)]).is_empty());
        assert!(ask(&mut a, &[]).is_empty());
        assert!(ask(&mut a, &[("Password:", false)]) == ["pw"]);
        // Una seconda domanda nascosta (per esempio un codice OTP) non riceve la password.
        assert!(ask(&mut a, &[("Codice di verifica:", false)]).is_empty());
    }

    #[test]
    fn fingerprint_format() {
        // Chiave ed25519 di github.com e la sua impronta pubblicata in "GitHub's SSH key fingerprints"
        // (docs.github.com); `ssh-keygen -lf` sulla stessa riga dà la stessa impronta.
        let blob = STANDARD.decode("AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl").unwrap();
        assert_eq!(fingerprint(&blob).unwrap(), "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU");
    }

    /// `MC_TEST_SFTP=host:porta:utente:password:cartella_scrivibile` (la password senza `:`). Le impronte vanno in
    /// una cartella temporanea, mai nel `known_hosts` vero. Niente `assert_eq!` che possa stampare la password.
    #[test]
    #[ignore = "richiede un server SFTP (MC_TEST_SFTP)"]
    fn sftp_live() {
        let v = std::env::var("MC_TEST_SFTP").expect("MC_TEST_SFTP non impostata");
        let parts: Vec<&str> = v.splitn(5, ':').collect();
        let [host, port, user, password, dir] = parts[..] else { panic!("MC_TEST_SFTP: host:porta:utente:password:cartella") };
        let port: u16 = port.parse().expect("porta");
        let conn = Connection {
            id: "live".into(),
            name: "live".into(),
            scheme: Scheme::Sftp,
            host: host.into(),
            port,
            user: user.into(),
            start_dir: dir.into(),
            ftps_implicit: false,
        };
        let cfg = tempfile::tempdir().unwrap();
        let code = |r: Result<SftpProvider, AppError>| r.err().map(|e| e.code);

        // Impronta mai vista: errore con l'impronta, che si accetta.
        let e = open_in(cfg.path(), &conn, Some(password)).err().expect("impronta sconosciuta");
        assert_eq!(e.code, ErrorCode::HostKeyUnknown);
        let fp = e.detail.unwrap();
        assert!(fp.starts_with("SHA256:"));
        trust::accept_in(cfg.path(), "ssh", host, port, &fp).unwrap();
        assert_eq!(code(open_in(cfg.path(), &conn, None)), Some(ErrorCode::AuthRequired));
        assert_eq!(code(open_in(cfg.path(), &conn, Some("mc-password-sbagliata"))), Some(ErrorCode::AuthFailed));
        let p = open_in(cfg.path(), &conn, Some(password)).map_err(|e| e.code).unwrap();

        let base = join(dir, &format!("mc-sftp-live-{}", uuid::Uuid::new_v4().simple()));
        p.mkdir(&base).unwrap();
        assert_eq!(p.mkdir(&base).unwrap_err().code, ErrorCode::AlreadyExists);
        let a = join(&base, "a.txt");
        let b = join(&base, "b.txt");
        p.writer(&a, true).unwrap().write_all(b"0123456789").unwrap();
        assert_eq!(p.writer(&a, true).err().unwrap().code, ErrorCode::AlreadyExists);
        assert_eq!(p.read_at(&a, 2, 3).unwrap(), b"234");
        assert_eq!(p.read_at(&a, 8, 100).unwrap(), b"89");
        let mut all = Vec::new();
        p.reader(&a).unwrap().read_to_end(&mut all).unwrap();
        assert_eq!(all, b"0123456789");
        p.set_meta(&a, Some(1_000_000_000_000), Some(0o600)).unwrap();
        let s = p.stat(&a).unwrap();
        assert_eq!((s.name.as_str(), s.is_dir, s.size, s.modified_ms, s.mode), ("a.txt", false, 10, Some(1_000_000_000_000), Some(0o600)));
        let names: Vec<String> = p.list(&base).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["a.txt"]);

        // Rinomina: senza `replace` una destinazione esistente non si tocca; con `replace` la sostituisce solo
        // `posix-rename` (OpenSSH ce l'ha), altrimenti resta intatta.
        p.writer(&b, true).unwrap().write_all(b"b").unwrap();
        assert_eq!(p.rename(&a, &b, false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"b");
        let c = join(&base, "c.txt");
        p.writer(&c, true).unwrap().write_all(b"c").unwrap();
        if p.caps().atomic_replace {
            p.rename(&c, &b, true).unwrap();
            assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"c");
            assert_eq!(p.stat(&c).unwrap_err().code, ErrorCode::NotFound);
        } else {
            assert_eq!(p.rename(&c, &b, true).unwrap_err().code, ErrorCode::AlreadyExists);
            assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"b");
            p.delete(&c, false).unwrap();
        }
        p.delete(&b, false).unwrap();
        p.rename(&a, &b, false).unwrap();
        assert_eq!(p.stat(&a).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"0123456789");

        // Collegamenti: a cartella (`dir_link`, si elimina il collegamento e non la cartella) e a file (il file a cui
        // punta). L'ordine degli argomenti di `SSH_FXP_SYMLINK` è invertito in OpenSSH: vale l'ordine che riesce
        // (l'altro proverebbe a creare il collegamento sopra il bersaglio, che esiste).
        let sub = join(&base, "sub");
        p.mkdir(&sub).unwrap();
        let link = |target: &str, at: &str| {
            let (t, a) = (Path::new(target), Path::new(at));
            assert!(p.sftp.symlink(t, a).is_ok() || p.sftp.symlink(a, t).is_ok(), "symlink {at}");
        };
        let dl = join(&base, "dl");
        let fl = join(&base, "fl");
        link(&sub, &dl);
        link(&b, &fl);
        let s = p.stat(&dl).unwrap();
        assert_eq!((s.is_dir, s.dir_link), (true, true));
        let s = p.stat(&fl).unwrap();
        assert_eq!((s.is_dir, s.dir_link, s.size), (false, false, 10));
        let mut listed: Vec<(String, bool, bool)> = p.list(&base).unwrap().into_iter().map(|s| (s.name, s.is_dir, s.dir_link)).collect();
        listed.sort();
        let expected = [("b.txt", false, false), ("dl", true, true), ("fl", false, false), ("sub", true, false)];
        assert_eq!(listed, expected.map(|(n, d, l)| (n.to_owned(), d, l)));
        p.delete(&dl, true).unwrap();
        assert!(p.stat(&sub).unwrap().is_dir);
        p.delete(&fl, false).unwrap();
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"0123456789");

        p.delete(&sub, true).unwrap();
        p.delete(&b, false).unwrap();
        p.delete(&base, true).unwrap();
        assert_eq!(p.list(&base).unwrap_err().code, ErrorCode::NotFound);
    }
}
