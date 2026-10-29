//! Cursori dei formati a flusso (TAR.GZ, 7z): il decoder dell'ultima voce letta resta aperto, così leggere le voci
//! nell'ordine d'archivio (vedi `Provider::stream_order`) decomprime ogni blocco una volta sola. Uno slot per
//! archivio in cache; una voce più indietro, o in un altro blocco, riparte dall'inizio.

use super::container::ContainerFile;
use super::list::{corrupt, io_err};
use super::pipe::{PipeWriter, pipe};
use super::tar_walk::{TarWalker, skip_read};
use crate::error::{AppError, ErrorCode};
use flate2::read::MultiGzDecoder;
use sevenz_rust2::{Archive, BlockDecoder, EncoderMethod, Password};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Un decoder 7z senza richieste per tanto si chiude.
const IDLE: Duration = Duration::from_secs(30);

pub(super) type Slot = Arc<Mutex<Option<Cursor>>>;

type GzWalker = TarWalker<MultiGzDecoder<ContainerFile>>;

pub(super) enum Cursor {
    /// Il flusso è subito dopo i dati della voce `next - 1`.
    TarGz { walker: Box<GzWalker>, next: u64 },
    SevenZ(Decoder),
}

/// Errore di lettura di una voce, per lo stream: quello del provider del contenitore com'è, gli altri tradotti.
pub(super) fn to_io(e: io::Error, name: &Path) -> io::Error {
    if e.get_ref().is_some_and(|e| e.is::<AppError>()) { e } else { io::Error::other(io_err(&e, name)) }
}

/// La voce `order` di un TAR.GZ. Dal cursore se è più avanti, altrimenti decomprimendo dall'inizio.
pub(super) fn targz(slot: &Slot, file: &ContainerFile, order: u64, name: PathBuf) -> Result<Box<dyn Read + Send>, AppError> {
    let taken = slot.lock().unwrap().take();
    let (mut walker, mut next) = match taken {
        Some(Cursor::TarGz { walker, next }) if next <= order => (walker, next),
        _ => {
            let mut f = file.clone();
            f.seek(SeekFrom::Start(0)).map_err(|e| io_err(&e, &name))?;
            (Box::new(TarWalker::new(MultiGzDecoder::new(f), skip_read, &name)), 0)
        }
    };
    // Il cursore può venire da una voce precedente: gli errori nominano questa.
    let here = |e: AppError| AppError { path: Some(name.to_string_lossy().into_owned()), ..e };
    while next <= order {
        walker.next_header().map_err(here)?.ok_or_else(|| corrupt(&name, "voce non trovata"))?;
        next += 1;
    }
    Ok(Box::new(TarGzEntry { walker: Some(walker), next, slot: slot.clone(), name }))
}

/// I dati di una voce del TAR.GZ. Letta fino in fondo, alla chiusura il cursore torna nello slot.
struct TarGzEntry {
    walker: Option<Box<GzWalker>>,
    next: u64,
    slot: Slot,
    name: PathBuf,
}

impl Read for TarGzEntry {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let walker = self.walker.as_mut().expect("cursore presente fino alla chiusura");
        walker.read(buf).map_err(|e| to_io(e, &self.name))
    }
}

impl Drop for TarGzEntry {
    fn drop(&mut self) {
        if let Some(walker) = self.walker.take().filter(|w| w.entry_left() == 0) {
            *self.slot.lock().unwrap() = Some(Cursor::TarGz { walker, next: self.next });
        }
    }
}

/// Thread che decomprime un blocco 7z e passa le voci chieste, nell'ordine del blocco.
pub(super) struct Decoder {
    block: usize,
    tx: Sender<Request>,
    state: Arc<Mutex<State>>,
}

struct State {
    /// Prossima voce (indice dei file dell'archivio) che il thread offrirà.
    next: usize,
    /// Ha una richiesta da servire: una nuova richiesta aspetterebbe la chiusura di quella prima.
    busy: bool,
    /// Il thread non accetta più richieste.
    dead: bool,
}

struct Request {
    file: usize,
    out: PipeWriter,
    /// Forma testuale della voce, per gli errori.
    name: PathBuf,
}

/// Il blocco 7z è cifrato: senza password non si legge.
pub(super) fn encrypted_block(archive: &Archive, block: usize) -> bool {
    archive.blocks.get(block).is_some_and(|b| b.coders.iter().any(|c| c.encoder_method_id() == EncoderMethod::ID_AES256_SHA256))
}

