//! Processo helper (`--elevated-helper <pipe> <token>`): si connette alla pipe dell'app, manda
//! `Hello` ed esegue le richieste una alla volta. Esce quando la pipe si chiude (app terminata),
//! dopo un messaggio non valido o dopo 60 s senza richieste.

use super::protocol::{Request, Response, read_msg, validate, write_msg};
use super::{PIPE_PREFIX, Pipe, owned, raw};
use crate::error::{AppError, ErrorCode};
use crate::ops::exec::{Executor, LocalExecutor};
use crate::paths::{normalize_lexical, same_path};
use std::ffi::OsString;
use std::io::BufReader;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{HSTRING, PWSTR};

const IDLE_MS: u64 = 60_000;

/// 60 s; solo con la feature `test-hooks` `MC_HELPER_IDLE_MS` lo cambia.
fn idle_timeout() -> Duration {
    #[cfg(feature = "test-hooks")]
    if let Some(ms) = std::env::var("MC_HELPER_IDLE_MS").ok().and_then(|v| v.parse().ok()) {
        return Duration::from_millis(ms);
    }
    Duration::from_millis(IDLE_MS)
}

/// `SECURITY_IDENTIFICATION`: il server può solo identificare l'helper, non agire con il suo token
/// elevato. Con l'unica istanza già occupata `CreateFileW` fallisce (`ERROR_PIPE_BUSY`).
fn open(name: &str) -> windows::core::Result<Pipe> {
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(name),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )
    }?;
    Ok(Pipe::new(unsafe { owned(h) }))
}

/// Solo `\\.\pipe\mc-elev-<uuid in forma canonica>` e token di 64 cifre esadecimali.
fn valid_args(pipe_name: &str, token: &str) -> bool {
    let uuid_ok = pipe_name
        .strip_prefix(PIPE_PREFIX)
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id));
    uuid_ok && token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Eseguibile di un processo secondo `QueryFullProcessImageNameW`, che risolve unità `subst`,
/// giunzioni e collegamenti: le due immagini da confrontare si ricavano entrambe così.
fn image_of(process: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
}

fn server_image(pipe: &Pipe) -> Option<PathBuf> {
    let mut pid = 0;
    unsafe { GetNamedPipeServerProcessId(pipe.handle(), &mut pid) }.ok()?;
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    image_of(raw(&unsafe { owned(h) }))
}

/// Il server deve eseguire lo stesso eseguibile dell'helper. Solo con la feature `test-hooks`
/// `MC_TEST_SERVER_EXE` indica un altro eseguibile atteso (nei test d'integrazione il server è
/// l'eseguibile dei test); senza la feature il ramo non viene compilato.
fn expected_server_image() -> Option<PathBuf> {
    #[cfg(feature = "test-hooks")]
    if let Some(p) = std::env::var_os("MC_TEST_SERVER_EXE") {
        return Some(p.into());
    }
    image_of(unsafe { GetCurrentProcess() })
}

/// Confronto senza distinguere maiuscole e minuscole.
fn server_is_trusted(pipe: &Pipe) -> bool {
    match (server_image(pipe), expected_server_image()) {
        (Some(server), Some(expected)) => same_path(&normalize_lexical(&server), &normalize_lexical(&expected)),
        _ => false,
    }
}

/// Codice d'uscita: 0 fine normale, 1 pipe non raggiungibile, 2 argomenti non validi, 3 server
/// della pipe non riconosciuto (il token non viene inviato).
pub fn run(pipe_name: &str, token: &str) -> i32 {
    if !valid_args(pipe_name, token) {
        return 2;
    }
    let Ok(pipe) = open(pipe_name) else { return 1 };
    if !server_is_trusted(&pipe) {
        return 3;
    }
    let mut out = pipe.clone();
    if write_msg(&mut out, &Request::Hello { token: token.to_owned() }).is_err() {
        return 1;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let flag = cancel.clone();
    // Il lettore gira accanto all'esecuzione, così `Cancel` arriva durante una copia. Si ferma a
    // fine pipe o al primo messaggio non valido; il canale chiuso fa uscire il ciclo principale.
    std::thread::spawn(move || {
        let mut input = BufReader::new(pipe);
        while let Ok(Some(req)) = read_msg::<Request>(&mut input) {
            if req == Request::Cancel {
                flag.store(true, Ordering::SeqCst);
            } else if tx.send(req).is_err() {
                break;
            }
        }
    });
    let idle = idle_timeout();
    // Inattività o pipe chiusa: si esce (il thread lettore muore con il processo).
    while let Ok(req) = rx.recv_timeout(idle) {
        // Un `Cancel` arrivato dopo la fine della copia precedente non vale per questa richiesta:
        // il lettore lo ha registrato prima di inoltrare la richiesta successiva.
        cancel.store(false, Ordering::SeqCst);
        let resp = match execute(&req, &mut out, &cancel) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Err(e),
        };
        if write_msg(&mut out, &resp).is_err() {
            break;
        }
    }
    0
}

/// Valida, normalizza i percorsi ed esegue con `LocalExecutor`.
fn execute(req: &Request, out: &mut Pipe, cancel: &AtomicBool) -> Result<(), AppError> {
    validate(req)?;
    let x = LocalExecutor;
    let n = normalize_lexical;
    match req {
        Request::Copy { src, dst, overwrite, flush } => x.copy_file(&n(src), &n(dst), *overwrite, *flush, &mut |bytes| {
            // Pipe chiusa: si annulla la copia.
            write_msg(out, &Response::Progress { bytes }).is_ok() && !cancel.load(Ordering::SeqCst)
        }),
        Request::Move { src, dst, overwrite } => x.move_path(&n(src), &n(dst), *overwrite),
        Request::Delete { path, is_dir, permanent } => x.delete(&n(path), *is_dir, *permanent),
        Request::Mkdir { path } => x.mkdir(&n(path)),
        Request::Rename { src, dst } => x.rename(&n(src), &n(dst)),
        Request::List { path } => {
            for e in x.list_dir(&n(path))? {
                // Pipe chiusa: anche la risposta finale non arriverà, e il ciclo principale esce.
                write_msg(out, &Response::Entry(e)).map_err(|e| AppError::from_io(&e, path))?;
            }
            Ok(())
        }
        Request::Hello { .. } | Request::Cancel => Err(AppError::new(ErrorCode::InvalidRequest, None)),
    }
}
