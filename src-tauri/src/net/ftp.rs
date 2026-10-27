//! Provider FTP e FTPS (`suppaftp`, TLS di `net::tls`).
//!
//! Modalità passiva, con i dati sempre verso l'indirizzo della connessione di controllo: l'IP della risposta `PASV`
//! si ignora (dietro un NAT è privato, e un server ostile non fa collegare il client altrove). FTPS esplicito
//! (`AUTH TLS`, poi `PBSZ 0` e `PROT P`) o implicito (`Connection.ftps_implicit`, di solito la porta 990).
//!
//! La connessione di controllo fa una cosa alla volta: le operazioni si mettono in fila su `Ctl`, e uno stream di
//! `reader`/`writer` la tiene occupata finché non si chiude (le altre aspettano fino a 30 s, poi `TIMEOUT`).
//! Chiudere uno stream conclude il trasferimento leggendo la risposta del server, così la connessione resta usabile.
//! Una connessione caduta o in timeout non si riusa: ogni operazione successiva dà `CONNECTION_FAILED`.
//!
//! Collegamenti simbolici: `LIST` e `MLSD` non dicono se puntano a una cartella. Un collegamento è a cartella se
//! `CWD` ci entra (`dir_link`); altrimenti è il file a cui punta (o un collegamento rotto), con la dimensione di `SIZE`.
//!
//! Nomi non UTF-8: come in SFTP, si mostrano con U+FFFD e ogni operazione su quei percorsi dà `INVALID_NAME`.
//! ponytail: `suppaftp` scrive i comandi, `PASS` compreso, nel log a livello trace. Nelle build di rilascio il
//! livello trace è tolto (`log` con `release_max_level_debug`); in debug l'app non installa un logger, ma se se ne
//! aggiunge uno `suppaftp` va filtrato sotto trace.

use super::connections::Connection;
use super::sessions::ProviderFactory;
use super::MAX_ENTRIES;
use super::http_index::LastListing;
use super::{CONNECT_TIMEOUT, IO_TIMEOUT_MS, drop_in_background, tcp};
use crate::error::{AppError, ErrorCode};
use crate::vfs::provider::{Caps, Provider, Stat};
use rustls::ClientConfig;
use std::io::{self, BufRead, Read, Write};
use std::mem::ManuallyDrop;
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, UNIX_EPOCH};
use suppaftp::list::{File, ListParser, PosixPexQuery};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpResult, ImplFtpStream, Mode, RustlsConnector, Status, TlsConnector, TransferStream};

const IO_TIMEOUT: Duration = Duration::from_millis(IO_TIMEOUT_MS as u64);

/// Connettore TLS che mette i timeout sul socket prima dell'handshake (la connessione implicita la apre `suppaftp`).
#[derive(Debug)]
struct Tls(RustlsConnector);

impl TlsConnector for Tls {
    type Stream = <RustlsConnector as TlsConnector>::Stream;

    fn connect(&self, domain: &str, stream: TcpStream) -> FtpResult<Self::Stream> {
        timeouts(&stream).map_err(FtpError::ConnectionError)?;
        self.0.connect(domain, stream)
    }
}

type Ftp = ImplFtpStream<<Tls as TlsConnector>::Stream>;
type Data = TransferStream<<Tls as TlsConnector>::Stream>;

fn timeouts(s: &TcpStream) -> io::Result<()> {
    s.set_read_timeout(Some(IO_TIMEOUT))?;
    s.set_write_timeout(Some(IO_TIMEOUT))
}

/// Connessione dati passiva, con gli stessi timeout del controllo.
fn data_stream(addr: std::net::SocketAddr) -> FtpResult<TcpStream> {
    let s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(FtpError::ConnectionError)?;
    timeouts(&s).map_err(FtpError::ConnectionError)?;
    Ok(s)
}

/// Codice di una risposta del server. `550` copre sia "non esiste" sia "non permesso": decide il testo.
fn reply_code(code: u32, text: &str) -> ErrorCode {
    let t = text.to_ascii_lowercase();
    match code {
        421 | 425 | 426 => ErrorCode::ConnectionFailed,
        530 => ErrorCode::AuthFailed,
        // 553: nome non permesso, per esempio un file che non si può creare nella cartella.
        532 | 553 => ErrorCode::AccessDenied,
        550 if t.contains("denied") || t.contains("permission") => ErrorCode::AccessDenied,
        550 => ErrorCode::NotFound,
        452 | 552 => ErrorCode::DiskFull,
        500 | 502 | 504 => ErrorCode::Unsupported,
        _ => ErrorCode::Io,
    }
}

fn io_code(e: &io::Error) -> ErrorCode {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ErrorCode::Timeout,
        _ => ErrorCode::ConnectionFailed,
    }
}

