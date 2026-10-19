//! "Riprova come amministratore". L'app (server della pipe) avvia con `runas` lo stesso eseguibile
//! con `--elevated-helper <pipe> <token>`; l'helper si connette, si autentica con il token ed esegue
//! solo primitive su file (`LocalExecutor`) su percorsi che valida e normalizza da sé.
//!
//! Sicurezza (spec §6.2, non semplificabile):
//! - pipe `\\.\pipe\mc-elev-<uuid>` con `FILE_FLAG_FIRST_PIPE_INSTANCE` (nessuno può crearla prima
//!   dell'app), una sola istanza, `PIPE_REJECT_REMOTE_CLIENTS`, DACL limitata all'utente corrente e
//!   agli amministratori;
//! - il client connesso deve essere il processo lanciato (`GetNamedPipeClientProcessId`) e il primo
//!   messaggio deve portare il token (32 byte casuali, confronto a tempo costante);
//! - l'helper apre la pipe con `SECURITY_IDENTIFICATION`: il server non può impersonarlo;
//! - righe oltre `MAX_LINE` chiudono la connessione; il token non va mai nei log.
//!
//! ## Nota di sicurezza
//!
//! **Modello di minaccia.** L'helper gira con un token amministrativo e accetta comandi da una
//! pipe: chi riuscisse a parlargli al posto dell'app otterrebbe operazioni su file come
//! amministratore senza prompt UAC. Gli avversari considerati sono i processi non elevati di
//! qualsiasi utente della macchina e i client remoti.
//!
//! **Cosa garantiscono i controlli.**
//! - ACL (utente corrente + Administrators) e `PIPE_REJECT_REMOTE_CLIENTS`: altri utenti non
//!   elevati e client di rete non aprono la pipe.
//! - `FILE_FLAG_FIRST_PIPE_INSTANCE` e istanza unica: nessuno può creare la pipe prima dell'app o
//!   aggiungere una seconda istanza; con l'helper connesso un altro client trova la pipe occupata.
//! - PID del client = processo lanciato e token monouso: anche un processo dello stesso utente che
//!   conosce nome e token non sostituisce l'helper.
//! - L'helper verifica il server (`GetNamedPipeServerProcessId` + immagine = proprio eseguibile)
//!   prima di inviare il token, valida in modo stretto nome della pipe e token e rifiuta percorsi
//!   relativi, `.`/`..`, dispositivi e le condivisioni `pipe`/`IPC$`/`mailslot` (niente
//!   richieste ad altre pipe tramite UNC).
//! - Giunzioni e collegamenti nei componenti intermedi (`pathlock`): per tutta la durata della
//!   primitiva l'helper tiene aperte le cartelle antenate, una a una, con
//!   `FILE_FLAG_OPEN_REPARSE_POINT` e senza `FILE_SHARE_DELETE`: non si possono rinominare né
//!   eliminare né sostituire (la radice dell'unità o della condivisione non si apre, e non ha
//!   sentinella: non può diventare giunzione). Un reparse point come componente intermedio è
//!   rifiutato (`Unsupported`, `reparse_point_in_path`); come bersaglio (`reparse_point`) solo per
//!   la sorgente di una copia, l'elenco e l'eliminazione nel Cestino. Spostamento, rinomina ed
//!   eliminazione definitiva operano sul collegamento stesso, senza seguirlo.
//! - Sentinella: una cartella vuota si trasforma sul posto in giunzione anche se bloccata
//!   (`FSCTL_SET_REPARSE_POINT` chiede solo `FILE_WRITE_ATTRIBUTES`). Il blocco crea quindi nella
//!   cartella bloccata più profonda un file `.mc-lock-<uuid>` (condivisione 0, `DELETE_ON_CLOSE`,
//!   eliminazione POSIX al rilascio) che la tiene non vuota; l'elenco elevato non lo mostra. Se
//!   l'elemento stesso è bloccato (sorgente di una copia, elenco) il padre non ha
//!   sentinella: lo tiene non vuoto l'handle sull'elemento; una cartella bloccata ne ha una dentro.
//!   Senza sentinella il blocco fallisce.
//! - L'app risolve (giunzioni, `subst`, unità di rete) solo le radici del job, una volta, quando il
//!   job passa all'helper (`ops::rooted`); una radice che non esiste ancora si risolve dal suo
//!   antenato esistente più profondo. Una giunzione comparsa dopo sotto quel punto arriva
//!   all'helper come giunzione e, come componente intermedio, viene rifiutata. I blocchi si
//!   rilasciano prima della risposta finale.
//!
//! **Rischi residui.**
//! - Una radice del job sostituita con una giunzione prima della prima elevazione viene seguita:
//!   l'app la risolve prima di inviarla.
//! - Eliminazione nel Cestino: l'elemento si controlla ma non si blocca (il Cestino non sposta un
//!   elemento aperto senza `FILE_SHARE_DELETE`); resta una finestra in cui si può scambiare con un
//!   collegamento, che finisce nel Cestino come collegamento.
//! - Anche i reparse point che non sono collegamenti sono rifiutati: i volumi montati in una
//!   cartella come componenti intermedi; i file deduplicati e i segnaposto di OneDrive ("File su
//!   richiesta") soprattutto come bersaglio (sorgente di una copia, elenco, Cestino).
//! - Una cartella in cui l'amministratore non può creare file (supporti in sola lettura) non si
//!   blocca dove serve la sentinella: scriverci o elencarla come amministratore fallisce. La
//!   copia da quei supporti funziona (la sorgente non ha sentinella nel padre).
//! - Durante un'operazione elevata il file nascosto `.mc-lock-*` è visibile nella cartella
//!   bloccata (pannelli con i file nascosti mostrati, eventi del watcher); dopo un'interruzione
//!   di corrente a metà richiesta può restare come file normale, da eliminare a mano.
//! - Percorsi che l'app non riesce a risolvere arrivano invariati; l'helper li rifiuta se
//!   contengono un collegamento.
//! - Il codice che gira già con l'utente corrente può pilotare l'app (input, debug) e quindi le
//!   operazioni elevate: è fuori dal confine, come per qualsiasi app che usa l'UAC.
//! - L'eseguibile non deve essere scrivibile dall'utente, altrimenti il binario lanciato con
//!   `runas` potrebbe essere sostituito: serve un'installazione per macchina (`Program Files`),
//!   che il Task 22 configura con NSIS `perMachine`. Una copia portable in una cartella
//!   dell'utente non ha questa garanzia.

