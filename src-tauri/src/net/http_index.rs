//! Indici HTTP/HTTPS (Apache, nginx, IIS) in sola lettura, con `ureq`.
//!
//! L'URL di base è `<schema>://<host>:<porta>`; il percorso del provider è il percorso POSIX sotto di esso. L'HTML
//! dell'indice è testo non fidato: `parse_index` ne estrae solo nomi, dimensioni e date dei figli diretti, e non
//! arriva mai al frontend.
//!
//! HTTPS usa la `ClientConfig` di `net::tls` (archivio di Windows, poi impronte accettate): un certificato
//! rifiutato dà `CERT_UNTRUSTED` con l'impronta. I redirect non si seguono mai (un `Location` verso un altro host
//! riceverebbe le credenziali): un `3xx` dà `IO` con lo stato e la destinazione in `detail`.
//!
//! `stat` legge l'elenco della cartella madre, così cartelle e file hanno gli stessi metadati di `list`.
//! ponytail: si ricorda solo l'ultimo elenco, per pochi secondi: `stat` alternati su cartelle diverse rielencano
//! ogni volta; `HEAD` o una cache per cartella se serve.
//! ponytail: le date dell'indice non dicono il fuso (Apache usa l'ora locale del server, nginx l'UTC): si leggono
//! come UTC, al minuto; le date di IIS si capiscono solo nel formato inglese.
//! ponytail: `ureq` scrive le intestazioni senza oscurarle (`Authorization` compresa) solo nel log a livello trace,
//! che nelle build di rilascio è tolto (`log` con `release_max_level_debug`); come per `suppaftp`.

use super::connections::Connection;
use super::http_auth::Credentials;
use super::sessions::ProviderFactory;
use super::sftp::{CONNECT_TIMEOUT, IO_TIMEOUT_MS};
use super::tls::Slot;
use crate::error::{AppError, ErrorCode};
use crate::vfs::provider::{Caps, Provider, Stat};
use chrono::NaiveDate;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use ureq::config::Config;
use ureq::http::Response;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::time::{Duration as UDuration, Instant as UInstant};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, TcpConnector, Transport, TransportAdapter,
};
use ureq::{Agent, Body, BodyReader};

/// Limite dell'HTML di un indice.
const INDEX_LIMIT: u64 = 16 << 20;

/// Per quanto `stat` riusa l'ultimo elenco letto.
const LISTING_TTL: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------------------------------------------
// Indice: funzioni pure su testo non fidato (mai un panic).

/// I segmenti non vuoti di un percorso POSIX.
fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// `%XX` in byte, poi UTF-8 (U+FFFD dove non lo è). Un `%` senza due cifre esadecimali resta com'è.
pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |j: usize| b.get(j).copied().and_then(hex_val);
        match (b[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = name.strip_prefix('#')?;
            let v = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(v)
        }
    }
}

/// Le entità HTML di un attributo; una sconosciuta resta com'è.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match rest.find(';').filter(|&e| e <= 10).and_then(|e| entity(&rest[1..e]).map(|c| (c, e))) {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Le righe dell'indice: a capo, `<br>` (IIS mette tutto su una riga) e `<tr>`.
fn rows(html: &str) -> Vec<&str> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..b.len() {
        let tag = |t: &[u8]| b[i + 1..].get(..2).is_some_and(|s| s.eq_ignore_ascii_case(t));
        if (b[i] == b'\n' || (b[i] == b'<' && (tag(b"br") || tag(b"tr")))) && i > start {
            out.push(&html[start..i]);
            start = i;
        }
    }
    out.push(&html[start..]);
    out
}

/// Il valore di `href` in un tag (`a href="x"`), tra virgolette, apici o senza.
fn href(tag: &str) -> Option<&str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("href").map(|i| from + i) {
        from = i + 4;
        if !lower[..i].ends_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let Some(v) = tag[i + 4..].trim_start().strip_prefix('=') else { continue };
        let v = v.trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let v = &v[1..];
                &v[..v.find(q).unwrap_or(v.len())]
            }
            _ => &v[..v.find(|c: char| c.is_ascii_whitespace()).unwrap_or(v.len())],
        });
    }
    None
}

/// Gli `href` dei link di una riga e il testo fuori dai link (dove stanno data e dimensione).
fn scan(row: &str) -> (Vec<&str>, String) {
    let (mut hrefs, mut text, mut in_a) = (Vec::new(), String::new(), false);
    let mut rest = row;
    while let Some(lt) = rest.find('<') {
        if !in_a {
            text.push_str(&rest[..lt]);
        }
        text.push(' ');
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { return (hrefs, text) };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        let name = tag.split(|c: char| c.is_ascii_whitespace()).next().unwrap_or_default();
        if name.eq_ignore_ascii_case("a") {
            in_a = true;
            hrefs.extend(href(tag));
        } else if name.eq_ignore_ascii_case("/a") {
            in_a = false;
        }
    }
    if !in_a {
        text.push_str(rest);
    }
    (hrefs, text)
}