/// Le risposte dei server e gli errori di `suppaftp` non contengono la password.
fn ftp_error(e: FtpError, path: Option<&str>) -> AppError {
    let (code, detail) = match &e {
        FtpError::ConnectionError(io) => (io_code(io), io.to_string()),
        FtpError::UnexpectedResponse(r) => {
            let text = String::from_utf8_lossy(&r.body).trim_end().to_owned();
            (reply_code(r.status.code(), &text), text)
        }
        // Una risposta illeggibile o un errore TLS: le risposte successive potrebbero non essere allineate, la sessione
        // va scartata come dopo una caduta.
        FtpError::BadResponse | FtpError::SecureError(_) => (ErrorCode::ConnectionFailed, e.to_string()),
        _ => (ErrorCode::Io, e.to_string()),
    };
    AppError { detail: Some(detail), ..AppError::new(code, path.map(Path::new)) }
}

fn fe(path: &str) -> impl Fn(FtpError) -> AppError + '_ {
    move |e| ftp_error(e, Some(path))
}

/// Un comando di cui un rifiuto del server è una risposta valida (`None`). Ogni altro errore (connessione, timeout,
/// risposta illeggibile) si propaga: la sessione va segnata come rotta, o una risposta in ritardo verrebbe letta
/// come quella del comando successivo.
fn soft<T>(r: FtpResult<T>) -> FtpResult<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(FtpError::UnexpectedResponse(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn io_error(e: io::Error, path: &str) -> AppError {
    AppError { detail: Some(e.to_string()), ..AppError::new(io_code(&e), Some(Path::new(path))) }
}

/// Permessi POSIX dalle triadi di `LIST`/`MLSD` (senza setuid, setgid e sticky).
fn mode_of(f: &File) -> u32 {
    let bits = |q| u32::from(f.can_read(q)) << 2 | u32::from(f.can_write(q)) << 1 | u32::from(f.can_execute(q));
    bits(PosixPexQuery::Owner) << 6 | bits(PosixPexQuery::Group) << 3 | bits(PosixPexQuery::Others)
}

/// Una riga di `MLSD` o di `LIST` (Unix, poi DOS). Niente per `.`, `..`, la cartella stessa e le righe che non si
/// capiscono (`total 8`). Un collegamento esce come collegamento a cartella in sospeso: lo verifica `list_in`.
/// Le date di `LIST` sono al minuto (o al giorno, per i file vecchi); quelle di `MLSD` al secondo.
fn parse_line(line: &str, mlsd: bool) -> Option<Stat> {
    let fixed: String;
    let (f, has_mode) = if mlsd {
        // Come in `suppaftp`: i fatti finiscono al primo "; ", poi c'è il nome.
        let end = line.find("; ");
        let facts = end.map_or("", |e| &line[..e]);
        let lower = facts.to_ascii_lowercase();
        let ty = lower.split(';').find_map(|f| f.strip_prefix("type=")).unwrap_or_default();
        if matches!(ty, "cdir" | "pdir") {
            return None;
        }
        // `type=OS.unix=slink:<bersaglio>` o `OS.unix=symlink` (ProFTPD, Pure-FTPd): `suppaftp` non li conosce e
        // scarterebbe la riga. Diventano `type=link`, un collegamento in sospeso come le voci `l` di `LIST`.
        // ponytail: un server che segue i collegamenti senza dirlo mostra un collegamento a cartella come una
        // cartella vera, che la copia attraversa e l'eliminazione svuota; non lo si può distinguere da qui.
        let line = match end {
            Some(e) if ty.starts_with("os.unix=slink") || ty.starts_with("os.unix=symlink") => {
                let facts: Vec<&str> =
                    facts.split(';').map(|f| if f.to_ascii_lowercase().starts_with("type=") { "type=link" } else { f }).collect();
                fixed = format!("{}; {}", facts.join(";"), &line[e + 2..]);
                fixed.as_str()
            }
            _ => line,
        };
        (ListParser::parse_mlsd(line).ok()?, lower.contains("unix.mode="))
    } else if let Ok(f) = ListParser::parse_posix(line) {
        (f, true)
    } else {
        (ListParser::parse_dos(line).ok()?, false)
    };
    if matches!(f.name(), "." | "..") {
        return None;
    }
    let link = f.is_symlink();
    let modified_ms =
        f.modified().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_millis()).ok()).filter(|&ms| ms != 0);
    Some(Stat {
        name: f.name().to_owned(),
        is_dir: f.is_directory() || link,
        dir_link: link,
        size: if f.is_directory() { 0 } else { f.size() as u64 },
        modified_ms,
        mode: has_mode.then(|| mode_of(&f)),
    })
}

/// Byte al massimo di un elenco (`LIST`/`MLSD`).
const LISTING_BYTES: u64 = 64 << 20;

/// Le righe non vuote di un elenco, in UTF-8 lossy (come `suppaftp`). Oltre `MAX_ENTRIES` righe o `LISTING_BYTES`
/// byte: `TOO_LARGE`, senza leggere il resto.
fn read_lines(r: impl Read, dir: &str) -> Result<Vec<String>, AppError> {
    let mut r = io::BufReader::new(r.take(LISTING_BYTES + 1));
    let (mut lines, mut total, mut buf) = (Vec::new(), 0u64, Vec::new());
    let too_large = || AppError::new(ErrorCode::TooLarge, Some(Path::new(dir)));
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf).map_err(|e| io_error(e, dir))?;
        if n == 0 {
            return Ok(lines);
        }
        total += n as u64;
        if total > LISTING_BYTES {
            return Err(too_large());
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if lines.len() == MAX_ENTRIES {
            return Err(too_large());
        }
        lines.push(String::from_utf8_lossy(line).into_owned());
    }
}

