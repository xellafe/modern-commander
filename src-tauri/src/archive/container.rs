//! Il file contenitore di un archivio, locale o remoto, letto come `Read + Seek`. È l'unico punto da cui si apre
//! un contenitore locale (un `Loc::Archive` non arriva mai a `std::fs`); il remoto si legge con `read_at` del suo
//! provider. Ogni clone ha posizione e lettura anticipata sue: indice, voci e cursori leggono in modo indipendente.

use crate::error::{AppError, ErrorCode};
use crate::fs::content::open_shared;
use crate::vfs::loc::{Loc, Scheme};
use crate::vfs::provider::{Connector, Purpose};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Lettura anticipata: le intestazioni di ZIP e 7z si leggono a pezzi piccoli e vicini.
pub(super) const READ_AHEAD: usize = 256 * 1024;

/// Lettura anticipata dopo un salto: un'intestazione letta a salti (inizio e fine di un 7z, intestazione locale di
/// una voce ZIP) non trascina con sé `READ_AHEAD` byte; chi legge di seguito torna subito a `READ_AHEAD`.
const JUMP_AHEAD: usize = 64 * 1024;

enum Source {
    Local(std::fs::File),
    /// Il provider si chiede al connettore a ogni lettura: nessuna sessione resta occupata dalla cache.
    Remote { connector: Arc<dyn Connector>, scheme: Scheme, conn: String, path: String },
}

struct Shared {
    source: Source,
    /// Dimensione all'apertura: la cache rilegge l'indice se il contenitore cambia.
    len: u64,
    /// Forma testuale della posizione, per gli errori.
    name: PathBuf,
}

#[derive(Clone)]
pub struct ContainerFile {
    shared: Arc<Shared>,
    pos: u64,
    /// Finestra letta in anticipo e suo inizio; un clone la condivide finché non ne legge un'altra.
    buf: Arc<[u8]>,
    buf_at: u64,
}

pub fn open(connector: &Arc<dyn Connector>, container: &Loc) -> Result<ContainerFile, AppError> {
    match container {
        Loc::Local(p) => open_local(p),
        Loc::Remote { scheme, conn, path } => {
            let len = connector.provider(*scheme, conn, Purpose::Transfer)?.stat(path)?.size;
            let source = Source::Remote { connector: connector.clone(), scheme: *scheme, conn: conn.clone(), path: path.clone() };
            Ok(ContainerFile::new(source, len, container.to_string()))
        }
        Loc::Archive { .. } => Err(container.error(ErrorCode::InvalidRequest)),
    }
}

/// Contenitore locale (anche per l'ispettore, che non passa dal connettore).
pub(super) fn open_local(path: &Path) -> Result<ContainerFile, AppError> {
    let err = |e: io::Error| AppError::from_io(&e, path);
    let f = open_shared(path).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    Ok(ContainerFile::new(Source::Local(f), len, path.to_string_lossy().into_owned()))
}

impl ContainerFile {
    fn new(source: Source, len: u64, name: String) -> Self {
        Self { shared: Arc::new(Shared { source, len, name: name.into() }), pos: 0, buf: Arc::from([]), buf_at: 0 }
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.shared.len
    }

    /// Posizione del contenitore per i messaggi d'errore.
    pub(super) fn name(&self) -> &Path {
        &self.shared.name
    }

    /// Fino a `len` byte da `offset`, più corto solo a fine file. Gli errori del provider viaggiano dentro
    /// `io::Error::other` (vedi `list::io_err`).
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        match &self.shared.source {
            Source::Local(f) => {
                let mut buf = vec![0; len];
                let mut n = 0;
                while n < len {
                    match f.seek_read(&mut buf[n..], offset + n as u64) {
                        Ok(0) => break,
                        Ok(k) => n += k,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(e),
                    }
                }
                buf.truncate(n);
                Ok(buf)
            }
            Source::Remote { connector, scheme, conn, path } => connector
                .provider(*scheme, conn, Purpose::Transfer)
                .and_then(|p| p.read_at(path, offset, len as u32))
                .map_err(io::Error::other),
        }
    }
}

impl Read for ContainerFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !(self.buf_at..self.buf_at + self.buf.len() as u64).contains(&self.pos) {
            if out.is_empty() || self.pos >= self.shared.len {
                return Ok(0);
            }
            let sequential = !self.buf.is_empty() && self.pos == self.buf_at + self.buf.len() as u64;
            let ahead = if sequential { READ_AHEAD } else { JUMP_AHEAD };
            let want = (self.shared.len - self.pos).min(ahead as u64) as usize;
            self.buf = self.read_at(self.pos, want)?.into();
            self.buf_at = self.pos;
            if self.buf.is_empty() {
                return Ok(0); // accorciato dopo l'apertura
            }
        }
        let start = (self.pos - self.buf_at) as usize;
        let n = out.len().min(self.buf.len() - start);
        out[..n].copy_from_slice(&self.buf[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for ContainerFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.shared.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = to.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::mem::MemConnector;
    use std::collections::HashMap;
    use std::io::{Read, Seek, SeekFrom};

    #[test]
    fn container_clones_read_independently() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.bin");
        let data: Vec<u8> = (0..(READ_AHEAD * 3) as u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let connector: Arc<dyn Connector> = Arc::new(MemConnector(HashMap::new()));
        let mut a = open(&connector, &Loc::Local(p)).unwrap();
        assert_eq!(a.len(), data.len() as u64);
        let mut b = a.clone();
        b.seek(SeekFrom::Start(READ_AHEAD as u64 * 2 + 7)).unwrap();
        let (mut at_a, mut at_b) = (0usize, READ_AHEAD * 2 + 7);
        let mut buf = [0u8; 1000];
        // Letture alternate: ognuna deve ritrovare la sua posizione e la sua finestra.
        for _ in 0..50 {
            let n = a.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], &data[at_a..at_a + n]);
            at_a += n;
            let n = b.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], &data[at_b..at_b + n]);
            at_b += n;
        }
        let mut rest = Vec::new();
        b.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, &data[at_b..]);
    }
}
