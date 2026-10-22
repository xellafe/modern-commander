//! Provider SFTP (`ssh2`, libssh2 con WinCNG): password, impronta dell'host in `known_hosts`.
//!
//! `ssh2::Session`, `Sftp` e `File` sono `Send + Sync` e si sincronizzano da soli (un mutex sulla sessione): le
//! operazioni sulla stessa sessione si mettono in fila, i trasferimenti hanno comunque una sessione loro.
//! ponytail: niente `posix-rename@openssh.com`. `ssh2` 0.9 non espone l'handle SFTP di libssh2, quindi non si può
//! né sapere se il server ha l'estensione né chiamarla: `atomic_replace` è sempre `false` e la sovrascrittura passa
//! dal fallback confermato. Si aggiunge con una chiamata FFI a `libssh2_sftp_posix_rename_ex` se serve.
//! ponytail: `ssh2` su Windows fa `from_utf8(..).unwrap()` sui nomi letti dalla cartella: un nome remoto non UTF-8
//! fa andare in panico l'elenco (e con `panic = "abort"` chiude l'app). Va sostituito con un `readdir` via FFI.

use super::connections::Connection;
use super::sessions::ProviderFactory;
use super::trust::{self, Trust};
use crate::error::{AppError, ErrorCode};
use crate::fs::hash::sha256;
use crate::vfs::provider::{Caps, Provider, Stat};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use ssh2::{FileStat, KeyboardInteractivePrompt, OpenFlags, OpenType, Prompt, RenameFlags, Session, Sftp};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Attesa massima di una risposta dopo il collegamento: con la VPN caduta si ha `TIMEOUT`, non un'attesa infinita.
const IO_TIMEOUT_MS: u32 = 30_000;

// Stati SFTP (`SSH_FX_*`) ed errori di libssh2 (`LIBSSH2_ERROR_*`): `ssh2` non riesporta le costanti di `libssh2-sys`.
const FX_NO_SUCH_FILE: i32 = 2;
const FX_PERMISSION_DENIED: i32 = 3;
const FX_NO_CONNECTION: i32 = 6;
const FX_CONNECTION_LOST: i32 = 7;
const FX_NO_SUCH_PATH: i32 = 10;
const FX_FILE_ALREADY_EXISTS: i32 = 11;
const FX_WRITE_PROTECT: i32 = 12;
const FX_NO_SPACE_ON_FILESYSTEM: i32 = 14;
const FX_QUOTA_EXCEEDED: i32 = 15;
const BANNER_RECV: i32 = -2;
const BANNER_SEND: i32 = -3;
const SOCKET_SEND: i32 = -7;
const SESSION_TIMEOUT: i32 = -9;
const SOCKET_DISCONNECT: i32 = -13;
const PASSWORD_EXPIRED: i32 = -15;
const AUTHENTICATION_FAILED: i32 = -18;
const CHANNEL_CLOSED: i32 = -26;
const CHANNEL_EOF_SENT: i32 = -27;
const SOCKET_TIMEOUT: i32 = -30;
const SOCKET_RECV: i32 = -43;
const BAD_SOCKET: i32 = -45;

/// Lo stato di una risposta SFTP; quelli generici (`SSH_FX_FAILURE`, per esempio un file che c'è già in SFTP v3)
/// danno `IO`.
pub fn map_sftp_error(code: i32) -> ErrorCode {
    match code {
        FX_NO_SUCH_FILE | FX_NO_SUCH_PATH => ErrorCode::NotFound,
        FX_PERMISSION_DENIED | FX_WRITE_PROTECT => ErrorCode::AccessDenied,
        FX_FILE_ALREADY_EXISTS => ErrorCode::AlreadyExists,
        FX_NO_SPACE_ON_FILESYSTEM | FX_QUOTA_EXCEEDED => ErrorCode::DiskFull,
        FX_NO_CONNECTION | FX_CONNECTION_LOST => ErrorCode::ConnectionFailed,
        _ => ErrorCode::Io,
    }
}

