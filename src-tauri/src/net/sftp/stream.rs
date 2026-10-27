//! Letture e scritture SFTP con più richieste in volo, sulla sessione grezza: il `File` di russh-sftp perde i codici
//! di stato SFTP. Gli errori escono come `io::Error::other(AppError)` con il percorso (`vfs::provider::stream_error`).

use super::connect::Link;
use super::errors::sftp_error;
use super::rt;
use crate::error::{AppError, ErrorCode};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesOrdered;
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Data, Status, StatusCode};
use std::collections::VecDeque;
use std::io;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

/// Richieste in volo per lettura o scrittura, come i default di russh-sftp.
pub(super) const IN_FLIGHT: usize = 16;

type Reply = (u64, u32, Result<Data, SftpError>);

fn with_path(e: AppError, path: &str) -> AppError {
    AppError { path: Some(path.to_owned()), ..e }
}

/// Letture anticipate da `chunk` byte a offset crescenti fino a `end`, con i risultati in ordine. `S` è la sessione:
/// in prestito per `read_range`, condivisa per `Reader`.
struct Window<'a, S> {
    sftp: S,
    handle: String,
    chunk: u32,
    /// Il prossimo offset da chiedere.
    ahead: u64,
    end: u64,
    queue: FuturesOrdered<BoxFuture<'a, Reply>>,
}

impl<'a, S: Deref<Target = RawSftpSession> + Clone + Send + Sync + 'a> Window<'a, S> {
    fn new(sftp: S, handle: String, chunk: u32, offset: u64, end: u64) -> Self {
        Self { sftp, handle, chunk: chunk.max(1), ahead: offset, end, queue: FuturesOrdered::new() }
    }

    fn request(&self, offset: u64, len: u32) -> BoxFuture<'a, Reply> {
        let (sftp, handle) = (self.sftp.clone(), self.handle.clone());
        Box::pin(async move { (offset, len, sftp.read(handle, offset, len).await) })
    }

    /// I byte successivi; `None` a fine file (stato `EOF` o dati vuoti) o a `end`. Una risposta corta si completa con
    /// una richiesta del resto, davanti alle altre.
    async fn next(&mut self) -> Result<Option<Vec<u8>>, SftpError> {
        while self.queue.len() < IN_FLIGHT && self.ahead < self.end {
            let len = (self.end - self.ahead).min(self.chunk as u64) as u32;
            self.queue.push_back(self.request(self.ahead, len));
            self.ahead += len as u64;
        }
        let Some((offset, len, reply)) = self.queue.next().await else { return Ok(None) };
        let mut data = match reply {
            Ok(d) => d.data,
            Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => return Ok(None),
            Err(e) => return Err(e),
        };
        if data.is_empty() {
            return Ok(None);
        }
        data.truncate(len as usize);
        let got = data.len() as u32;
        if got < len {
            self.queue.push_front(self.request(offset + got as u64, len - got));
        }
        Ok(Some(data))
    }
}

/// `len` byte da `offset` (meno a fine file), con fino a `IN_FLIGHT` richieste da `chunk` byte in volo.
pub(super) async fn read_range(
    sftp: &RawSftpSession,
    handle: &str,
    offset: u64,
    len: u32,
    chunk: u32,
) -> Result<Vec<u8>, AppError> {
    let mut window = Window::new(sftp, handle.to_owned(), chunk, offset, offset.saturating_add(len as u64));
    let mut out = Vec::new();
    while let Some(data) = window.next().await.map_err(|e| sftp_error(e, None))? {
        out.extend_from_slice(&data);
    }
    Ok(out)
}

/// Lettura sequenziale con letture anticipate; ogni `read` che chiede dati al server aspetta al massimo `io_timeout`.
pub(super) struct Reader {
    /// Tiene viva la connessione.
    link: Arc<Link>,
    path: String,
    io_timeout: Duration,
    window: Window<'static, Arc<RawSftpSession>>,
    data: Vec<u8>,
    pos: usize,
    eof: bool,
    /// Il primo errore, ripetuto: il blocco fallito non si richiede più, quindi una lettura successiva salterebbe i
    /// suoi byte.
    error: Option<AppError>,
}

impl Reader {
    pub(super) fn new(link: Arc<Link>, handle: String, path: String, chunk: u32, io_timeout: Duration) -> Self {
        let window = Window::new(link.sftp.clone(), handle, chunk, 0, u64::MAX);
        Self { link, path, io_timeout, window, data: Vec::new(), pos: 0, eof: false, error: None }
    }
}