/// Un percorso con U+FFFD (nome non UTF-8) o con un a capo (spezzerebbe il comando) dà `INVALID_NAME`.
fn checked(path: &str) -> Result<&str, AppError> {
    match path.contains(['\u{FFFD}', '\r', '\n']) {
        true => Err(AppError::new(ErrorCode::InvalidName, Some(Path::new(path)))),
        false => Ok(path),
    }
}

fn name_of(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

fn parent_of(path: &str) -> &str {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((p, _)) => p,
    }
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

fn dropped(e: &AppError) -> bool {
    matches!(e.code, ErrorCode::ConnectionFailed | ErrorCode::Timeout)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct State {
    ftp: Ftp,
    /// Uno stream tiene la connessione.
    busy: bool,
    /// Caduta o in timeout: le risposte potrebbero non essere più allineate ai comandi.
    broken: bool,
}

impl State {
    fn note<T>(&mut self, r: Result<T, AppError>) -> Result<T, AppError> {
        if let Err(e) = &r
            && dropped(e)
        {
            self.broken = true;
        }
        r
    }
}

struct Ctl {
    state: Mutex<State>,
    idle: Condvar,
}

impl Ctl {
    /// La connessione libera; `TIMEOUT` se uno stream la tiene oltre il timeout, `CONNECTION_FAILED` se è caduta.
    fn free(&self) -> Result<MutexGuard<'_, State>, AppError> {
        let s = lock(&self.state);
        let (s, _) = self.idle.wait_timeout_while(s, IO_TIMEOUT, |s| s.busy).unwrap_or_else(|e| e.into_inner());
        if s.busy {
            return Err(AppError::new(ErrorCode::Timeout, None));
        }
        if s.broken {
            return Err(AppError::new(ErrorCode::ConnectionFailed, None));
        }
        Ok(s)
    }
}

#[derive(Clone, Copy)]
struct Feat {
    mlst: bool,
    mdtm: bool,
    size: bool,
    mfmt: bool,
}

/// Collega, verifica il certificato (FTPS) con le impronte di `dir/known_hosts` e accede.
pub(crate) fn open_in(dir: &Path, conn: &Connection, password: Option<&str>, tls: bool) -> Result<FtpProvider, AppError> {
    let secure = if tls { Some(super::tls::ftps_config_in(dir, &conn.host, conn.port)?) } else { None };
    // Un certificato rifiutato fa fallire l'handshake con un errore di I/O: l'impronta nello slot dice il perché.
    let untrusted = |e: AppError| match secure.as_ref().and_then(|(_, slot)| lock(slot).take()) {
        Some(fp) => AppError { detail: Some(fp), ..AppError::new(ErrorCode::CertUntrusted, None) },
        None => e,
    };
    let mut ftp = connect(conn, secure.as_ref().map(|(c, _)| c)).map_err(untrusted)?;
    let err = |e| ftp_error(e, None);
    // Senza password: anonimo se l'utente è vuoto o `anonymous`, come vuole la convenzione FTP; altrimenti si chiede.
    let user = if conn.user.is_empty() { "anonymous" } else { conn.user.as_str() };
    let pass = match password {
        Some(p) => p,
        None if user.eq_ignore_ascii_case("anonymous") => "anonymous@",
        None => return Err(AppError::new(ErrorCode::AuthRequired, None)),
    };
    ftp.login(user, pass).map_err(err)?;
    ftp.transfer_type(FileType::Binary).map_err(err)?;
    // Un server senza `FEAT` non ha nessuna delle estensioni.
    let features = soft(ftp.feat()).map_err(err)?.unwrap_or_default();
    let has = |k: &str| features.keys().any(|f| f.eq_ignore_ascii_case(k));
    if has("UTF8") {
        soft(ftp.opts("UTF8", Some("ON"))).map_err(err)?;
    }
    let feat = Feat { mlst: has("MLST"), mdtm: has("MDTM"), size: has("SIZE"), mfmt: has("MFMT") };
    let state = State { ftp, busy: false, broken: false };
    let ctl = Arc::new(Ctl { state: Mutex::new(state), idle: Condvar::new() });
    Ok(FtpProvider { ctl: ManuallyDrop::new(ctl), feat, last: LastListing::default() })
}

fn connect(conn: &Connection, tls: Option<&Arc<ClientConfig>>) -> Result<Ftp, AppError> {
    let err = |e| ftp_error(e, None);
    let io = |e| ftp_error(FtpError::ConnectionError(e), None);
    let tls_for = |c: &Arc<ClientConfig>| Tls(RustlsConnector::from(Arc::clone(c)));
    let ftp = match tls {
        Some(c) if conn.ftps_implicit => {
            // `suppaftp` apre da sé la connessione implicita, con il timeout del sistema (21 s): l'indirizzo si sceglie
            // prima con il timeout di 15 s, e la connessione di prova si chiude subito.
            // ponytail: una connessione in più verso il server; si toglie se `suppaftp` accetta un socket già aperto.
            let addr = tcp(&conn.host, conn.port)?.peer_addr().map_err(io)?;
            let mut ftp = Ftp::connect_secure_implicit(addr, tls_for(c), &conn.host).map_err(err)?;
            timeouts(&ftp.get_ref()).map_err(io)?;
            // A differenza di `into_secure`, `suppaftp` qui non chiede la protezione dei dati: senza `PROT P` un server
            // che la esige rifiuta ogni elenco e trasferimento (vsftpd: "522 Data connections must be encrypted").
            ftp.custom_command("PBSZ 0", &[Status::CommandOk]).map_err(err)?;
            ftp.custom_command("PROT P", &[Status::CommandOk]).map_err(err)?;
            ftp
        }
        _ => {
            let s = tcp(&conn.host, conn.port)?;
            timeouts(&s).map_err(io)?;
            let ftp = Ftp::connect_with_stream(s).map_err(err)?;
            match tls {
                Some(c) => ftp.into_secure(tls_for(c), &conn.host).map_err(err)?,
                None => ftp,
            }
        }
    };
    let mut ftp = ftp.passive_stream_builder(data_stream);
    ftp.set_passive_nat_workaround(true);
    // `PASV` porta solo indirizzi IPv4: su IPv6 serve `EPSV`, che dà solo la porta.
    if ftp.get_ref().peer_addr().is_ok_and(|a| a.is_ipv6()) {
        ftp.set_mode(Mode::ExtendedPassive);
    }
    Ok(ftp)
}

/// `tls`: FTPS (esplicito o implicito, come dice la connessione).
pub struct FtpFactory {
    pub tls: bool,
}

impl ProviderFactory for FtpFactory {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        Ok(Arc::new(open_in(&crate::config::current_config_dir()?, conn, password, self.tls)?))
    }
}