pub mod client;
pub mod helper;
pub mod pathlock;
pub mod protocol;

pub use client::{ElevatedExecutor, set_owner_window, shared, shared_with};
pub use protocol::{Request, Response, validate};

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_NOT_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::HRESULT;

/// Prefisso dei nomi delle pipe: l'helper non si connette ad altro.
pub(crate) const PIPE_PREFIX: &str = r"\\.\pipe\mc-elev-";

/// Codice Win32 di un errore `windows` (HRESULT `0x8007xxxx`), come lo vuole `io::Error`.
pub(crate) fn os_code(e: &windows::core::Error) -> i32 {
    let c = e.code().0;
    if (c as u32) & 0xFFFF_0000 == 0x8007_0000 { c & 0xFFFF } else { c }
}

pub(crate) fn win_io(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(os_code(&e))
}

pub(crate) fn is(e: &windows::core::Error, code: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    e.code() == HRESULT::from_win32(code.0)
}

pub(crate) fn raw(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

/// SAFETY del chiamante: `h` è valido e non è posseduto da altri.
pub(crate) unsafe fn owned(h: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(h.0) }
}

/// Evento manual-reset per un'operazione overlapped.
pub(crate) fn event() -> io::Result<OwnedHandle> {
    let h = unsafe { CreateEventW(None, true, false, None) }.map_err(win_io)?;
    Ok(unsafe { owned(h) })
}

/// Estremità di una named pipe aperta con `FILE_FLAG_OVERLAPPED`: lettura e scrittura possono
/// procedere insieme da thread diversi (con un handle sincrono si bloccherebbero a vicenda).
/// Ogni operazione attende la propria fine, quindi i buffer restano validi.
#[derive(Clone)]
pub(crate) struct Pipe(Arc<OwnedHandle>);

impl Pipe {
    pub(crate) fn new(h: OwnedHandle) -> Self {
        Pipe(Arc::new(h))
    }

    pub(crate) fn handle(&self) -> HANDLE {
        raw(&self.0)
    }

    fn overlapped(&self, start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> windows::core::Result<()>) -> io::Result<usize> {
        let ev = event()?;
        let mut ov = OVERLAPPED { hEvent: raw(&ev), ..Default::default() };
        if let Err(e) = start(self.handle(), &mut ov)
            && !is(&e, ERROR_IO_PENDING)
        {
            return Err(win_io(e));
        }
        let mut n = 0;
        unsafe { GetOverlappedResult(self.handle(), &ov, &mut n, true) }.map_err(win_io)?;
        Ok(n as usize)
    }
}

impl io::Read for Pipe {
    /// L'altra estremità chiusa è la fine dello stream.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.overlapped(|h, ov| unsafe { ReadFile(h, Some(buf), None, Some(ov)) }) {
            Err(e) if [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED].iter().any(|c| e.raw_os_error() == Some(c.0 as i32)) => Ok(0),
            r => r,
        }
    }
}

impl io::Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.overlapped(|h, ov| unsafe { WriteFile(h, Some(buf), None, Some(ov)) })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