impl io::Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.data.len() {
            if let Some(e) = &self.error {
                return Err(io::Error::other(e.clone()));
            }
            if self.eof {
                return Ok(0);
            }
            let window = &mut self.window;
            match rt::block(self.io_timeout, async { window.next().await.map_err(|e| sftp_error(e, None)) }) {
                Ok(Some(data)) => (self.data, self.pos) = (data, 0),
                Ok(None) => self.eof = true,
                Err(e) => self.error = Some(with_path(e, &self.path)),
            }
        }
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Drop for Reader {
    /// Chiude l'handle sul runtime, senza aspettare; le letture in volo si abbandonano.
    fn drop(&mut self) {
        let (link, handle) = (self.link.clone(), std::mem::take(&mut self.window.handle));
        rt::spawn(async move {
            let _ = link.sftp.close(handle).await;
        });
    }
}

/// Scrittura sequenziale a blocchi da `chunk` byte, fino a `IN_FLIGHT` in volo: un errore del server arriva in un
/// `write` successivo o in `flush`, che aspetta tutte le conferme e chiude l'handle.
pub(super) struct Writer {
    /// Tiene viva la connessione.
    link: Arc<Link>,
    handle: String,
    path: String,
    chunk: usize,
    io_timeout: Duration,
    buf: Vec<u8>,
    offset: u64,
    /// Le conferme delle scritture inviate, in ordine.
    in_flight: VecDeque<BoxFuture<'static, Result<Status, SftpError>>>,
    /// Il primo errore, ripetuto da ogni `write` e `flush`.
    error: Option<AppError>,
    closed: bool,
}

impl Writer {
    pub(super) fn new(link: Arc<Link>, handle: String, path: String, chunk: u32, io_timeout: Duration) -> Self {
        let chunk = chunk.max(1) as usize;
        Self {
            link,
            handle,
            path,
            chunk,
            io_timeout,
            buf: Vec::with_capacity(chunk),
            offset: 0,
            in_flight: VecDeque::new(),
            error: None,
            closed: false,
        }
    }

    /// Ricorda l'errore (il primo) e lo restituisce.
    fn fail(&mut self, e: AppError) -> io::Error {
        io::Error::other(self.error.get_or_insert(with_path(e, &self.path)).clone())
    }

    fn wait_oldest(&mut self) -> Result<(), AppError> {
        let Some(reply) = self.in_flight.pop_front() else { return Ok(()) };
        rt::block(self.io_timeout, async move { reply.await.map(drop).map_err(|e| sftp_error(e, None)) })
    }

    /// Manda un blocco, subito e dopo i precedenti; con `IN_FLIGHT` blocchi in volo prima aspetta il più vecchio.
    fn send(&mut self, data: Vec<u8>) -> Result<(), AppError> {
        if self.in_flight.len() >= IN_FLIGHT {
            self.wait_oldest()?;
        }
        let (sftp, handle, offset) = (self.link.sftp.clone(), self.handle.clone(), self.offset);
        self.offset += data.len() as u64;
        self.in_flight.push_back(rt::start(async move { sftp.write(handle, offset, data).await })?);
        Ok(())
    }

    /// Il resto del buffer, tutte le conferme, la chiusura.
    fn finish(&mut self) -> Result<(), AppError> {
        if !self.buf.is_empty() {
            let last = std::mem::take(&mut self.buf);
            self.send(last)?;
        }
        while !self.in_flight.is_empty() {
            self.wait_oldest()?;
        }
        let (sftp, handle) = (self.link.sftp.clone(), self.handle.clone());
        rt::block(self.io_timeout, async move { sftp.close(handle).await.map(drop).map_err(|e| sftp_error(e, None)) })?;
        self.closed = true;
        Ok(())
    }
}

impl io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(e) = &self.error {
            return Err(io::Error::other(e.clone()));
        }
        if self.closed {
            return Err(io::Error::other(with_path(AppError::new(ErrorCode::InvalidRequest, None), &self.path)));
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            let n = (self.chunk - self.buf.len()).min(rest.len());
            self.buf.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            if self.buf.len() == self.chunk {
                let block = std::mem::replace(&mut self.buf, Vec::with_capacity(self.chunk));
                if let Err(e) = self.send(block) {
                    return Err(self.fail(e));
                }
            }
        }
        Ok(bytes.len())
    }

    /// Dopo un errore lo ripete e lascia la chiusura a `Drop`; dopo un `flush` riuscito non fa nulla.
    fn flush(&mut self) -> io::Result<()> {
        if let Some(e) = &self.error {
            return Err(io::Error::other(e.clone()));
        }
        if self.closed {
            return Ok(());
        }
        self.finish().map_err(|e| self.fail(e))
    }
}