/// La connessione si chiude in `Drop`, su un altro thread (vedi `drop_in_background`).
pub(crate) struct FtpProvider {
    ctl: ManuallyDrop<Arc<Ctl>>,
    feat: Feat,
    /// Per `stat` senza `MLST`; `list` lo rinnova, ogni comando che cambia il server lo dimentica.
    last: LastListing,
}

impl Drop for FtpProvider {
    fn drop(&mut self) {
        // SAFETY: il campo si prende una volta sola, qui, e dopo `drop` non si usa più.
        drop_in_background(unsafe { ManuallyDrop::take(&mut self.ctl) });
    }
}

/// Lettura o scrittura di un file: tiene occupata la connessione di controllo finché non si conclude. Gli errori
/// portano l'`AppError` dentro `io::Error::other`.
struct Stream {
    data: Option<Data>,
    ctl: Arc<Ctl>,
    path: String,
}

impl Stream {
    /// Chiude la connessione dati, legge la risposta del server e libera la connessione. `early`: chiusa prima della
    /// fine, quindi la risposta (226, oppure 426 per il trasferimento interrotto) si legge e basta.
    fn end(&mut self, early: bool) -> io::Result<()> {
        let Some(mut data) = self.data.take() else { return Ok(()) };
        let flushed = data.flush();
        let r = match data.finish() {
            Err(FtpError::UnexpectedResponse(_)) if early => Ok(()),
            r => r.map_err(|e| ftp_error(e, Some(&self.path))),
        };
        // La risposta del server (552 spazio esaurito, ...) spiega meglio dell'errore di scrittura.
        let r = r.and(flushed.map_err(|e| io_error(e, &self.path)));
        let mut s = lock(&self.ctl.state);
        let r = s.note(r);
        s.busy = false;
        drop(s);
        self.ctl.idle.notify_all();
        r.map_err(io::Error::other)
    }
}

impl Read for Stream {
    /// A fine file la risposta del server dice se il file è arrivato intero.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(data) = &mut self.data else { return Ok(0) };
        match data.read(buf) {
            Ok(0) if !buf.is_empty() => self.end(false).map(|()| 0),
            r => r.map_err(|e| io::Error::other(io_error(e, &self.path))),
        }
    }
}

impl Write for Stream {
    /// Se il server chiude la connessione dati a metà (552, quota, ...), l'errore è la sua risposta finale.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(data) = &mut self.data else {
            return Err(io::Error::other(AppError::new(ErrorCode::Io, Some(Path::new(&self.path)))));
        };
        match data.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                let e = io::Error::other(io_error(e, &self.path));
                Err(self.end(false).err().unwrap_or(e))
            }
        }
    }

    /// Conclude il caricamento e ne dà l'esito: in FTP un errore (spazio esaurito, ...) arriva con la risposta finale.
    /// Dopo `flush` non si scrive più.
    fn flush(&mut self) -> io::Result<()> {
        self.end(false)
    }
}

/// Lasciato cadere prima della fine (annullamento, errore, lettura parziale): si chiude come interrotto.
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.end(true);
    }
}

