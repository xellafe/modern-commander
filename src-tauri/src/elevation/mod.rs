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

pub mod client;
pub mod helper;
pub mod protocol;

pub use client::{ElevatedExecutor, shared};
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
