//! Provider SFTP su `russh` e `russh-sftp`: password, impronta dell'host (con il suo tipo) in `known_hosts`.
//!
//! Un solo canale `sftp` per connessione, con la sessione grezza di russh-sftp per tutto: operazioni, stream e
//! `posix-rename@openssh.com`. Le richieste girano sul runtime `sftp-rt` (`rt`) e ogni metodo vi si affaccia con
//! `rt::block`, entro `IO_TIMEOUT_MS`. I metodi del provider e degli stream si chiamano solo da thread bloccanti, mai
//! da un task tokio: lì `block_on` va in panico, e il profilo release ha `panic = "abort"`. La connessione si chiude
//! nel `Drop` di `Link`, sul runtime, quando provider e stream sono caduti tutti.
//!
//! Nomi remoti non rappresentabili: l'elenco li mostra con la decodifica lossy di russh-sftp (U+FFFD), e ogni
//! operazione su un percorso con U+FFFD dà `INVALID_NAME` senza inviare nulla (il nome vero non si ricostruisce).
//! ponytail: anche i percorsi con `\` danno `INVALID_NAME`: un nome remoto `a\b` copiato su Windows finirebbe nella
//! sottocartella `a`, e i trasferimenti e `net/temp.rs` controllano solo l'ultimo componente. Si sblocca con un
//! controllo sulla destinazione locale.

use super::connections::Connection;
use super::sessions::ProviderFactory;
use super::{MAX_ENTRIES, tcp};
use crate::error::{AppError, ErrorCode};
use crate::fs::hash::sha256;
use crate::vfs::provider::{Caps, Provider, Stat};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use connect::{Link, Options, POSIX_RENAME, connect};
use errors::sftp_error;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Packet, StatusCode};
use std::future::Future;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use stream::{IN_FLIGHT, Reader, Writer, read_range};
use tokio::io::{AsyncRead, AsyncWrite};

mod connect;
mod errors;
#[cfg(test)]
mod fake;
mod rt;
mod stream;

/// Impronta come `ssh-keygen -lf`: `SHA256:` e il base64 senza padding del blob della chiave.
pub fn fingerprint(key: &[u8]) -> Result<String, AppError> {
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(sha256(key)?)))
}

/// Apre una sessione SFTP verificando l'impronta in `dir/known_hosts`.
pub(crate) fn open_in(dir: &Path, conn: &Connection, password: Option<&str>) -> Result<SftpProvider, AppError> {
    let io = |e: std::io::Error| AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) };
    let tcp = tcp(&conn.host, conn.port)?;
    tcp.set_nodelay(true).map_err(io)?;
    tcp.set_nonblocking(true).map_err(io)?;
    // `from_std` registra il socket nel reactor del runtime, che deve essere quello corrente.
    let stream = {
        let _rt = rt::runtime()?.enter();
        tokio::net::TcpStream::from_std(tcp).map_err(io)?
    };
    open_on(dir, conn, password, stream, &Options::DEFAULT)
}

/// `connect` su `stream`. Il limite di fuori non taglia quelli di dentro: `connect_timeout` per la connessione, poi
/// `init` e `limits@openssh.com` entro `io_timeout` ciascuna (russh-sftp lo arrotonda per eccesso al secondo).
fn open_on<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    dir: &Path,
    conn: &Connection,
    password: Option<&str>,
    stream: S,
    opts: &Options,
) -> Result<SftpProvider, AppError> {
    let limit = opts.connect_timeout + 2 * (opts.io_timeout + Duration::from_secs(1));
    let c = rt::block(limit, connect(dir, conn, password, stream, opts))?;
    Ok(SftpProvider { link: c.link, atomic: c.atomic, chunk: c.chunk, io_timeout: opts.io_timeout })
}

pub struct SftpFactory;

impl ProviderFactory for SftpFactory {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        Ok(Arc::new(open_in(&crate::config::current_config_dir()?, conn, password)?))
    }
}