fn map_session_error(code: i32) -> ErrorCode {
    match code {
        SESSION_TIMEOUT | SOCKET_TIMEOUT => ErrorCode::Timeout,
        BANNER_RECV | BANNER_SEND | SOCKET_SEND | SOCKET_DISCONNECT | CHANNEL_CLOSED | CHANNEL_EOF_SENT | SOCKET_RECV
        | BAD_SOCKET => ErrorCode::ConnectionFailed,
        AUTHENTICATION_FAILED | PASSWORD_EXPIRED => ErrorCode::AuthFailed,
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

/// Risponde con la password a ogni domanda di keyboard-interactive. Niente `Debug`: contiene la password.
struct Answer<'a>(&'a str);

impl KeyboardInteractivePrompt for Answer<'_> {
    fn prompt<'b>(&mut self, _: &str, _: &str, prompts: &[Prompt<'b>]) -> Vec<String> {
        vec![self.0.to_owned(); prompts.len()]
    }
}

fn tcp(host: &str, port: u16) -> Result<TcpStream, AppError> {
    let fail = |code, e: io::Error| AppError { detail: Some(e.to_string()), ..AppError::new(code, None) };
    let addrs = (host, port).to_socket_addrs().map_err(|e| fail(ErrorCode::ConnectionFailed, e))?;
    let mut last = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    let code = if last.kind() == io::ErrorKind::TimedOut { ErrorCode::Timeout } else { ErrorCode::ConnectionFailed };
    Err(fail(code, last))
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
        return Err(AppError { detail: Some(fp), ..AppError::new(code, None) });
    }
    // Chiede i metodi con l'autenticazione "none": se il server la accetta si è già dentro.
    let methods = sess.auth_methods(&conn.user).map_err(err)?.to_owned();
    if !sess.authenticated() {
        let plan = auth_plan(&methods, password.is_some()).map_err(|code| AppError::new(code, None))?;
        let pw = password.unwrap_or_default();
        match plan {
            Auth::Password => sess.userauth_password(&conn.user, pw),
            Auth::KeyboardInteractive => sess.userauth_keyboard_interactive(&conn.user, &mut Answer(pw)),
        }
        .map_err(err)?;
    }
    sess.set_timeout(IO_TIMEOUT_MS);
    let sftp = sess.sftp().map_err(err)?;
    Ok(SftpProvider { sess, sftp })
}

pub struct SftpFactory;

impl ProviderFactory for SftpFactory {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        Ok(Arc::new(open_in(&crate::config::current_config_dir()?, conn, password)?))
    }
}

pub(crate) struct SftpProvider {
    sess: Session,
    sftp: Sftp,
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
    /// a cui punta. Un collegamento rotto resta un file (la sua copia dà `NOT_FOUND`).
    fn to_stat(&self, name: String, path: &str, l: FileStat) -> Stat {
        let (t, link) = match l.file_type().is_symlink() {
            true => self.sftp.stat(Path::new(path)).map_or((l, false), |t| (t, true)),
            false => (l, false),
        };
        let is_dir = t.is_dir();
        Stat {
            name,
            is_dir,
            dir_link: link && is_dir,
            size: if is_dir { 0 } else { t.size.unwrap_or(0) },
            modified_ms: t.mtime.map(|s| s as i64 * 1000),
            mode: t.perm.map(|m| m & 0o7777),
        }
    }
}