impl Drop for Writer {
    /// Senza `flush` riuscito il resto del buffer si perde. La chiusura parte subito, dopo le scritture già inviate e
    /// prima di ogni richiesta successiva (per esempio la cancellazione del temporaneo, che su alcuni server con
    /// l'handle aperto fallirebbe); né la sua risposta né quelle delle scritture si aspettano.
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let (sftp, handle) = (self.link.sftp.clone(), std::mem::take(&mut self.handle));
        let _ = rt::start(async move { sftp.close(handle).await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::net::sftp::connect::Connected;
    use crate::net::sftp::errors::sftp_error;
    use crate::net::sftp::fake::{self, FakeOptions};
    use crate::vfs::provider::stream_error;
    use russh_sftp::protocol::{FileAttributes, OpenFlags};
    use std::io::{Read, Write};
    use std::time::Instant;

    const IO: Duration = Duration::from_secs(10);

    /// 600 KiB pseudo-casuali: più di `IN_FLIGHT` blocchi da 32 KiB.
    fn data() -> Vec<u8> {
        let mut x = 0x2545_f491_u32;
        (0..600 * 1024)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn open(c: &Connected, path: &str, flags: OpenFlags) -> String {
        let (sftp, path) = (c.link.sftp.clone(), path.to_owned());
        let open = async move { sftp.open(path, flags, FileAttributes::default()).await.map_err(|e| sftp_error(e, None)) };
        rt::block(IO, open).unwrap().handle
    }

    fn reader(c: &Connected, path: &str) -> Reader {
        Reader::new(c.link.clone(), open(c, path, OpenFlags::READ), path.into(), c.chunk, IO)
    }

    fn writer(c: &Connected, path: &str, io_timeout: Duration) -> Writer {
        let h = open(c, path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE);
        Writer::new(c.link.clone(), h, path.into(), c.chunk, io_timeout)
    }

    fn range_by(c: &Connected, handle: &str, offset: u64, len: u32, chunk: u32) -> Vec<u8> {
        let (sftp, h) = (c.link.sftp.clone(), handle.to_owned());
        rt::block(IO, async move { read_range(&sftp, &h, offset, len, chunk).await }).unwrap()
    }

    fn range(c: &Connected, handle: &str, offset: u64, len: u32) -> Vec<u8> {
        range_by(c, handle, offset, len, c.chunk)
    }

    /// Le chiusure di `Drop` girano sul runtime: si aspetta (poco) che il finto non abbia più handle aperti.
    fn all_closed(fake: &fake::FakeServer) -> bool {
        let t = Instant::now();
        while fake.open_handles() > 0 && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        fake.open_handles() == 0
    }

    fn read_all(opts: FakeOptions, host: &str) {
        let (fake, c) = fake::connected(host, opts);
        let d = data();
        fake.put("/f", &d);
        let mut out = Vec::new();
        reader(&c, "/f").read_to_end(&mut out).unwrap();
        assert!(out == d, "{} byte letti su {}", out.len(), d.len());
    }

    #[test]
    fn reader_reads_everything_in_order() {
        read_all(FakeOptions::default(), "reader.test");
    }

    #[test]
    fn reader_handles_short_reads_in_order() {
        read_all(FakeOptions { short_reads: true, ..Default::default() }, "short-reads.test");
    }

    #[test]
    fn read_range_from_offset() {
        let (fake, c) = fake::connected("range.test", FakeOptions::default());
        let d = data();
        fake.put("/f", &d);
        let h = open(&c, "/f", OpenFlags::READ);
        assert!(range(&c, &h, 100_000, 70_000) == d[100_000..170_000]);
        assert_eq!(range(&c, &h, d.len() as u64 - 10, 100), d[d.len() - 10..]);
        // Un blocco da 0 byte vale 1: mai richieste vuote scambiate per la fine del file.
        assert_eq!(range_by(&c, &h, 5, 3, 0), d[5..8]);
    }

    #[test]
    fn empty_file_and_read_past_end() {
        let (fake, c) = fake::connected("empty.test", FakeOptions::default());
        fake.put("/empty", b"");
        assert_eq!(reader(&c, "/empty").read(&mut [0; 16]).unwrap(), 0);
        fake.put("/f", b"abc");
        let h = open(&c, "/f", OpenFlags::READ);
        assert!(range(&c, &h, 3, 100).is_empty());
        assert!(range(&c, &h, 1000, 100).is_empty());
    }

    #[test]
    fn writer_writes_everything_and_flush_closes() {
        let (fake, c) = fake::connected("writer.test", FakeOptions::default());
        let d = data();
        let mut w = writer(&c, "/out", IO);
        for part in d.chunks(8 * 1024) {
            w.write_all(part).unwrap();
        }
        w.flush().unwrap();
        assert!(fake.get("/out") == Some(d));
        assert_eq!(fake.open_handles(), 0);
        w.flush().unwrap();
        assert_eq!(stream_error(w.write(b"x").unwrap_err(), "/out").code, ErrorCode::InvalidRequest);
    }

    /// I blocchi partono in ordine di offset: un server che scrive dove è arrivato (qui: rifiuta ogni altro offset)
    /// riceve il file intero.
    #[test]
    fn writes_leave_in_offset_order() {
        let (fake, c) = fake::connected("ordered-writes.test", FakeOptions { sequential_writes: true, ..Default::default() });
        let d = data();
        for _ in 0..5 {
            let mut w = writer(&c, "/out", IO);
            w.write_all(&d).unwrap();
            w.flush().unwrap();
            assert!(fake.get("/out") == Some(d.clone()));
        }
    }

    /// `Drop` senza `flush` accoda la chiusura prima di tornare: una richiesta successiva (per esempio la
    /// cancellazione del temporaneo) trova l'handle già chiuso.
    #[test]
    fn dropped_writer_closes_before_the_next_request() {
        let (fake, c) = fake::connected("drop-close-order.test", FakeOptions::default());
        let mut w = writer(&c, "/out", IO);
        w.write_all(&data()).unwrap();
        drop(w);
        let sftp = c.link.sftp.clone();
        rt::block(IO, async move { sftp.lstat("/out").await.map_err(|e| sftp_error(e, None)) }).unwrap();
        assert_eq!(fake.open_handles(), 0);
    }

    #[test]
    fn write_error_surfaces_with_its_code() {
        let (_fake, c) = fake::connected("write-error.test", FakeOptions::default());
        let mut w = writer(&c, "/ro/x", IO);
        // Le scritture in volo: l'errore arriva in un `write` successivo o in `flush`.
        let e = data().chunks(8 * 1024).try_for_each(|p| w.write_all(p)).and_then(|_| w.flush()).unwrap_err();
        let e = stream_error(e, "altro");
        assert_eq!((e.code, e.path.as_deref()), (ErrorCode::AccessDenied, Some("/ro/x")));
        assert_eq!(stream_error(w.flush().unwrap_err(), "").code, ErrorCode::AccessDenied);
        assert_eq!(stream_error(w.write(b"x").unwrap_err(), "").code, ErrorCode::AccessDenied);
    }

    /// Sotto `/ro` l'apertura con `CREATE | TRUNCATE` riesce ma non cambia nulla: l'errore arriva da `write`.
    #[test]
    fn read_only_open_changes_nothing() {
        let (fake, c) = fake::connected("ro-open.test", FakeOptions::default());
        fake.put("/ro/f", b"abc");
        drop(writer(&c, "/ro/f", IO));
        drop(writer(&c, "/ro/new", IO));
        assert_eq!(fake.get("/ro/f").as_deref(), Some(&b"abc"[..]));
        assert_eq!(fake.get("/ro/new"), None);
    }

    #[test]
    fn dropped_reader_and_writer_close_their_handles() {
        let (fake, c) = fake::connected("drop-close.test", FakeOptions::default());
        fake.put("/f", &data());
        let mut r = reader(&c, "/f");
        r.read_exact(&mut [0; 1000]).unwrap();
        let mut w = writer(&c, "/out", IO);
        w.write_all(&data()).unwrap();
        assert_eq!(fake.open_handles(), 2);
        drop((r, w));
        assert!(all_closed(&fake), "{} handle aperti", fake.open_handles());
    }

    #[test]
    fn writer_on_stalled_server_times_out() {
        let (fake, c) = fake::connected("stalled-writer.test", FakeOptions::default());
        let mut w = writer(&c, "/out", Duration::from_millis(300));
        fake.stall();
        let block = vec![7; c.chunk as usize];
        let t = Instant::now();
        let e = (0..=IN_FLIGHT).try_for_each(|_| w.write_all(&block)).unwrap_err();
        assert_eq!(stream_error(e, "").code, ErrorCode::Timeout);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        let t = Instant::now();
        drop(w);
        assert!(t.elapsed() < Duration::from_millis(100), "{:?}", t.elapsed());
    }

    #[test]
    fn killed_connection_is_connection_failed() {
        let (fake, c) = fake::connected("killed-reader.test", FakeOptions::default());
        fake.put("/f", &data());
        let mut r = reader(&c, "/f");
        r.read_exact(&mut [0; 64 * 1024]).unwrap();
        fake.kill();
        // Le letture anticipate coprono al massimo `IN_FLIGHT` blocchi: il resto del file serve una richiesta nuova.
        let e = std::io::copy(&mut r, &mut std::io::sink()).unwrap_err();
        assert_eq!(stream_error(e, "").code, ErrorCode::ConnectionFailed);
    }
}
