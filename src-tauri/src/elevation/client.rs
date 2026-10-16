//! Lato app: crea la pipe, avvia l'helper elevato, lo autentica e gli inoltra le primitive.

use super::protocol::{MAX_LINE, Request, Response, read_msg, write_msg};
use super::{PIPE_PREFIX, Pipe, event, is, os_code, owned, raw};
use crate::error::{AppError, ErrorCode};
use crate::ops::exec::Executor;
use std::ffi::c_void;
use std::io::{self, BufReader};
use std::os::windows::io::OwnedHandle;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use windows::Win32::Foundation::{
    ERROR_CANCELLED, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, HWND, LocalFree, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

/// Tempo massimo tra l'avvio dell'helper e la sua connessione alla pipe.
const CONNECT_TIMEOUT_MS: u32 = 30_000;

/// Finestra principale (come intero: `HWND` non è `Sync`), proprietaria del prompt UAC.
static OWNER: OnceLock<isize> = OnceLock::new();

/// Da chiamare nel `setup` di Tauri: il prompt UAC compare sopra l'app invece che dietro.
pub fn set_owner_window(hwnd: HWND) {
    let _ = OWNER.set(hwnd.0 as isize);
}

type Progress<'a> = &'a mut dyn FnMut(u64) -> bool;

fn fail(code: ErrorCode, detail: &str) -> AppError {
    AppError { detail: Some(detail.into()), ..AppError::new(code, None) }
}

fn win_fail(e: &windows::core::Error) -> AppError {
    AppError { detail: Some(e.message()), ..AppError::from_io(&io::Error::from_raw_os_error(os_code(e)), Path::new("")) }
}

/// L'helper non risponde più (uscito per inattività, terminato, protocollo rotto).
fn gone() -> AppError {
    fail(ErrorCode::Io, "helper_gone")
}

/// SID dell'utente del processo corrente, in forma testuale (`S-1-5-21-…`).
fn current_user_sid() -> windows::core::Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
    let token = unsafe { owned(token) };
    let mut len = 0;
    let _ = unsafe { GetTokenInformation(raw(&token), TokenUser, None, 0, &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)]; // allineato per TOKEN_USER
    unsafe { GetTokenInformation(raw(&token), TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len) }?;
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut s = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut s) }?;
    let sid = unsafe { s.to_string() };
    unsafe { LocalFree(Some(HLOCAL(s.0.cast()))) };
    sid.map_err(|_| windows::core::Error::from_thread())
}

/// Unica istanza, solo client locali, DACL: utente corrente e Administrators (`BA` serve quando
/// l'amministratore del prompt UAC è un altro account).
fn create_pipe(name: &str) -> Result<Pipe, AppError> {
    let sid = current_user_sid().map_err(|e| win_fail(&e))?;
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})(A;;GA;;;BA)"));
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None) }.map_err(|e| win_fail(&e))?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let h = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_LINE as u32,
            MAX_LINE as u32,
            0,
            Some(&sa),
        )
    };
    let err = h.is_invalid().then(windows::core::Error::from_thread);
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
    match err {
        Some(e) => Err(win_fail(&e)),
        None => Ok(Pipe::new(unsafe { owned(h) })),
    }
}

/// 32 byte casuali in esadecimale.
fn random_token() -> Result<String, AppError> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|_| fail(ErrorCode::Io, "rng"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Confronto a tempo costante (la lunghezza del token non è segreta).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && std::hint::black_box(a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y))) == 0
}

