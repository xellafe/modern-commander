//! Lettore a canale: chi produce i byte (un decoder che sa solo scrivere, o che vive in un altro thread) scrive in
//! un canale limitato e il lettore li riceve come `Read`. La memoria resta di pochi blocchi qualunque sia la voce.

use crate::error::{AppError, ErrorCode};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

/// Blocco del canale.
const CHUNK: usize = 64 * 1024;
/// Blocchi in attesa nel canale.
const DEPTH: usize = 4;

enum Msg {
    Data(Vec<u8>),
    /// Fine regolare: senza, la chiusura del canale è un'interruzione.
    Done,
    Fail(AppError),
}

pub(super) struct PipeWriter {
    tx: SyncSender<Msg>,
    buf: Vec<u8>,
    closed: Arc<AtomicBool>,
}

pub(super) struct PipeReader {
    rx: Receiver<Msg>,
    chunk: Vec<u8>,
    at: usize,
    done: bool,
    closed: Arc<AtomicBool>,
}

/// Le due estremità di un canale vuoto.
pub(super) fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = sync_channel(DEPTH);
    let closed = Arc::new(AtomicBool::new(false));
    let w = PipeWriter { tx, buf: Vec::with_capacity(CHUNK), closed: closed.clone() };
    (w, PipeReader { rx, chunk: Vec::new(), at: 0, done: false, closed })
}

/// `f` gira in un thread e scrive nel canale; il suo errore arriva al lettore dentro `io::Error::other` (vedi
/// `stream_error`). Chiuso il lettore, la scrittura successiva fallisce e il thread finisce.
pub fn piped(f: impl FnOnce(&mut dyn Write) -> Result<(), AppError> + Send + 'static) -> Box<dyn Read + Send> {
    let (mut w, r) = pipe();
    std::thread::spawn(move || match f(&mut w) {
        Ok(()) => w.finish(),
        Err(e) => w.fail(e),
    });
    Box::new(r)
}

fn broken() -> io::Error {
    io::ErrorKind::BrokenPipe.into()
}

impl PipeWriter {
    /// Il lettore è stato chiuso: inutile continuare a produrre.
    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn send_buf(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let full = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK));
            self.tx.send(Msg::Data(full)).map_err(|_| broken())?;
        }
        Ok(())
    }

    /// Fine regolare dei dati.
    pub(super) fn finish(mut self) {
        if self.send_buf().is_ok() {
            let _ = self.tx.send(Msg::Done);
        }
    }

    /// I dati si interrompono con `e`, dopo quelli già scritti.
    pub(super) fn fail(mut self, e: AppError) {
        if self.send_buf().is_ok() {
            let _ = self.tx.send(Msg::Fail(e));
        }
    }
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.is_closed() {
            return Err(broken());
        }
        let n = data.len().min(CHUNK - self.buf.len());
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() == CHUNK {
            self.send_buf()?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_buf()
    }
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at == self.chunk.len() {
            if self.done || out.is_empty() {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(Msg::Data(d)) => (self.chunk, self.at) = (d, 0),
                Ok(Msg::Done) => self.done = true,
                Ok(Msg::Fail(e)) => return Err(io::Error::other(e)),
                // Chi scriveva è finito senza dirlo: i dati sono incompleti.
                Err(_) => {
                    let e = AppError { detail: Some("interrupted".into()), ..AppError::new(ErrorCode::Io, None) };
                    return Err(io::Error::other(e));
                }
            }
        }
        let n = out.len().min(self.chunk.len() - self.at);
        out[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::provider::stream_error;
    use std::time::Duration;

    #[test]
    fn pipe_propagates_errors_and_stops_on_drop() {
        let mut r = piped(|w| {
            w.write_all(b"abc").unwrap();
            Err(AppError::new(ErrorCode::NotFound, None))
        });
        let mut got = Vec::new();
        let e = r.read_to_end(&mut got).unwrap_err();
        assert_eq!((stream_error(e, "x").code, got.as_slice()), (ErrorCode::NotFound, &b"abc"[..]));

        // Una voce "infinita": chiuso il lettore dopo il primo blocco, il thread deve finire.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut r = piped(move |w| {
            let block = vec![7u8; CHUNK];
            let r = loop {
                if let Err(e) = w.write_all(&block) {
                    break e;
                }
            };
            tx.send(r.kind()).unwrap();
            Ok(())
        });
        let mut first = vec![0u8; CHUNK];
        r.read_exact(&mut first).unwrap();
        drop(r);
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn writer_dropped_without_finish_is_an_error() {
        let (mut w, mut r) = pipe();
        w.write_all(b"partial").unwrap();
        w.flush().unwrap();
        drop(w);
        let mut got = Vec::new();
        assert_eq!(stream_error(r.read_to_end(&mut got).unwrap_err(), "x").code, ErrorCode::Io);
        assert_eq!(got, b"partial");
    }
}