/// Il figlio diretto di `dir` a cui punta `href` (nome decodificato, cartella sì/no), se lo è. Fuori: query,
/// frammenti soli, altri schemi e host (`http:`, `//host`), `..`, altre cartelle, nomi non validi.
fn child(href: &str, dir: &[&str]) -> Option<(String, bool)> {
    // Come i browser: spazi e controlli ai bordi tolti, tab e a capo interni ignorati.
    let href = decode_entities(href);
    let href: String = href.trim_matches(|c: char| c <= ' ').chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    let href = href.split('#').next().unwrap_or_default();
    if href.is_empty() || href.contains('?') || href.starts_with("//") || href.split('/').next()?.contains(':') {
        return None;
    }
    let mut segs: Vec<String> = match href.starts_with('/') {
        true => Vec::new(),
        false => dir.iter().map(|s| (*s).to_owned()).collect(),
    };
    for raw in href.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if raw == ".." {
            return None;
        }
        segs.push(percent_decode(raw));
    }
    let name = segs.pop()?;
    if segs.len() != dir.len() || segs.iter().zip(dir).any(|(a, b)| a != b) {
        return None;
    }
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return None;
    }
    Some((name, href.ends_with('/')))
}

fn month(s: &str) -> Option<u32> {
    let m = s.get(..3)?.to_ascii_lowercase();
    let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    names.iter().position(|n| *n == m).map(|i| i as u32 + 1)
}

/// `H:MM` o `HH:MM[:SS]` (i secondi si ignorano).
fn time(s: &str) -> Option<(u32, u32)> {
    let mut p = s.split(':');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

fn utc_ms(y: i32, mo: u32, d: u32, (h, mi): (u32, u32)) -> Option<i64> {
    Some(NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, 0)?.and_utc().timestamp_millis())
}

/// Una data all'inizio di `t` e quante parole occupa: Apache `2026-10-21 10:00`, nginx `21-Oct-2026 10:00`,
/// IIS `Wednesday, October 21, 2026 10:00 AM`.
fn date_at(t: &[&str]) -> Option<(i64, usize)> {
    let first = *t.first()?;
    if let [a, b, c] = first.split('-').collect::<Vec<_>>()[..] {
        let tm = time(t.get(1)?)?;
        let ms = match a.len() {
            4 => utc_ms(a.parse().ok()?, b.parse().ok()?, c.parse().ok()?, tm),
            _ => utc_ms(c.parse().ok()?, month(b)?, a.parse().ok()?, tm),
        };
        return ms.map(|ms| (ms, 2));
    }
    let [_, mo, d, y, tm, ampm, ..] = t else { return None };
    if !first.ends_with(',') {
        return None;
    }
    let (h, mi) = time(tm)?;
    let h = match ampm.to_ascii_uppercase().as_str() {
        "AM" => h % 12,
        "PM" => h % 12 + 12,
        _ => return None,
    };
    let ms = utc_ms(y.parse().ok()?, month(mo)?, d.strip_suffix(',')?.parse().ok()?, (h, mi))?;
    Some((ms, 6))
}

/// `1234`, oppure `1.2K`, `3M`, `2G` (potenze di 1024); `-` e `<dir>` non sono dimensioni.
fn size_of(s: &str) -> Option<u64> {
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic())?);
    let exp = match unit.to_ascii_uppercase().trim_end_matches("IB").trim_end_matches('B') {
        "K" => 1,
        "M" => 2,
        "G" => 3,
        "T" => 4,
        _ => return None,
    };
    let v = num.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)?;
    Some((v * 1024f64.powi(exp)).round() as u64)
}

/// Data e dimensione dal testo di una riga: la dimensione è la parola subito dopo la data.
fn meta(text: &str) -> (Option<i64>, u64) {
    let t: Vec<&str> = text.split_whitespace().collect();
    (0..t.len())
        .find_map(|i| date_at(&t[i..]).map(|(ms, n)| (Some(ms), t.get(i + n).and_then(|s| size_of(s)).unwrap_or(0))))
        .unwrap_or((None, 0))
}

/// I figli diretti di `dir_path` elencati nell'HTML di un indice, nell'ordine della pagina e senza doppioni.
pub fn parse_index(html: &str, dir_path: &str) -> Vec<Stat> {
    let dir = segments(dir_path);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in rows(html) {
        let (hrefs, text) = scan(row);
        let (modified_ms, size) = meta(&text);
        for (name, is_dir) in hrefs.into_iter().filter_map(|h| child(h, &dir)) {
            if seen.insert(name.clone()) {
                out.push(Stat { name, is_dir, dir_link: false, size: if is_dir { 0 } else { size }, modified_ms, mode: None });
            }
        }
    }
    out
}