/// Attende la connessione dell'helper (al massimo `CONNECT_TIMEOUT_MS`, o finché il processo
/// lanciato non esce) e verifica che il client sia proprio quel processo.
fn accept(pipe: &Pipe, process: &OwnedHandle) -> Result<(), AppError> {
    let ev = event().map_err(|e| AppError::from_io(&e, Path::new("")))?;
    let mut ov = OVERLAPPED { hEvent: raw(&ev), ..Default::default() };
    match unsafe { ConnectNamedPipe(pipe.handle(), Some(&mut ov)) } {
        Err(e) if is(&e, ERROR_PIPE_CONNECTED) => {}
        Err(e) if !is(&e, ERROR_IO_PENDING) => return Err(win_fail(&e)),
        _ => {
            let w = unsafe { WaitForMultipleObjects(&[raw(&ev), raw(process)], false, CONNECT_TIMEOUT_MS) };
            let mut n = 0;
            if w != WAIT_OBJECT_0 {
                // Si attende la fine dell'operazione annullata: `ov` deve restare valido fino ad allora.
                let _ = unsafe { CancelIoEx(pipe.handle(), Some(&ov)) };
                let _ = unsafe { GetOverlappedResult(pipe.handle(), &ov, &mut n, true) };
                return Err(fail(ErrorCode::Io, "helper_not_connected"));
            }
            unsafe { GetOverlappedResult(pipe.handle(), &ov, &mut n, false) }.map_err(|e| win_fail(&e))?;
        }
    }
    let mut pid = 0;
    unsafe { GetNamedPipeClientProcessId(pipe.handle(), &mut pid) }.map_err(|e| win_fail(&e))?;
    if pid != unsafe { GetProcessId(raw(process)) } {
        let _ = unsafe { DisconnectNamedPipe(pipe.handle()) };
        return Err(fail(ErrorCode::AccessDenied, "helper_pid_mismatch"));
    }
    Ok(())
}

struct Conn {
    input: BufReader<Pipe>,
    output: Pipe,
}

impl Conn {
    /// Manda la richiesta e legge fino alla risposta finale; se `progress` restituisce `false`
    /// manda `Cancel` (una volta) e continua a leggere.
    fn exchange(&mut self, req: &Request, mut progress: Option<Progress>) -> io::Result<Result<(), AppError>> {
        write_msg(&mut self.output, req)?;
        let mut cancelled = false;
        loop {
            match read_msg::<Response>(&mut self.input)? {
                None => return Err(io::ErrorKind::BrokenPipe.into()),
                Some(Response::Ok) => return Ok(Ok(())),
                Some(Response::Err(e)) => return Ok(Err(e)),
                Some(Response::Progress { bytes }) => {
                    if let Some(p) = progress.as_mut()
                        && !cancelled
                        && !p(bytes)
                    {
                        cancelled = true;
                        write_msg(&mut self.output, &Request::Cancel)?;
                    }
                }
            }
        }
    }
}

/// `Executor` che inoltra ogni primitiva all'helper elevato.
pub struct ElevatedExecutor {
    // ponytail: un'operazione elevata alla volta per tutta l'app. Una copia in pausa blocca le
    // altre elevate, e un job in attesa di questo lock non si annulla finché non lo ottiene;
    // più connessioni se servirà parallelismo.
    conn: Mutex<Conn>,
    /// Connessione inutilizzabile: errore di I/O o scambio interrotto a metà (anche da un panico).
    broken: AtomicBool,
    process: OwnedHandle,
}

impl ElevatedExecutor {
    /// Avvia l'helper con il prompt UAC. Prompt rifiutato: `ACCESS_DENIED`, detail `uac_declined`.
    pub fn spawn() -> Result<Self, AppError> {
        let exe = std::env::current_exe().map_err(|e| AppError::from_io(&e, Path::new("")))?;
        Self::connect(|pipe, token| {
            let file = HSTRING::from(exe.as_os_str());
            let params = HSTRING::from(format!("--elevated-helper {pipe} {token}"));
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
                hwnd: HWND(OWNER.get().copied().unwrap_or(0) as *mut c_void),
                lpVerb: w!("runas"),
                lpFile: PCWSTR(file.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_HIDE.0,
                ..Default::default()
            };
            match unsafe { ShellExecuteExW(&mut info) } {
                Err(e) if is(&e, ERROR_CANCELLED) => Err(fail(ErrorCode::AccessDenied, "uac_declined")),
                Err(e) => Err(win_fail(&e)),
                Ok(()) if info.hProcess.is_invalid() => Err(fail(ErrorCode::Io, "helper_not_started")),
                Ok(()) => Ok(unsafe { owned(info.hProcess) }),
            }
        })
    }

    /// Per i test: `launch(pipe, token)` avvia l'helper come preferisce e ne restituisce il PID.
    #[doc(hidden)]
    pub fn spawn_with(launch: impl FnOnce(&str, &str) -> Result<u32, AppError>) -> Result<Self, AppError> {
        Self::connect(|pipe, token| {
            let pid = launch(pipe, token)?;
            let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
                .map_err(|e| win_fail(&e))?;
            Ok(unsafe { owned(h) })
        })
    }