impl FtpProvider {
    /// Un comando sulla connessione libera.
    fn with<T>(&self, f: impl FnOnce(&mut Ftp) -> Result<T, AppError>) -> Result<T, AppError> {
        let mut s = self.ctl.free()?;
        let r = f(&mut s.ftp);
        s.note(r)
    }

    /// Come `with`, per un comando che cambia il server: l'elenco ricordato si dimentica prima (le verifiche di
    /// esistenza di `rename` e `mkdir` rileggono il server: un'altra sessione può aver creato la destinazione) e dopo,
    /// anche se il comando fallisce.
    fn change<T>(&self, f: impl FnOnce(&mut Ftp) -> Result<T, AppError>) -> Result<T, AppError> {
        self.with(|ftp| {
            self.last.clear();
            let r = f(ftp);
            self.last.clear();
            r
        })
    }

    /// Apre uno stream, che tiene la connessione fino alla sua chiusura.
    fn stream(&self, path: &str, f: impl FnOnce(&mut Ftp) -> Result<Data, AppError>) -> Result<Stream, AppError> {
        let mut s = self.ctl.free()?;
        let r = f(&mut s.ftp);
        let data = s.note(r)?;
        s.busy = true;
        Ok(Stream { data: Some(data), ctl: Arc::clone(&self.ctl), path: path.to_owned() })
    }

    /// `CWD` prima di elencare: una cartella che non c'è dà errore anche dove `LIST` darebbe un elenco vuoto, e il
    /// percorso non passa come argomento di `LIST` (che alcuni server leggono come opzioni o caratteri jolly).
    fn list_in(&self, ftp: &mut Ftp, dir: &str) -> Result<Vec<Stat>, AppError> {
        ftp.cwd(dir).map_err(fe(dir))?;
        // Come `list`/`mlsd` di `suppaftp`, ma con un limite a quanto si legge.
        let cmd = if self.feat.mlst { "MLSD" } else { "LIST" };
        let (_, mut data) =
            ftp.custom_data_command(cmd, &[Status::AboutToSend, Status::AlreadyOpen]).map_err(fe(dir))?;
        let lines = read_lines(&mut data, dir);
        // Interrotto (`TOO_LARGE`, errore): il server risponde 226 o 426, una risposta sola in ogni caso.
        let done = match data.finish() {
            Err(FtpError::UnexpectedResponse(_)) if lines.is_err() => Ok(()),
            r => r,
        };
        let lines = lines?;
        done.map_err(fe(dir))?;
        let mut out: Vec<Stat> = lines.iter().filter_map(|l| parse_line(l, self.feat.mlst)).collect();
        for s in out.iter_mut().filter(|s| s.dir_link) {
            let full = join(dir, &s.name);
            if checked(&full).is_ok() && soft(ftp.cwd(&full)).map_err(fe(dir))?.is_some() {
                s.size = 0;
            } else {
                s.is_dir = false;
                s.dir_link = false;
                if self.feat.size
                    && checked(&full).is_ok()
                    && let Some(n) = soft(ftp.size(&full)).map_err(fe(dir))?
                {
                    s.size = n as u64;
                }
            }
        }
        self.last.put(dir, &out);
        Ok(out)
    }

    /// Un file si riconosce con `MLST` o `SIZE` (che seguono i collegamenti); cartelle e collegamenti solo
    /// nell'elenco della cartella madre. Con `MDTM` la data di un file è al secondo.
    /// Senza `MLST`, un elemento che non c'è o una cartella costano un elenco della cartella madre, riusato per
    /// `LISTING_TTL` finché nessun comando di questo provider cambia il server.
    /// ponytail: un caricamento cambia il server, quindi ogni file caricato in una cartella costa ancora un elenco;
    /// aggiornare l'elenco ricordato al posto di dimenticarlo, se serve. Un cambiamento fatto da un'altra sessione
    /// non si vede per `LISTING_TTL`: la verifica di `create_new` in `writer` può guardare un elenco vecchio di
    /// qualche secondo (non era atomica neanche prima); `rename` e `mkdir` rileggono sempre (vedi `change`).
    fn stat_in(&self, ftp: &mut Ftp, path: &str) -> Result<Stat, AppError> {
        let name = name_of(path).to_owned();
        if name.is_empty() {
            return Ok(Stat { name, is_dir: true, dir_link: false, size: 0, modified_ms: None, mode: None });
        }
        let found = if self.feat.mlst {
            let line = ftp.mlst(Some(path)).map_err(fe(path))?;
            parse_line(&line, true).filter(|s| !s.is_dir)
        } else if self.feat.size
            && let Some(n) = soft(ftp.size(path)).map_err(fe(path))?
        {
            Some(Stat { name: String::new(), is_dir: false, dir_link: false, size: n as u64, modified_ms: None, mode: None })
        } else {
            None
        };
        let mut s = match found {
            Some(s) => Stat { name, ..s },
            None => {
                let parent = parent_of(path);
                let found = match self.last.find(parent, &name) {
                    Some(found) => found,
                    None => self.list_in(ftp, parent)?.into_iter().find(|s| s.name == name),
                };
                found.ok_or_else(|| AppError::new(ErrorCode::NotFound, Some(Path::new(path))))?
            }
        };
        if !s.is_dir
            && self.feat.mdtm
            && !self.feat.mlst
            && let Some(t) = soft(ftp.mdtm(path)).map_err(fe(path))?
        {
            s.modified_ms = Some(t.and_utc().timestamp_millis());
        }
        Ok(s)
    }