/// L'URL di `path` sull'origine di `base` (schema, host e porta): ogni segmento in percent-encoding, tranne i
/// caratteri non riservati. Una barra finale resta.
pub fn url_for(base: &str, path: &str) -> String {
    let host_start = base.find("://").map_or(0, |i| i + 3);
    let origin = &base[..base[host_start..].find('/').map_or(base.len(), |i| host_start + i)];
    let mut url = origin.to_owned();
    for seg in segments(path) {
        url.push('/');
        for b in seg.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => url.push(b as char),
                _ => url.push_str(&format!("%{b:02X}")),
            }
        }
    }
    if url.len() == origin.len() || path.ends_with('/') {
        url.push('/');
    }
    url
}

// ---------------------------------------------------------------------------------------------------------------
// Trasporto HTTPS con la configurazione TLS di `net::tls`.

/// Il `RustlsConnector` di `ureq` costruisce da sé la sua `ClientConfig` e non ne accetta una data: questo lo
/// ricalca con quella di `net::tls`. Il nome del server è l'host della connessione (i redirect non si seguono).
/// ponytail: l'API dei trasporti di `ureq` è `unversioned` (fuori da semver): va ricontrollata a ogni aggiornamento.
#[derive(Debug)]
struct Tls {
    config: Arc<ClientConfig>,
    name: ServerName<'static>,
}

impl<In: Transport> Connector<In> for Tls {
    type Out = Either<In, TlsTransport>;

    fn connect(&self, details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(t) = chained else { return Ok(None) };
        if !details.needs_tls() || t.is_tls() {
            return Ok(Some(Either::A(t)));
        }
        let mut conn = ClientConnection::new(Arc::clone(&self.config), self.name.clone())?;
        let mut sock = TransportAdapter::new(t.boxed(), details.current_time.clone());
        // Il tempo della connessione TCP conta: TCP più handshake restano entro il timeout di connessione.
        let mut timeout = details.timeout;
        if let (UDuration::Exact(after), UInstant::Exact(start), UInstant::Exact(now)) =
            (timeout.after, details.now, (details.current_time)())
        {
            timeout.after = UDuration::Exact(after.saturating_sub(now.saturating_duration_since(start)));
        }
        sock.set_timeout(timeout);
        conn.complete_io(&mut sock)?;
        let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
        Ok(Some(Either::B(TlsTransport { buffers, stream: StreamOwned::new(conn, sock) })))
    }
}

struct TlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl std::fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsTransport")
    }
}

impl Transport for TlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.sock.set_timeout(timeout);
        self.stream.write_all(&self.buffers.output()[..amount])?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.sock.set_timeout(timeout);
        let n = self.stream.read(self.buffers.input_append_buf())?;
        self.buffers.input_appended(n);
        Ok(n > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.sock.get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Provider.

fn agent(tls: Option<Tls>) -> Agent {
    let io = Some(Duration::from_millis(u64::from(IO_TIMEOUT_MS)));
    let config = Config::builder()
        .http_status_as_error(false)
        .max_redirects(0)
        // Mai un proxy dalle variabili d'ambiente: la catena dei connettori va diretta al server.
        .proxy(None)
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(io)
        .timeout_per_write(io)
        .build();
    let tcp = ().chain(TcpConnector::default());
    match tls {
        Some(t) => Agent::with_parts(config, tcp.chain(t), DefaultResolver::default()),
        None => Agent::with_parts(config, tcp, DefaultResolver::default()),
    }
}

/// Il codice di un errore di I/O, anche quando porta dentro un errore di `ureq` (un timeout di lettura del corpo).
fn io_code(e: &io::Error) -> ErrorCode {
    match e.get_ref().and_then(|x| x.downcast_ref::<ureq::Error>()) {
        Some(e) => ureq_code(e),
        None if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => ErrorCode::Timeout,
        None => ErrorCode::ConnectionFailed,
    }
}

fn ureq_code(e: &ureq::Error) -> ErrorCode {
    match e {
        ureq::Error::Timeout(_) => ErrorCode::Timeout,
        ureq::Error::Io(e) => io_code(e),
        ureq::Error::BodyExceedsLimit(_) => ErrorCode::TooLarge,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
            ErrorCode::ConnectionFailed
        }
        _ => ErrorCode::Io,
    }
}

fn io_error(e: io::Error, path: &str) -> AppError {
    AppError { detail: Some(e.to_string()), ..AppError::new(io_code(&e), Some(Path::new(path))) }
}

fn read_only(path: &str) -> AppError {
    AppError::new(ErrorCode::ReadOnlySource, Some(Path::new(path)))
}