    /// `launch` restituisce l'handle del processo avviato.
    fn connect(launch: impl FnOnce(&str, &str) -> Result<OwnedHandle, AppError>) -> Result<Self, AppError> {
        let name = format!("{PIPE_PREFIX}{}", uuid::Uuid::new_v4());
        let token = random_token()?;
        let pipe = create_pipe(&name)?;
        let process = launch(&name, &token)?;
        accept(&pipe, &process)?;
        // Chiudendo la pipe (return d'errore) l'helper riceve la fine dello stream ed esce.
        let mut input = BufReader::new(pipe.clone());
        match read_msg::<Request>(&mut input) {
            Ok(Some(Request::Hello { token: t })) if ct_eq(t.as_bytes(), token.as_bytes()) => {}
            _ => return Err(fail(ErrorCode::AccessDenied, "helper_auth_failed")),
        }
        Ok(Self { conn: Mutex::new(Conn { input, output: pipe }), broken: AtomicBool::new(false), process })
    }

    /// Connessione integra e processo ancora in vita.
    fn is_alive(&self) -> bool {
        !self.broken.load(Ordering::SeqCst) && unsafe { WaitForSingleObject(raw(&self.process), 0) } == WAIT_TIMEOUT
    }

    /// `broken` segna solo i guasti veri: durante uno scambio in corso l'helper resta "vivo" per
    /// `shared`, che altrimenti ne avvierebbe un secondo (con un altro prompt UAC).
    fn call(&self, req: Request, progress: Option<Progress>) -> Result<(), AppError> {
        let mut conn = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        if self.broken.load(Ordering::SeqCst) {
            return Err(gone());
        }
        let _guard = BreakOnPanic(&self.broken);
        let res = conn.exchange(&req, progress);
        if res.is_err() {
            self.broken.store(true, Ordering::SeqCst);
        }
        res.unwrap_or_else(|_| Err(gone()))
    }
}

/// Un panico (per esempio nel callback di avanzamento) interrompe lo scambio a metà: le risposte
/// rimaste nella pipe desincronizzerebbero il protocollo, quindi la connessione non si usa più.
struct BreakOnPanic<'a>(&'a AtomicBool);

impl Drop for BreakOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

impl Executor for ElevatedExecutor {
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, flush: bool, progress: Progress) -> Result<(), AppError> {
        let req = Request::Copy { src: src.into(), dst: dst.into(), overwrite, flush };
        self.call(req, Some(progress))
    }

    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
        self.call(Request::Move { src: src.into(), dst: dst.into(), overwrite }, None)
    }

    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
        self.call(Request::Delete { path: path.into(), is_dir, permanent }, None)
    }

    fn mkdir(&self, path: &Path) -> Result<(), AppError> {
        self.call(Request::Mkdir { path: path.into() }, None)
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
        self.call(Request::Rename { src: src.into(), dst: dst.into() }, None)
    }
}

/// Helper condiviso dall'intera sessione: si avvia (con il prompt UAC) al primo uso e si riavvia
/// al primo uso dopo la sua uscita. Il lock copre l'avvio: due job non aprono due prompt.
pub fn shared() -> Result<Arc<dyn Executor>, AppError> {
    static SHARED: Mutex<Option<Arc<ElevatedExecutor>>> = Mutex::new(None);
    Ok(shared_with(&SHARED, ElevatedExecutor::spawn)?)
}

/// `shared` con slot e avvio indicati dal chiamante (i test usano `spawn_with`, senza UAC).
/// Un secondo job che chiede l'helper mentre un altro lo usa riceve lo stesso esecutore e poi
/// attende il mutex della connessione: finché non lo ottiene non si può annullare.
#[doc(hidden)]
pub fn shared_with(
    slot: &Mutex<Option<Arc<ElevatedExecutor>>>,
    spawn: impl FnOnce() -> Result<ElevatedExecutor, AppError>,
) -> Result<Arc<ElevatedExecutor>, AppError> {
    let mut g = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(x) = g.as_ref()
        && x.is_alive()
    {
        return Ok(x.clone());
    }
    let x = Arc::new(spawn()?);
    *g = Some(x.clone());
    Ok(x)
}