impl Provider for SftpProvider {
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: false, precise_mtime: true, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let l = self.sftp.lstat(Path::new(path)).map_err(self.err(path))?;
        Ok(self.to_stat(name_of(path), path, l))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        let entries = self.sftp.readdir(Path::new(path)).map_err(self.err(path))?;
        Ok(entries
            .into_iter()
            .map(|(p, l)| {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let full = join(path, &name);
                self.to_stat(name, &full, l)
            })
            .collect())
    }

    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let mut file = self.sftp.open(Path::new(path)).map_err(self.err(path))?;
        // Il seek di libssh2 è locale: nessun pacchetto verso il server.
        file.seek(SeekFrom::Start(offset)).map_err(|e| file_error(&self.sess, e, path))?;
        let mut buf = Vec::with_capacity(len as usize);
        file.take(len as u64).read_to_end(&mut buf).map_err(|e| file_error(&self.sess, e, path))?;
        Ok(buf)
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let file = self.sftp.open(Path::new(path)).map_err(self.err(path))?;
        Ok(Box::new(Stream { file, sess: self.sess.clone(), path: path.to_owned() }))
    }

    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let flags = OpenFlags::WRITE | if create_new { OpenFlags::EXCLUSIVE } else { OpenFlags::TRUNCATE };
        let file = self.sftp.open_mode(Path::new(path), flags, 0o644, OpenType::File).map_err(self.exists_err(path))?;
        Ok(Box::new(Stream { file, sess: self.sess.clone(), path: path.to_owned() }))
    }

    /// Senza `posix-rename` non si sostituisce mai: una destinazione esistente dà `ALREADY_EXISTS` e resta intatta,
    /// anche con `replace` (chi chiama, visto `atomic_replace = false`, passa dal fallback confermato).
    fn rename(&self, from: &str, to: &str, _replace: bool) -> Result<(), AppError> {
        if from == to {
            return Ok(());
        }
        if self.sftp.lstat(Path::new(to)).is_ok() {
            return Err(AppError::new(ErrorCode::AlreadyExists, Some(Path::new(to))));
        }
        self.sftp.rename(Path::new(from), Path::new(to), Some(RenameFlags::empty())).map_err(self.err(from))
    }

    /// Un collegamento a cartella si toglie con `unlink`: mai `rmdir` né ricorsione nella cartella a cui punta.
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let p = Path::new(path);
        let link = is_dir && self.sftp.lstat(p).map_err(self.err(path))?.file_type().is_symlink();
        match is_dir && !link {
            true => self.sftp.rmdir(p),
            false => self.sftp.unlink(p),
        }
        .map_err(self.err(path))
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.sftp.mkdir(Path::new(path), 0o755).map_err(self.exists_err(path))
    }

    /// SFTP imposta insieme accesso e modifica: l'accesso prende la stessa data.
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        if modified_ms.is_none() && mode.is_none() {
            return Ok(());
        }
        let secs = modified_ms.map(|ms| ms.div_euclid(1000).max(0) as u64);
        let stat = FileStat { size: None, uid: None, gid: None, perm: mode, atime: secs, mtime: secs };
        self.sftp.setstat(Path::new(path), stat).map_err(self.err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::loc::Scheme;
    use base64::engine::general_purpose::STANDARD;

    #[test]
    fn sftp_status_mapping() {
        assert_eq!(map_sftp_error(FX_NO_SUCH_FILE), ErrorCode::NotFound);
        assert_eq!(map_sftp_error(FX_PERMISSION_DENIED), ErrorCode::AccessDenied);
        assert_eq!(map_sftp_error(4), ErrorCode::Io); // SSH_FX_FAILURE
        assert_eq!(map_sftp_error(FX_FILE_ALREADY_EXISTS), ErrorCode::AlreadyExists);
        assert_eq!(map_sftp_error(FX_CONNECTION_LOST), ErrorCode::ConnectionFailed);
        assert_eq!(map_session_error(SESSION_TIMEOUT), ErrorCode::Timeout);
        assert_eq!(map_session_error(SOCKET_RECV), ErrorCode::ConnectionFailed);
        assert_eq!(map_session_error(AUTHENTICATION_FAILED), ErrorCode::AuthFailed);
    }

    #[test]
    fn auth_without_password_is_required() {
        assert_eq!(auth_plan("publickey,password", false), Err(ErrorCode::AuthRequired));
        assert_eq!(auth_plan("publickey,password", true), Ok(Auth::Password));
        assert_eq!(auth_plan("publickey,keyboard-interactive", true), Ok(Auth::KeyboardInteractive));
        assert_eq!(auth_plan("publickey", true), Err(ErrorCode::AuthFailed));
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

        // Rinomina: una destinazione esistente non si tocca, anche con `replace`.
        p.writer(&b, true).unwrap().write_all(b"b").unwrap();
        assert_eq!(p.rename(&a, &b, false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.rename(&a, &b, true).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"b");
        p.delete(&b, false).unwrap();
        p.rename(&a, &b, false).unwrap();
        assert_eq!(p.stat(&a).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"0123456789");

        p.delete(&b, false).unwrap();
        p.delete(&base, true).unwrap();
        assert_eq!(p.list(&base).unwrap_err().code, ErrorCode::NotFound);
    }
}