/// Una risposta `2xx`, o l'errore del suo stato (`401` lo tratta `get`).
fn ok(resp: Response<Body>, path: &str) -> Result<Response<Body>, AppError> {
    let status = resp.status().as_u16();
    let code = match status {
        200..=299 => return Ok(resp),
        404 | 410 => ErrorCode::NotFound,
        403 => ErrorCode::AccessDenied,
        _ => ErrorCode::Io,
    };
    let location = resp.headers().get("location").and_then(|v| v.to_str().ok());
    let detail = match location {
        Some(l) if (300..400).contains(&status) => format!("HTTP {status} {l}"),
        _ => format!("HTTP {status}"),
    };
    Err(AppError { detail: Some(detail), ..AppError::new(code, Some(Path::new(path))) })
}

/// `tls`: HTTPS.
pub struct HttpFactory {
    pub tls: bool,
}

impl ProviderFactory for HttpFactory {
    fn open(&self, conn: &Connection, password: Option<&str>) -> Result<Arc<dyn Provider>, AppError> {
        Ok(Arc::new(open_in(&crate::config::current_config_dir()?, conn, password, self.tls)?))
    }
}

/// Niente `Debug`: le credenziali contengono la password.
pub(crate) struct HttpProvider {
    agent: Agent,
    /// `<schema>://<host>:<porta>`, senza barra finale.
    base: String,
    /// `None`: utente vuoto, nessuna autenticazione (un `401` dà `AUTH_REQUIRED`).
    creds: Option<Credentials>,
    /// HTTPS: l'impronta di un certificato rifiutato (vedi `net::tls`).
    slot: Option<Slot>,
    /// L'ultima richiesta con `Range` ha avuto `206`.
    ranged: AtomicBool,
    /// L'ultimo elenco letto (cartella, quando, voci): `stat` lo riusa per `LISTING_TTL`, così la copia di una
    /// cartella (uno `stat` per file) non riscarica l'indice a ogni file. `list` lo rinnova sempre.
    last: Mutex<Option<(String, Instant, Vec<Stat>)>>,
}

/// Prepara il client e prova cartella iniziale, certificato e credenziali con una richiesta `Range: bytes=0-0`.
/// Utente senza password: `AUTH_REQUIRED` subito.
pub(crate) fn open_in(dir: &Path, conn: &Connection, password: Option<&str>, tls: bool) -> Result<HttpProvider, AppError> {
    let creds = match (conn.user.is_empty(), password) {
        (true, _) => None,
        (false, Some(p)) => Some(Credentials::new(&conn.user, p)),
        (false, None) => return Err(AppError::new(ErrorCode::AuthRequired, None)),
    };
    let host = if conn.host.contains(':') { format!("[{}]", conn.host) } else { conn.host.clone() };
    let base = format!("{}://{host}:{}", if tls { "https" } else { "http" }, conn.port);
    let (agent, slot) = if tls {
        let (config, slot) = super::tls::client_config_in(dir, &conn.host, conn.port)?;
        let name = ServerName::try_from(conn.host.as_str()).map(|n| n.to_owned()).map_err(|e| AppError {
            detail: Some(e.to_string()),
            ..AppError::new(ErrorCode::InvalidRequest, None)
        })?;
        (agent(Some(Tls { config, name })), Some(slot))
    } else {
        (agent(None), None)
    };
    let p = HttpProvider { agent, base, creds, slot, ranged: AtomicBool::new(false), last: Mutex::new(None) };
    let start = if conn.start_dir.is_empty() { "/" } else { conn.start_dir.as_str() };
    let resp = p.get(start, true, Some((0, 1)))?;
    p.ranged.store(resp.status().as_u16() == 206, Ordering::Relaxed);
    ok(resp, start)?;
    Ok(p)
}

impl HttpProvider {
    fn last(&self) -> std::sync::MutexGuard<'_, Option<(String, Instant, Vec<Stat>)>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Un errore di `ureq`; un certificato rifiutato diventa `CERT_UNTRUSTED` con l'impronta.
    fn error(&self, e: ureq::Error, path: &str) -> AppError {
        if let Some(fp) = self.slot.as_ref().and_then(|s| s.lock().unwrap_or_else(|e| e.into_inner()).take()) {
            return AppError { detail: Some(fp), ..AppError::new(ErrorCode::CertUntrusted, None) };
        }
        AppError { detail: Some(e.to_string()), ..AppError::new(ureq_code(&e), Some(Path::new(path))) }
    }

