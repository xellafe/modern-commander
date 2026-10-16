//! Processo helper (`--elevated-helper <pipe> <token>`): si connette alla pipe dell'app, manda
//! `Hello` ed esegue le richieste una alla volta. Esce quando la pipe si chiude (app terminata),
//! dopo un messaggio non valido o dopo 60 s senza richieste (`MC_HELPER_IDLE_MS` nei test).

use super::protocol::{Request, Response, read_msg, validate, write_msg};
use super::{PIPE_PREFIX, Pipe, owned};
use crate::error::{AppError, ErrorCode};
use crate::ops::exec::{Executor, LocalExecutor};
use crate::paths::normalize_lexical;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::core::HSTRING;

const DEFAULT_IDLE_MS: u64 = 60_000;

fn idle_timeout() -> Duration {
    let ms = std::env::var("MC_HELPER_IDLE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_IDLE_MS);
    Duration::from_millis(ms)
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

/// Codice d'uscita: 0 fine normale, 1 pipe non raggiungibile, 2 nome della pipe non valido.
pub fn run(pipe_name: &str, token: &str) -> i32 {
    if !pipe_name.starts_with(PIPE_PREFIX) {
        return 2;
    }
    let Ok(pipe) = open(pipe_name) else { return 1 };
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
        Request::Hello { .. } | Request::Cancel => Err(AppError::new(ErrorCode::InvalidRequest, None)),
    }
}