/// La voce `file` del blocco `block`: dal thread del cursore se è libero e non l'ha già superata, altrimenti da un
/// thread nuovo (quello vecchio, senza più richieste, finisce da sé).
pub(super) fn sevenz(slot: &Slot, archive: &Arc<Archive>, container: &ContainerFile, block: usize, file: usize, name: PathBuf) -> Box<dyn Read + Send> {
    let (out, reader) = pipe();
    let mut req = Request { file, out, name };
    let mut slot = slot.lock().unwrap();
    if let Some(Cursor::SevenZ(d)) = slot.as_ref()
        && d.block == block
    {
        let mut st = d.state.lock().unwrap();
        if !st.dead && !st.busy && st.next <= file {
            match d.tx.send(req) {
                Ok(()) => {
                    st.busy = true;
                    return Box::new(reader);
                }
                Err(e) => req = e.0,
            }
        }
    }
    let d = spawn(archive.clone(), container.clone(), block);
    d.state.lock().unwrap().busy = true;
    // Il thread è appena partito e aspetta: la richiesta arriva.
    let _ = d.tx.send(req);
    *slot = Some(Cursor::SevenZ(d));
    Box::new(reader)
}

fn spawn(archive: Arc<Archive>, container: ContainerFile, block: usize) -> Decoder {
    let (tx, rx) = channel();
    let first = archive.stream_map.block_first_file_index.get(block).copied().unwrap_or(0);
    let state = Arc::new(Mutex::new(State { next: first, busy: false, dead: false }));
    let st = state.clone();
    std::thread::spawn(move || run(&archive, container, block, first, &rx, &st));
    Decoder { block, tx, state }
}

/// La prossima richiesta; `None` se il cursore è stato sostituito o dopo `IDLE` senza richieste.
fn wait(rx: &Receiver<Request>, state: &Mutex<State>) -> Option<Request> {
    match rx.recv_timeout(IDLE) {
        Ok(r) => Some(r),
        // Sotto il lucchetto di chi chiede: o la richiesta è già nel canale, o non ne arriveranno altre.
        Err(RecvTimeoutError::Timeout) => {
            let mut st = state.lock().unwrap();
            let r = rx.try_recv().ok();
            st.dead = r.is_none();
            r
        }
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

/// Copia `data` in `out`, o lo scarta se `keep` è falso. `Ok(false)` se il lettore è stato chiuso.
fn pump(data: &mut dyn Read, out: &mut PipeWriter, keep: bool, name: &Path) -> Result<bool, AppError> {
    use std::io::Write;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if out.is_closed() {
            return Ok(false);
        }
        let n = match data.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(&e, name)),
        };
        if keep && out.write_all(&buf[..n]).is_err() {
            return Ok(false);
        }
    }
}

/// Errore del crate 7z come in `index::build_7z`.
fn sevenz_error(e: &sevenz_rust2::Error, name: &Path) -> AppError {
    use sevenz_rust2::Error;
    let unsupported = |detail: Option<&str>| AppError { detail: detail.map(Into::into), ..AppError::new(ErrorCode::Unsupported, Some(name)) };
    match e {
        Error::PasswordRequired | Error::MaybeBadPassword(_) => unsupported(Some("encrypted")),
        Error::UnsupportedCompressionMethod(m) if m.contains("AES") => unsupported(Some("encrypted")),
        Error::UnsupportedCompressionMethod(_) | Error::ExternalUnsupported => unsupported(None),
        Error::Io(e, _) | Error::FileOpen(e, _) => io_err(e, name),
        e => corrupt(name, e),
    }
}

fn run(archive: &Archive, mut container: ContainerFile, block: usize, first: usize, rx: &Receiver<Request>, state: &Mutex<State>) {
    let password = Password::empty();
    let (mut at, mut pending, mut failed): (usize, Option<Request>, Option<AppError>) = (first, None, None);
    let r = BlockDecoder::new(1, block, archive, &password, &mut container).for_each_entries(&mut |_, data: &mut dyn Read| {
        let i = at;
        at += 1;
        if pending.is_none() {
            pending = wait(rx, state);
        }
        // Una voce già passata non si chiede mai a questo thread (vedi `State::next`).
        let Some(req) = pending.as_mut().filter(|r| r.file >= i) else { return Ok(false) };
        let keep = req.file == i;
        match pump(data, &mut req.out, keep, &req.name) {
            Ok(true) => {
                // Lo stato prima della fine dei dati: chi ha letto la voce può chiedere subito la successiva e deve
                // trovare il thread libero, non aprirne un altro che rifarebbe il blocco da capo.
                {
                    let mut st = state.lock().unwrap();
                    st.next = i + 1;
                    st.busy &= !keep;
                }
                if keep {
                    pending.take().expect("richiesta servita").out.finish();
                }
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(e) => {
                failed = Some(e);
                Ok(false)
            }
        }
    });
    // Da qui nessuno manda più richieste: quelle rimaste (anche se il blocco non si è aperto) ricevono l'errore.
    state.lock().unwrap().dead = true;
    let r = r.err();
    for req in pending.into_iter().chain(rx.try_iter()) {
        let err = match (&failed, &r) {
            (Some(e), _) => AppError { path: Some(req.name.to_string_lossy().into_owned()), ..e.clone() },
            (None, Some(e)) => sevenz_error(e, &req.name),
            (None, None) => corrupt(&req.name, "voce non trovata nel blocco"),
        };
        req.out.fail(err);
    }
}