    /// `GET` di `path` (una cartella con la barra finale), con `Range` se c'è. Un `401` si ripete una volta con lo
    /// schema che il server chiede: senza credenziali `AUTH_REQUIRED`, rifiutate `AUTH_FAILED`. Ogni altro stato
    /// torna a chi chiama.
    fn get(&self, path: &str, dir: bool, range: Option<(u64, u32)>) -> Result<Response<Body>, AppError> {
        if path.contains('\u{FFFD}') {
            return Err(AppError::new(ErrorCode::InvalidName, Some(Path::new(path))));
        }
        let mut url = url_for(&self.base, path);
        if dir && !url.ends_with('/') {
            url.push('/');
        }
        let uri = &url[self.base.len()..];
        let with_path = |e: AppError| AppError { path: Some(path.to_owned()), ..e };
        let mut retried = false;
        loop {
            let auth = match &self.creds {
                Some(c) => c.header("GET", uri)?,
                None => None,
            };
            let mut req = self.agent.get(&url);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            if let Some((from, len)) = range {
                req = req.header("Range", format!("bytes={from}-{}", from + u64::from(len.max(1)) - 1));
            }
            let resp = req.call().map_err(|e| self.error(e, path))?;
            if resp.status().as_u16() != 401 {
                return Ok(resp);
            }
            let Some(c) = &self.creds else { return Err(with_path(AppError::new(ErrorCode::AuthRequired, None))) };
            if retried {
                return Err(with_path(AppError::new(ErrorCode::AuthFailed, None)));
            }
            let values: Vec<&str> =
                resp.headers().get_all("www-authenticate").iter().filter_map(|v| v.to_str().ok()).collect();
            c.challenge(&values, auth.is_some()).map_err(with_path)?;
            retried = true;
        }
    }
}

/// Il corpo di una risposta; gli errori portano l'`AppError` dentro `io::Error::other`.
struct Stream {
    body: BodyReader<'static>,
    path: String,
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.body.read(buf).map_err(|e| io::Error::other(io_error(e, &self.path)))
    }
}

impl Provider for HttpProvider {
    /// `ranged_read`: l'ultima richiesta con `Range` ha avuto `206` (nginx non la onora sull'indice della prova,
    /// la prima lettura di un file la aggiorna).
    fn caps(&self) -> Caps {
        Caps { writable: false, atomic_replace: false, precise_mtime: false, ranged_read: self.ranged.load(Ordering::Relaxed) }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let (parent, name) = path.trim_end_matches('/').rsplit_once('/').unwrap_or(("", ""));
        if name.is_empty() {
            return Ok(Stat { name: String::new(), is_dir: true, dir_link: false, size: 0, modified_ms: None, mode: None });
        }
        let parent = if parent.is_empty() { "/" } else { parent };
        let cached = match &*self.last() {
            Some((dir, at, items)) if dir == parent && at.elapsed() < LISTING_TTL => {
                Some(items.iter().find(|s| s.name == name).cloned())
            }
            _ => None,
        };
        let found = match cached {
            Some(found) => found,
            None => self.list(parent)?.into_iter().find(|s| s.name == name),
        };
        found.ok_or_else(|| AppError::new(ErrorCode::NotFound, Some(Path::new(path))))
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        let mut resp = ok(self.get(path, true, None)?, path)?;
        let html = resp.body_mut().with_config().limit(INDEX_LIMIT).read_to_vec().map_err(|e| self.error(e, path))?;
        let items = parse_index(&String::from_utf8_lossy(&html), path);
        let dir = match path.trim_end_matches('/') {
            "" => "/",
            d => d,
        };
        *self.last() = Some((dir.to_owned(), Instant::now(), items.clone()));
        Ok(items)
    }