/// La connessione si chiude nel `Drop` di `Link`, quando anche gli stream aperti sono caduti.
pub(crate) struct SftpProvider {
    link: Arc<Link>,
    /// Il server ha `posix-rename@openssh.com`.
    atomic: bool,
    /// Byte per richiesta di lettura o scrittura.
    chunk: u32,
    io_timeout: Duration,
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
fn checked(path: &str) -> Result<&str, AppError> {
    match path.contains(['\u{FFFD}', '\\']) {
        true => Err(AppError::new(ErrorCode::InvalidName, Some(Path::new(path)))),
        false => Ok(path),
    }
}

/// I dati di `posix-rename@openssh.com`: i due percorsi come stringhe SSH (lunghezza `u32` big-endian e byte).
fn ssh_strings(a: &str, b: &str) -> Vec<u8> {
    [a, b].iter().flat_map(|s| (s.len() as u32).to_be_bytes().into_iter().chain(s.bytes())).collect()
}

impl SftpProvider {
    /// `fut` entro `io_timeout`; ogni errore porta `path`.
    fn block<T>(&self, path: &str, fut: impl Future<Output = Result<T, AppError>>) -> Result<T, AppError> {
        rt::block(self.io_timeout, fut).map_err(|e| AppError { path: Some(path.to_owned()), ..e })
    }

    /// Una richiesta sulla sessione grezza, entro `io_timeout`.
    fn run<T>(&self, path: &str, fut: impl Future<Output = Result<T, SftpError>>) -> Result<T, AppError> {
        self.block(path, async { fut.await.map_err(|e| sftp_error(e, None)) })
    }

    /// Chiude `handle` sul runtime, senza aspettare.
    fn close(&self, handle: String) {
        let sftp = self.link.sftp.clone();
        rt::spawn(async move {
            let _ = sftp.close(handle).await;
        });
    }

    fn lstat(&self, path: &str) -> Result<FileAttributes, AppError> {
        self.run(path, async { self.link.sftp.lstat(path).await.map(|a| a.attrs) })
    }

    /// In SFTP v3 "esiste già" arriva come `SSH_FX_FAILURE`: si controlla se `path` c'è.
    fn exists_err(&self, path: &str, mut e: AppError) -> AppError {
        if e.code == ErrorCode::Io && self.lstat(path).is_ok() {
            e.code = ErrorCode::AlreadyExists;
        }
        e
    }

    /// Da `lstat`: un collegamento si risolve con `stat`. A cartella è una cartella con `dir_link`; a file è il file
    /// a cui punta. Un collegamento rotto, o con un nome che non si può inviare, resta un file (la sua copia dà
    /// `NOT_FOUND` o `INVALID_NAME`).
    fn to_stat(&self, name: String, path: &str, l: FileAttributes) -> Stat {
        let (t, link) = match l.file_type().is_symlink() && checked(path).is_ok() {
            true => match self.run(path, async { self.link.sftp.stat(path).await.map(|a| a.attrs) }) {
                Ok(t) => (t, true),
                Err(_) => (l, false),
            },
            false => (l, false),
        };
        let is_dir = t.file_type().is_dir();
        Stat {
            name,
            is_dir,
            dir_link: link && is_dir,
            size: if is_dir { 0 } else { t.size.unwrap_or(0) },
            modified_ms: mtime_ms(t.mtime.map(u64::from)),
            mode: t.permissions.map(|m| m & 0o7777),
        }
    }
}

impl Provider for SftpProvider {
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: self.atomic, precise_mtime: true, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let l = self.lstat(checked(path)?)?;
        Ok(self.to_stat(name_of(path), path, l))
    }