    fn exists(&self, ftp: &mut Ftp, path: &str) -> Result<bool, AppError> {
        match self.stat_in(ftp, path) {
            Ok(_) => Ok(true),
            Err(e) if e.code == ErrorCode::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }
}

impl Provider for FtpProvider {
    /// `RNFR`/`RNTO` sopra un file esistente non ha un comportamento garantito: le sovrascritture passano dal
    /// fallback confermato. Le date sono precise solo con `MDTM`.
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: false, precise_mtime: self.feat.mdtm, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let p = checked(path)?;
        self.with(|ftp| self.stat_in(ftp, p))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        let p = checked(path)?;
        self.with(|ftp| self.list_in(ftp, p))
    }

    /// `REST` e `RETR`, poi la connessione dati si chiude dopo `len` byte.
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let p = checked(path)?;
        self.with(|ftp| {
            ftp.resume_transfer(offset as usize).map_err(fe(path))?;
            let mut data = ftp.retr_as_stream(p).map_err(fe(path))?;
            let mut buf = Vec::with_capacity(len as usize);
            let read = (&mut data).take(u64::from(len)).read_to_end(&mut buf);
            // Finito prima di `len`: la risposta dice se il file è arrivato intero. Altrimenti si chiude prima della
            // fine e il server risponde 226 o 426: una risposta sola in ogni caso.
            let early = read.is_err() || buf.len() == len as usize;
            let done = match data.finish() {
                Err(FtpError::UnexpectedResponse(_)) if early => Ok(()),
                r => r,
            };
            read.map_err(|e| io_error(e, path))?;
            done.map_err(fe(path))?;
            Ok(buf)
        })
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let p = checked(path)?;
        Ok(Box::new(self.stream(path, |ftp| ftp.retr_as_stream(p).map_err(fe(path)))?))
    }

    /// Il writer va concluso con `flush`, che restituisce l'esito del caricamento.
    /// ponytail: con `create_new` la verifica e `STOR` non sono atomiche (FTP non ha la creazione esclusiva).
    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let p = checked(path)?;
        Ok(Box::new(self.stream(path, |ftp| {
            if create_new && self.exists(ftp, p)? {
                return Err(AppError::new(ErrorCode::AlreadyExists, Some(Path::new(path))));
            }
            let r = ftp.put_with_stream(p).map_err(fe(path));
            self.last.clear();
            r
        })?))
    }

    /// Non sostituisce mai: una destinazione esistente dà `ALREADY_EXISTS` e resta intatta (vedi `caps`).
    fn rename(&self, from: &str, to: &str, _replace: bool) -> Result<(), AppError> {
        let (f, t) = (checked(from)?, checked(to)?);
        if from == to {
            return Ok(());
        }
        self.change(|ftp| {
            if self.exists(ftp, t)? {
                return Err(AppError::new(ErrorCode::AlreadyExists, Some(Path::new(to))));
            }
            ftp.rename(f, t).map_err(fe(from))
        })
    }

    /// Una cartella si prova prima con `DELE`, che toglie un collegamento a cartella senza toccare il bersaglio e su
    /// una cartella vera fallisce; poi `RMD`. Mai ricorsione.
    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let p = checked(path)?;
        self.change(|ftp| {
            match ftp.rm(p) {
                Err(FtpError::UnexpectedResponse(_)) if is_dir => ftp.rmdir(p),
                r => r,
            }
            .map_err(fe(path))
        })
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        let p = checked(path)?;
        self.change(|ftp| {
            let Err(e) = ftp.mkdir(p) else { return Ok(()) };
            match self.exists(ftp, p) {
                Ok(true) => Err(AppError::new(ErrorCode::AlreadyExists, Some(Path::new(path)))),
                Err(x) if dropped(&x) => Err(x),
                _ => Err(ftp_error(e, Some(path))),
            }
        })
    }

    /// La data con `MFMT`, se il server lo ha; altrimenti resta quella del caricamento. I permessi no: con
    /// `atomic_replace` falso un file sostituito si riscrive sul posto e tiene i suoi.
    fn set_meta(&self, path: &str, modified_ms: Option<i64>, _mode: Option<u32>) -> Result<(), AppError> {
        let p = checked(path)?;
        let Some(t) = modified_ms.filter(|_| self.feat.mfmt).and_then(chrono::DateTime::from_timestamp_millis) else {
            return Ok(());
        };
        let cmd = format!("MFMT {} {p}", t.format("%Y%m%d%H%M%S"));
        self.change(|ftp| ftp.custom_command(cmd, &[Status::File]).map(|_| ()).map_err(fe(path)))
    }

    fn is_broken(&self) -> bool {
        lock(&self.ctl.state).broken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftp_list_parsing() {
        let p = |l: &str| parse_line(l, false).unwrap();
        let s = p("drwxr-xr-x 2 u g 4096 Oct 21 10:00 logs");
        assert_eq!((s.name.as_str(), s.is_dir, s.dir_link, s.size, s.mode), ("logs", true, false, 0, Some(0o755)));
        // Senza anno: quest'anno o il precedente, al minuto (10:00).
        let ms = s.modified_ms.unwrap();
        assert_eq!((ms % 60_000, ms / 60_000 % (24 * 60)), (0, 600));
        let s = p("-rw-r--r-- 1 u g 12 Jan 3 2024 a.txt");
        let expected = Stat { name: "a.txt".into(), is_dir: false, dir_link: false, size: 12, modified_ms: Some(1_704_240_000_000), mode: Some(0o644) };
        assert_eq!(s, expected);
        // Un collegamento resta in sospeso come collegamento a cartella: `list` lo verifica con `CWD`.
        let s = p("lrwxrwxrwx 1 u g 10 Oct 21 10:00 current -> releases/1");
        assert_eq!((s.name.as_str(), s.is_dir, s.dir_link), ("current", true, true));
        let s = p("10-21-26  10:00AM <DIR> logs");
        let expected = Stat { name: "logs".into(), is_dir: true, dir_link: false, size: 0, modified_ms: Some(1_792_576_800_000), mode: None };
        assert_eq!(s, expected);
        let s = p("10-21-26  02:05PM 1,234 b.txt");
        assert_eq!((s.name.as_str(), s.is_dir, s.size, s.modified_ms), ("b.txt", false, 1234, Some(1_792_591_500_000)));
        // `.` e `..` e le righe che non si capiscono (`total 8`) non sono elementi.
        assert!(parse_line("drwxr-xr-x 2 u g 4096 Oct 21 10:00 .", false).is_none());
        assert!(parse_line("total 8", false).is_none());

        // MLSD: date al secondo, permessi solo se ci sono.
        let m = |l: &str| parse_line(l, true);
        let expected = Stat { name: "c d.txt".into(), is_dir: false, dir_link: false, size: 5, modified_ms: Some(1_792_576_805_000), mode: Some(0o640) };
        assert_eq!(m("type=file;size=5;modify=20261021100005;UNIX.mode=0640; c d.txt").unwrap(), expected);
        assert_eq!(m("type=dir;modify=20261021100005; sub").unwrap().mode, None);
        assert!(m("type=cdir;modify=20261021100005; /home/u").is_none());
        assert!(m("type=pdir;modify=20261021100005; ..").is_none());
        // Collegamenti di ProFTPD e Pure-FTPd: in sospeso come le voci `l`.
        for l in ["type=OS.unix=slink:/srv/releases/1;modify=20261021100005;UNIX.mode=0777; current", "Type=OS.unix=symlink; current"] {
            let s = m(l).unwrap();
            assert_eq!((s.name.as_str(), s.is_dir, s.dir_link), ("current", true, true), "{l}");
        }
    }

    #[test]
    fn listing_lines_are_capped() {
        let lines = read_lines(&b"total 8\r\n\r\n-rw-r--r-- 1 u g 1 Oct 21 10:00 a\n caf\xe9"[..], "/d").unwrap();
        assert_eq!(lines, ["total 8", "-rw-r--r-- 1 u g 1 Oct 21 10:00 a", " caf\u{FFFD}"]);
        let many = "a\n".repeat(crate::net::MAX_ENTRIES);
        assert_eq!(read_lines(many.as_bytes(), "/d").unwrap().len(), crate::net::MAX_ENTRIES);
        let too_many = many + "a\n";
        assert_eq!(read_lines(too_many.as_bytes(), "/d").unwrap_err().code, ErrorCode::TooLarge);
        // Una riga senza fine non riempie la memoria.
        let endless = io::repeat(b'x').take(LISTING_BYTES + 10);
        assert_eq!(read_lines(endless, "/d").unwrap_err().code, ErrorCode::TooLarge);
    }

    #[test]
    fn ftp_reply_mapping() {
        assert_eq!(reply_code(530, "530 Login incorrect."), ErrorCode::AuthFailed);
        assert_eq!(reply_code(550, "550 Failed to open file."), ErrorCode::NotFound);
        assert_eq!(reply_code(550, "550 a.txt: Permission denied"), ErrorCode::AccessDenied);
        assert_eq!(reply_code(553, "553 Could not create file."), ErrorCode::AccessDenied);
        assert_eq!(reply_code(552, "552 Quota exceeded"), ErrorCode::DiskFull);
        assert_eq!(reply_code(421, "421 Timeout."), ErrorCode::ConnectionFailed);
        assert_eq!(reply_code(502, "502 Command not implemented."), ErrorCode::Unsupported);
        assert_eq!(reply_code(451, "451 Local error."), ErrorCode::Io);
    }

    /// `MC_TEST_FTP` e/o `MC_TEST_FTPS` = `host:porta:utente:password:cartella_scrivibile` (la password senza `:`;
    /// FTPS implicito sulla porta 990). Le impronte vanno in una cartella temporanea, mai nel `known_hosts` vero.
    /// Niente `assert_eq!` che possa stampare la password.
    #[test]
    #[ignore = "richiede un server FTP/FTPS (MC_TEST_FTP, MC_TEST_FTPS)"]
    fn ftp_live() {
        let vars = [("MC_TEST_FTP", false), ("MC_TEST_FTPS", true)];
        let set: Vec<_> = vars.iter().filter_map(|&(k, tls)| std::env::var(k).ok().map(|v| (v, tls))).collect();
        assert!(!set.is_empty(), "MC_TEST_FTP o MC_TEST_FTPS non impostate");
        for (v, tls) in set {
            live(&v, tls);
        }
    }

    fn live(v: &str, tls: bool) {
        use crate::net::trust;
        use crate::vfs::loc::Scheme;
        let parts: Vec<&str> = v.splitn(5, ':').collect();
        let [host, port, user, password, dir] = parts[..] else { panic!("host:porta:utente:password:cartella") };
        let port: u16 = port.parse().expect("porta");
        let conn = Connection {
            id: "live".into(),
            name: "live".into(),
            scheme: if tls { Scheme::Ftps } else { Scheme::Ftp },
            host: host.into(),
            port,
            user: user.into(),
            start_dir: dir.into(),
            ftps_implicit: tls && (port == 990 || std::env::var_os("MC_TEST_FTPS_IMPLICIT").is_some()),
        };
        let cfg = tempfile::tempdir().unwrap();
        let code = |r: Result<FtpProvider, AppError>| r.err().map(|e| e.code);

        // Un certificato che Windows non accetta (autofirmato): errore con l'impronta, che si accetta.
        if tls && let Some(e) = open_in(cfg.path(), &conn, Some(password), true).err() {
            assert_eq!(e.code, ErrorCode::CertUntrusted);
            let fp = e.detail.unwrap();
            assert!(fp.starts_with("SHA256:"));
            trust::accept_in(cfg.path(), "tls", host, port, &fp).unwrap();
        }
        if !user.eq_ignore_ascii_case("anonymous") {
            assert_eq!(code(open_in(cfg.path(), &conn, None, tls)), Some(ErrorCode::AuthRequired));
            assert_eq!(code(open_in(cfg.path(), &conn, Some("mc-password-sbagliata"), tls)), Some(ErrorCode::AuthFailed));
        }
        let p = open_in(cfg.path(), &conn, Some(password), tls).map_err(|e| e.code).unwrap();

        let base = join(dir, &format!("mc-ftp-live-{}", uuid::Uuid::new_v4().simple()));
        p.mkdir(&base).unwrap();
        assert_eq!(p.mkdir(&base).unwrap_err().code, ErrorCode::AlreadyExists);
        let a = join(&base, "a.txt");
        let b = join(&base, "b.txt");
        let put = |path: &str, bytes: &[u8]| {
            let mut w = p.writer(path, true).unwrap();
            w.write_all(bytes).unwrap();
            w.flush().unwrap();
        };
        put(&a, b"0123456789");
        assert_eq!(p.writer(&a, true).err().unwrap().code, ErrorCode::AlreadyExists);

        // `REST` e chiusura anticipata: la connessione resta usabile dopo ogni lettura.
        assert_eq!(p.read_at(&a, 2, 3).unwrap(), b"234");
        assert_eq!(p.read_at(&a, 8, 100).unwrap(), b"89");
        assert_eq!(p.read_at(&a, 0, 1).unwrap(), b"0");
        let mut all = Vec::new();
        p.reader(&a).unwrap().read_to_end(&mut all).unwrap();
        assert_eq!(all, b"0123456789");
        let mut one = [0u8; 1];
        p.reader(&a).unwrap().read_exact(&mut one).unwrap();
        let s = p.stat(&a).unwrap();
        assert_eq!((s.name.as_str(), s.is_dir, s.size), ("a.txt", false, 10));
        if p.feat.mfmt && p.feat.mdtm {
            p.set_meta(&a, Some(1_000_000_000_000), None).unwrap();
            assert_eq!(p.stat(&a).unwrap().modified_ms, Some(1_000_000_000_000));
        }
        let names: Vec<String> = p.list(&base).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["a.txt"]);

        // Rinomina: una destinazione esistente non si tocca mai, anche con `replace`.
        put(&b, b"b");
        assert_eq!(p.rename(&a, &b, false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.rename(&a, &b, true).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"b");
        p.delete(&b, false).unwrap();
        p.rename(&a, &b, false).unwrap();
        assert_eq!(p.stat(&a).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(p.read_at(&b, 0, 10).unwrap(), b"0123456789");

        // Una sottocartella si elimina con `delete(_, true)`; la cartella si elenca solo se esiste.
        let sub = join(&base, "sub");
        p.mkdir(&sub).unwrap();
        assert!(p.stat(&sub).unwrap().is_dir);
        p.delete(&sub, true).unwrap();
        p.delete(&b, false).unwrap();
        p.delete(&base, true).unwrap();
        assert_eq!(p.list(&base).unwrap_err().code, ErrorCode::NotFound);
    }
}