    /// `206` con l'intervallo chiesto, altrimenti `RANGE_UNSUPPORTED`. Oltre la fine (`416`): niente.
    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let resp = self.get(path, false, Some((offset, len)))?;
        if resp.status().as_u16() == 416 {
            return Ok(Vec::new());
        }
        let mut resp = ok(resp, path)?;
        let start = format!("bytes {offset}-");
        let range = resp.headers().get("content-range").and_then(|v| v.to_str().ok());
        let partial = resp.status().as_u16() == 206 && range.is_some_and(|r| r.starts_with(&start));
        self.ranged.store(partial, Ordering::Relaxed);
        if !partial {
            return Err(AppError::new(ErrorCode::RangeUnsupported, Some(Path::new(path))));
        }
        let mut buf = Vec::new();
        resp.body_mut().as_reader().take(u64::from(len)).read_to_end(&mut buf).map_err(|e| io_error(e, path))?;
        Ok(buf)
    }

    /// Il file intero, anche senza `Range`.
    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let resp = ok(self.get(path, false, None)?, path)?;
        Ok(Box::new(Stream { body: resp.into_body().into_reader(), path: path.to_owned() }))
    }

    fn writer(&self, path: &str, _create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        Err(read_only(path))
    }

    fn rename(&self, from: &str, _to: &str, _replace: bool) -> Result<(), AppError> {
        Err(read_only(from))
    }

    fn delete(&self, path: &str, _is_dir: bool) -> Result<(), AppError> {
        Err(read_only(path))
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        Err(read_only(path))
    }

    fn set_meta(&self, path: &str, _modified_ms: Option<i64>, _mode: Option<u32>) -> Result<(), AppError> {
        Err(read_only(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64, modified_ms: i64) -> Stat {
        Stat { name: name.into(), is_dir, dir_link: false, size, modified_ms: Some(modified_ms), mode: None }
    }

    #[test]
    fn parses_apache_nginx_iis() {
        // 2026-10-20 08:15, 2026-10-21 10:00, 11:30, 13:30 (UTC: l'indice non dice il fuso).
        let (d20, d21, d21b, d21c) = (1_792_484_100_000, 1_792_576_800_000, 1_792_582_200_000, 1_792_589_400_000);
        let apache = parse_index(include_str!("fixtures/apache.html"), "/logs");
        assert_eq!(
            apache,
            [
                entry("archive", true, 0, d20),
                entry("app.log", false, 1229, d21), // 1.2K
                entry("a&b.txt", false, 17, d21b),
                entry("my report è.pdf", false, 3_565_158, d21c), // 3.4M
            ]
        );
        let nginx = parse_index(include_str!("fixtures/nginx.html"), "/logs/");
        assert_eq!(
            nginx,
            [
                entry("archive", true, 0, d20),
                entry("app.log", false, 1024, d21), // 1K
                entry("a-very-long-file-name-that-nginx-truncates.txt", false, 17, d21b),
                entry("my report è.pdf", false, 3 << 20, d21c), // 3M
            ]
        );
        let iis = parse_index(include_str!("fixtures/iis.html"), "/logs");
        assert_eq!(
            iis,
            [
                entry("archive", true, 0, d20),
                entry("app.log", false, 1234, d21),
                entry("my report è.pdf", false, 3_567_616, d21c),
            ]
        );
    }

    #[test]
    fn index_ignores_foreign_links() {
        let html = r##"<pre><a href="../">../</a> <a href="/altro/">altro</a>
            <a href="http://altrohost/x">x</a><br><a href="?C=N;O=D">Name</a><tr><a href="#top">top</a>
            <a href=" ../">x</a><a href="	..
/">y</a><a href=" ?C=M;O=A">z</a>"##;
        assert_eq!(parse_index(html, "/logs/"), []);
    }

    #[test]
    fn http_url_encodes_segments() {
        assert_eq!(url_for("https://h/logs/", "/logs/my app/è#1.log"), "https://h/logs/my%20app/%C3%A8%231.log");
    }

    /// Nomi pericolosi scartati, `./a:b` di Apache e un `href` senza virgolette tenuti; testo strano senza panic.
    #[test]
    fn index_survives_odd_input() {
        let html = r#"<a href="%2e%2e/">x</a><a href="a%2Fb">x</a><a href="a%5Cb">x</a><a href="nul%00">x</a>
            <a href="sub/x.txt">x</a><a href='./a:b.txt'>a:b</a><a class=x HREF=plain.txt>p</a><a href="mailto:x@y">m</a>"#;
        let names: Vec<String> = parse_index(html, "/").into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["a:b.txt", "plain.txt"]);
        let pieces = ["<", ">", "<a", " href=", "\"", "'", "%", "%e", "&", "&#", "&#x110000;", ";", "è", "<br>", "\n", "2026-10-21", "10:00", "21-Oct-2026", "Monday,", "October", "21,", "PM", "99999999999999999999K", "1e308K", "/", "../", "a/", "-", "<tr>", "</a>"];
        let mut seed = 0x2545_f491_u32;
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..(seed % 24) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                s.push_str(pieces[seed as usize % pieces.len()]);
            }
            let _ = parse_index(&s, "/a/");
        }
    }

    type Reply = (u16, Vec<(&'static str, String)>, Vec<u8>);

    trait ReadWrite: Read + Write {}
    impl<T: Read + Write> ReadWrite for T {}

    /// Server HTTP(S) di prova su 127.0.0.1: una richiesta per connessione (`Connection: close`).
    fn serve(tls: Option<Arc<rustls::ServerConfig>>, handler: impl Fn(&str) -> Reply + Send + 'static) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s: Box<dyn ReadWrite> = match &tls {
                    Some(c) => Box::new(StreamOwned::new(rustls::ServerConnection::new(Arc::clone(c)).unwrap(), s)),
                    None => Box::new(s),
                };
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && matches!(s.read(&mut b), Ok(1)) {
                    head.push(b[0]);
                }
                if head.is_empty() {
                    continue; // handshake rifiutato dal client
                }
                let (status, headers, body) = handler(&String::from_utf8_lossy(&head));
                let mut out = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (k, v) in headers {
                    out += &format!("{k}: {v}\r\n");
                }
                out += "\r\n";
                let _ = s.write_all(out.as_bytes()).and_then(|()| s.write_all(&body)).and_then(|()| s.flush());
            }
        });
        port
    }

    fn header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
        req.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
    }

    fn conn(port: u16, user: &str) -> Connection {
        Connection {
            id: "t".into(),
            name: "t".into(),
            scheme: crate::vfs::loc::Scheme::Http,
            host: "127.0.0.1".into(),
            port,
            user: user.into(),
            start_dir: "/".into(),
            ftps_implicit: false,
        }
    }

    const FILE: &[u8] = b"0123456789";

    /// Basic: `/a.txt` onora `Range`, `/n.txt` no, il resto è un redirect verso un altro host.
    fn basic_site(req: &str) -> Reply {
        if header(req, "authorization") != Some("Basic dTpw") {
            return (401, vec![("WWW-Authenticate", r#"Basic realm="x""#.into())], Vec::new());
        }
        let index = "<pre><a href=\"a.txt\">a.txt</a> 2026-10-21 10:00 10\n<a href=\"n.txt\">n.txt</a>\n<a href=\"r/\">r/</a>";
        let range = header(req, "range").and_then(|r| r.strip_prefix("bytes=")?.split_once('-'));
        match req.split(' ').nth(1).unwrap_or_default() {
            "/" => (200, Vec::new(), index.into()),
            "/a.txt" => match range.map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())) {
                Some((a, _)) if a >= FILE.len() => (416, Vec::new(), Vec::new()),
                Some((a, b)) => {
                    let b = b.min(FILE.len() - 1);
                    (206, vec![("Content-Range", format!("bytes {a}-{b}/{}", FILE.len()))], FILE[a..=b].to_vec())
                }
                None => (200, Vec::new(), FILE.to_vec()),
            },
            "/n.txt" => (200, Vec::new(), FILE.to_vec()),
            _ => (301, vec![("Location", "http://altrohost/".into())], Vec::new()),
        }
    }

    #[test]
    fn http_provider_basic_range_and_errors() {
        let port = serve(None, basic_site);
        let dir = tempfile::tempdir().unwrap();
        let code = |user: &str, pw: Option<&str>| open_in(dir.path(), &conn(port, user), pw, false).err().map(|e| e.code);
        assert_eq!(code("u", None), Some(ErrorCode::AuthRequired));
        assert_eq!(code("", None), Some(ErrorCode::AuthRequired));
        assert_eq!(code("u", Some("sbagliata")), Some(ErrorCode::AuthFailed));
        let p = open_in(dir.path(), &conn(port, "u"), Some("p"), false).map_err(|e| e.code).unwrap();

        let names: Vec<String> = p.list("/").unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["a.txt", "n.txt", "r"]);
        let s = p.stat("/a.txt").unwrap();
        assert_eq!((s.is_dir, s.size, s.modified_ms), (false, 10, Some(1_792_576_800_000)));
        assert!(p.stat("/r/").unwrap().is_dir);
        assert_eq!(p.stat("/x").unwrap_err().code, ErrorCode::NotFound);

        assert_eq!(p.read_at("/a.txt", 2, 3).unwrap(), b"234");
        assert_eq!(p.read_at("/a.txt", 8, 100).unwrap(), b"89");
        assert_eq!(p.read_at("/a.txt", 10, 4).unwrap(), b"");
        assert!(p.caps().ranged_read);
        assert_eq!(p.read_at("/n.txt", 0, 4).unwrap_err().code, ErrorCode::RangeUnsupported);
        assert!(!p.caps().ranged_read);
        let mut all = Vec::new();
        p.reader("/n.txt").unwrap().read_to_end(&mut all).unwrap();
        assert_eq!(all, FILE);

        // Il redirect non si segue: errore con lo stato e la destinazione.
        let e = p.list("/r/").unwrap_err();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Io, Some("HTTP 301 http://altrohost/")));
        assert_eq!(p.writer("/b.txt", true).err().unwrap().code, ErrorCode::ReadOnlySource);
        assert_eq!(p.mkdir("/d").unwrap_err().code, ErrorCode::ReadOnlySource);
        let c = p.caps();
        assert_eq!((c.writable, c.atomic_replace, c.precise_mtime), (false, false, false));
    }

    /// `stat` di più file della stessa cartella: un solo elenco scaricato; `list` lo rinnova.
    #[test]
    fn http_stat_reuses_listing() {
        use std::sync::atomic::AtomicUsize;
        let fetches = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&fetches);
        let port = serve(None, move |req| match req.split(' ').nth(1).unwrap_or_default() {
            "/d/" => {
                f.fetch_add(1, Ordering::SeqCst);
                (200, Vec::new(), b"<a href=\"a\">a</a>\n<a href=\"b\">b</a>\n<a href=\"c\">c</a>".to_vec())
            }
            _ => (200, Vec::new(), Vec::new()),
        });
        let dir = tempfile::tempdir().unwrap();
        let p = open_in(dir.path(), &conn(port, ""), None, false).map_err(|e| e.code).unwrap();
        for name in ["a", "b", "c"] {
            assert_eq!(p.stat(&format!("/d/{name}")).unwrap().name, name);
        }
        assert_eq!(p.stat("/d/x").unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(p.list("/d/").unwrap().len(), 3);
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
    }

    /// Digest SHA-256 verificato dal server con `digest_response`; HTTPS con certificato autofirmato: rifiutato con
    /// l'impronta, poi accettato.
    #[test]
    fn http_provider_digest_over_pinned_tls() {
        use super::super::http_auth::{Algo, Challenge, digest_response};
        let key = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
        let der = rustls::pki_types::PrivatePkcs8KeyDer::from(key.signing_key.serialize_der());
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![key.cert.der().clone()], der.into())
            .unwrap();
        let port = serve(Some(Arc::new(server)), |req| {
            let ch = Challenge { realm: "r".into(), nonce: "n1".into(), opaque: None, algo: Algo::Sha256, sess: false, qop: true };
            let param = |a: &str, k: &str| {
                a.split(", ").find_map(|kv| kv.split_once('=').filter(|(n, _)| *n == k).map(|(_, v)| v.trim_matches('"').to_owned()))
            };
            let valid = header(req, "authorization").and_then(|a| a.strip_prefix("Digest ")).is_some_and(|a| {
                let p = |k| param(a, k).unwrap_or_default();
                let nc = u32::from_str_radix(&p("nc"), 16).unwrap_or_default();
                p("response") == digest_response(&ch, "u", "p", "GET", &p("uri"), nc, &p("cnonce")).unwrap()
            });
            let challenge = r#"Digest realm="r", nonce="n1", qop="auth", algorithm=SHA-256"#;
            match valid {
                true => (200, Vec::new(), b"<a href=\"x.txt\">x.txt</a>".to_vec()),
                false => (401, vec![("WWW-Authenticate", challenge.into())], Vec::new()),
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let e = open_in(dir.path(), &conn(port, "u"), Some("p"), true).err().unwrap();
        assert_eq!(e.code, ErrorCode::CertUntrusted);
        let fp = e.detail.unwrap();
        assert!(fp.starts_with("SHA256:"));
        super::super::trust::accept_in(dir.path(), "tls", "127.0.0.1", port, &fp).unwrap();
        let p = open_in(dir.path(), &conn(port, "u"), Some("p"), true).map_err(|e| e.code).unwrap();
        let names: Vec<String> = p.list("/").unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["x.txt"]);
        let wrong = open_in(dir.path(), &conn(port, "u"), Some("q"), true).err().map(|e| e.code);
        assert_eq!(wrong, Some(ErrorCode::AuthFailed));
    }

    /// `MC_TEST_HTTP` = `url:utente:password` (utente e password senza `:`, vuoti per un indice pubblico), per
    /// esempio `https://host:443/cartella/:utente:password`. Le impronte vanno in una cartella temporanea.
    /// Niente `assert_eq!` che possa stampare la password.
    #[test]
    #[ignore = "richiede un indice HTTP vero (MC_TEST_HTTP)"]
    fn http_live() {
        let v = std::env::var("MC_TEST_HTTP").expect("MC_TEST_HTTP non impostata");
        let mut parts = v.rsplitn(3, ':');
        let (password, user, url) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().expect("url:utente:password"));
        let tls = url.starts_with("https://");
        let rest = url.split_once("://").expect("url").1;
        let (authority, start_dir) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.trim_matches(['[', ']']), p.parse().expect("porta")),
            None => (authority, if tls { 443 } else { 80 }),
        };
        let start_dir = percent_decode(if start_dir.is_empty() { "/" } else { start_dir });
        let scheme = if tls { crate::vfs::loc::Scheme::Https } else { crate::vfs::loc::Scheme::Http };
        let conn = Connection { scheme, host: host.into(), start_dir: start_dir.clone(), ..conn(port, user) };
        let cfg = tempfile::tempdir().unwrap();
        let pw = (!password.is_empty()).then_some(password);
        if tls && let Some(e) = open_in(cfg.path(), &conn, pw, true).err() {
            assert_eq!(e.code, ErrorCode::CertUntrusted);
            super::super::trust::accept_in(cfg.path(), "tls", host, port, &e.detail.unwrap()).unwrap();
        }
        let code = |pw: Option<&str>| open_in(cfg.path(), &conn, pw, tls).err().map(|e| e.code);
        if !user.is_empty() {
            assert_eq!(code(None), Some(ErrorCode::AuthRequired));
            assert_eq!(code(Some("mc-password-sbagliata")), Some(ErrorCode::AuthFailed));
        }
        let p = open_in(cfg.path(), &conn, pw, tls).map_err(|e| e.code).unwrap();
        let items = p.list(&start_dir).unwrap();
        assert!(!items.is_empty(), "indice vuoto o non riconosciuto");
        if let Some(f) = items.iter().find(|s| !s.is_dir) {
            let path = format!("{}/{}", start_dir.trim_end_matches('/'), f.name);
            let mut all = Vec::new();
            p.reader(&path).unwrap().read_to_end(&mut all).unwrap();
            match p.read_at(&path, 0, 1) {
                Ok(b) => assert_eq!(b, all[..1.min(all.len())]),
                Err(e) => assert_eq!(e.code, ErrorCode::RangeUnsupported),
            }
        }
    }
}