    /// `opendir`, `readdir` fino a `EOF`, `close`; senza `.` e `..`.
    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        let sftp = &self.link.sftp;
        let handle = self.run(checked(path)?, sftp.opendir(path))?.handle;
        let mut files = Vec::new();
        let read = loop {
            let page = self.run(path, async {
                match sftp.readdir(handle.as_str()).await {
                    Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => Ok(None),
                    r => r.map(|n| Some(n.files)),
                }
            });
            match page {
                Ok(Some(page)) => files.extend(page.into_iter().filter(|f| f.filename != "." && f.filename != "..")),
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
            // Un server ostile non riempie la memoria.
            if files.len() > MAX_ENTRIES {
                break Err(AppError::new(ErrorCode::TooLarge, Some(Path::new(path))));
            }
        };
        self.close(handle);
        read?;
        Ok(files
            .into_iter()
            .map(|f| {
                let full = join(path, &f.filename);
                self.to_stat(f.filename, &full, f.attrs)
            })
            .collect())
    }

    /// Un segmento da `IN_FLIGHT` blocchi alla volta, ognuno entro `io_timeout`: una lettura grande su una linea
    /// lenta non scade.
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let sftp = &self.link.sftp;
        let handle = self.run(checked(path)?, sftp.open(path, OpenFlags::READ, FileAttributes::default()))?.handle;
        let segment = self.chunk.max(1).saturating_mul(IN_FLIGHT as u32);
        let mut out = Vec::new();
        let read = loop {
            let done = out.len() as u32;
            if done >= len {
                break Ok(());
            }
            let want = (len - done).min(segment);
            match self.block(path, read_range(sftp, &handle, offset.saturating_add(done as u64), want, self.chunk)) {
                Ok(part) => {
                    let short = (part.len() as u32) < want;
                    out.extend(part);
                    if short {
                        break Ok(());
                    }
                }
                Err(e) => break Err(e),
            }
        };
        self.close(handle);
        read.map(|()| out)
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let open = self.link.sftp.open(checked(path)?, OpenFlags::READ, FileAttributes::default());
        let handle = self.run(path, open)?.handle;
        Ok(Box::new(Reader::new(self.link.clone(), handle, path.to_owned(), self.chunk, self.io_timeout)))
    }

    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let flags = OpenFlags::WRITE | OpenFlags::CREATE | if create_new { OpenFlags::EXCLUDE } else { OpenFlags::TRUNCATE };
        let attrs = FileAttributes { permissions: Some(0o644), ..Default::default() };
        let open = self.link.sftp.open(checked(path)?, flags, attrs);
        // Solo con `create_new` un `SSH_FX_FAILURE` su un percorso esistente vuol dire "esiste già".
        let handle = self.run(path, open).map_err(|e| match create_new {
            true => self.exists_err(path, e),
            false => e,
        })?;
        Ok(Box::new(Writer::new(self.link.clone(), handle.handle, path.to_owned(), self.chunk, self.io_timeout)))
    }

    /// Con `replace` e `posix-rename` sostituisce in un colpo solo. Altrimenti non sostituisce mai: una destinazione
    /// esistente dà `ALREADY_EXISTS` e resta intatta (la `rename` di SFTP v3 non ha flag di sovrascrittura; chi
    /// chiama, visto `atomic_replace = false`, passa dal fallback confermato).
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        let (f, t) = (checked(from)?, checked(to)?);
        if from == to {
            return Ok(());
        }
        let sftp = &self.link.sftp;
        if replace && self.atomic {
            return self.run(from, async {
                match sftp.extended(POSIX_RENAME, ssh_strings(f, t)).await {
                    Ok(Packet::Status(s)) if s.status_code == StatusCode::Ok => Ok(()),
                    Ok(Packet::Status(s)) => Err(SftpError::Status(s)),
                    Ok(_) => Err(SftpError::UnexpectedPacket),
                    Err(e) => Err(e),
                }
            });
        }
        if self.lstat(t).is_ok() {
            return Err(AppError::new(ErrorCode::AlreadyExists, Some(Path::new(to))));
        }
        self.run(from, sftp.rename(f, t)).map(drop)
    }

    /// Un collegamento a cartella si toglie con `remove`: mai `rmdir` né ricorsione nella cartella a cui punta.
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let p = checked(path)?;
        let link = is_dir && self.lstat(p)?.file_type().is_symlink();
        let sftp = &self.link.sftp;
        match is_dir && !link {
            true => self.run(p, sftp.rmdir(p)),
            false => self.run(p, sftp.remove(p)),
        }
        .map(drop)
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        let attrs = FileAttributes { permissions: Some(0o755), ..Default::default() };
        let mkdir = self.link.sftp.mkdir(checked(path)?, attrs);
        self.run(path, mkdir).map(drop).map_err(|e| self.exists_err(path, e))
    }

    /// SFTP imposta insieme accesso e modifica: l'accesso prende la stessa data (secondi in `u32`, saturati).
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        if modified_ms.is_none() && mode.is_none() {
            return Ok(());
        }
        let secs = modified_ms.map(|ms| ms.div_euclid(1000).clamp(0, u32::MAX as i64) as u32);
        let attrs = FileAttributes { permissions: mode, atime: secs, mtime: secs, ..Default::default() };
        let setstat = self.link.sftp.setstat(checked(path)?, attrs);
        self.run(path, setstat).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::trust;
    use crate::vfs::loc::Scheme;
    use base64::engine::general_purpose::STANDARD;
    use fake::{FakeOptions, FakeServer, conn, trusted};
    use std::io::{Read, Write};

    #[test]
    fn unrepresentable_names_are_never_sent() {
        // Un nome Latin-1 si mostra con U+FFFD; qualunque operazione su quel percorso dà `INVALID_NAME`.
        let name = String::from_utf8_lossy(b"caf\xe9.txt").into_owned();
        assert_eq!(name, "caf\u{FFFD}.txt");
        assert_eq!(checked(&join("/d", &name)).unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(checked("/d/a\\b").unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(checked("/d/caf\u{e9}.txt").unwrap(), "/d/caf\u{e9}.txt");
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
    fn fingerprint_format() {
        // Chiave ed25519 di github.com e la sua impronta pubblicata in "GitHub's SSH key fingerprints"
        // (docs.github.com); `ssh-keygen -lf` sulla stessa riga dà la stessa impronta.
        let blob = STANDARD.decode("AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl").unwrap();
        assert_eq!(fingerprint(&blob).unwrap(), "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU");
    }

    /// Il provider sul finto: impronta accettata, password giusta.
    fn on_fake(host: &str, opts: FakeOptions, o: &Options) -> (FakeServer, SftpProvider) {
        let dir = tempfile::tempdir().unwrap();
        let (fake, s) = trusted(dir.path(), host, opts);
        let p = open_on(dir.path(), &conn(host), Some("pw"), s, o).map_err(|e| e.code).unwrap();
        (fake, p)
    }

    /// `SSH_FXP_SYMLINK` con la sessione grezza del provider: `true` se riesce.
    fn symlink(p: &SftpProvider, linkpath: &str, target: &str) -> bool {
        let sftp = &p.link.sftp;
        rt::block(Duration::from_secs(10), async { sftp.symlink(linkpath, target).await.map_err(|e| sftp_error(e, None)) })
            .is_ok()
    }

    fn write_new(p: &SftpProvider, path: &str, data: &[u8]) {
        let mut w = p.writer(path, true).unwrap();
        w.write_all(data).unwrap();
        w.flush().unwrap();
    }

    /// Le operazioni di `sftp_live`, sul finto e sul server vero, in una cartella nuova dentro `dir`.
    /// `symlink(target, at)` crea il collegamento `at`.
    fn exercise(p: &SftpProvider, dir: &str, symlink: impl Fn(&str, &str)) {
        let base = join(dir, &format!("mc-sftp-live-{}", uuid::Uuid::new_v4().simple()));
        p.mkdir(&base).unwrap();
        assert_eq!(p.mkdir(&base).unwrap_err().code, ErrorCode::AlreadyExists);
        let a = join(&base, "a.txt");
        let b = join(&base, "b.txt");
        write_new(p, &a, b"0123456789");
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
        write_new(p, &b, b"b");
        assert_eq!(p.rename(&a, &b, false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"b");
        let c = join(&base, "c.txt");
        write_new(p, &c, b"c");
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
        // punta).
        let sub = join(&base, "sub");
        p.mkdir(&sub).unwrap();
        let dl = join(&base, "dl");
        let fl = join(&base, "fl");
        symlink(&sub, &dl);
        symlink(&b, &fl);
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

    fn exercise_on_fake(host: &str, posix_rename: bool) {
        let (_fake, p) = on_fake(host, FakeOptions { posix_rename, ..Default::default() }, &Options::DEFAULT);
        assert_eq!(p.caps().atomic_replace, posix_rename);
        p.mkdir("/home").unwrap();
        p.mkdir("/home/u").unwrap();
        // Il finto segue la specifica: prima il collegamento, poi la destinazione.
        exercise(&p, "/home/u", |target, at| assert!(symlink(&p, at, target), "symlink {at}"));
    }

    #[test]
    fn provider_on_fake_with_posix_rename() {
        exercise_on_fake("provider-posix.test", true);
    }

    #[test]
    fn provider_on_fake_without_posix_rename() {
        exercise_on_fake("provider-plain.test", false);
    }

    /// Il server di russh-sftp scrive solo `String`, quindi il finto non manda byte non UTF-8: la decodifica lossy è
    /// di russh-sftp. Qui il nome con U+FFFD si elenca e nessuna operazione lo invia.
    #[test]
    fn unrepresentable_name_listed_but_never_sent() {
        let (fake, p) = on_fake("unrepresentable.test", FakeOptions::default(), &Options::DEFAULT);
        p.mkdir("/d").unwrap();
        let bad = "/d/caf\u{FFFD}.txt";
        fake.put(bad, b"x");
        let names: Vec<String> = p.list("/d").unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["caf\u{FFFD}.txt"]);
        assert_eq!(p.stat(bad).err().unwrap().code, ErrorCode::InvalidName);
        assert_eq!(p.read_at(bad, 0, 1).unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(p.delete(bad, false).unwrap_err().code, ErrorCode::InvalidName);
        assert_eq!(fake.get(bad).as_deref(), Some(&b"x"[..]));
    }

    /// Più segmenti da `IN_FLIGHT` blocchi, ognuno con il suo limite.
    #[test]
    fn read_at_spans_many_requests() {
        let (fake, p) = on_fake("read-at.test", FakeOptions { short_reads: true, ..Default::default() }, &Options::DEFAULT);
        let data: Vec<u8> = (0..1_200_000u32).map(|i| (i % 251) as u8).collect();
        fake.put("/big", &data);
        assert!(p.read_at("/big", 1000, 1_100_000).unwrap() == data[1000..1_101_000]);
        assert!(p.read_at("/big", 1_199_990, 100).unwrap() == data[1_199_990..]);
    }

    /// Server fermo: `TIMEOUT` con il percorso, presto.
    #[test]
    fn stalled_server_times_out_with_the_path() {
        let o = Options { io_timeout: Duration::from_millis(500), ..Options::DEFAULT };
        let (fake, p) = on_fake("stalled-provider.test", FakeOptions::default(), &o);
        fake.stall();
        let t = std::time::Instant::now();
        let e = p.stat("/x").err().unwrap();
        assert_eq!((e.code, e.path.as_deref()), (ErrorCode::Timeout, Some("/x")));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
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
        // L'ordine degli argomenti di `SSH_FXP_SYMLINK` è invertito in OpenSSH: vale l'ordine che riesce (l'altro
        // proverebbe a creare il collegamento sopra il bersaglio, che esiste).
        exercise(&p, dir, |target, at| assert!(symlink(&p, at, target) || symlink(&p, target, at), "symlink {at}"));
    }
}
