//! Processo helper (`--elevated-helper <pipe> <token>`): si connette alla pipe dell'app, manda
//! `Hello` ed esegue le richieste una alla volta. Esce quando la pipe si chiude (app terminata),
//! dopo un messaggio non valido o dopo 60 s senza richieste.

use super::protocol::{Request, Response, read_msg, validate, write_msg};
use super::pathlock::{PathLock, lock_ancestors, lock_with_item};
use super::{PIPE_PREFIX, Pipe, owned, raw};
use crate::error::{AppError, ErrorCode};
use crate::ops::exec::{Executor, LocalExecutor};
use crate::paths::{normalize_lexical, same_path, to_extended};
use std::ffi::OsString;
use std::io::{BufReader, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
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

/// Blocchi da tenere durante l'esecuzione di `req` (percorsi già validati; `lock_*` li normalizza).
fn locks_for(req: &Request) -> Result<Vec<PathLock>, AppError> {
    Ok(match req {
        // La sorgente non può essere un collegamento: la copia elevata leggerebbe il file puntato.
        Request::Copy { src, dst, .. } => vec![lock_with_item(src)?, lock_ancestors(dst)?],
        // Un collegamento come bersaglio si sposta (o si elimina) come tale, senza seguirlo.
        Request::Move { src, dst, .. } | Request::Rename { src, dst } => vec![lock_ancestors(src)?, lock_ancestors(dst)?],
        Request::Delete { path, permanent: true, .. } | Request::Mkdir { path } => vec![lock_ancestors(path)?],
        // Un collegamento non va nel Cestino come amministratore. Niente handle sull'elemento: senza
        // `FILE_SHARE_DELETE` il Cestino fallirebbe; resta la finestra tra controllo e cestinazione.
        Request::Delete { path, permanent: false, .. } => {
            let lock = lock_ancestors(path)?;
            let p = normalize_lexical(path);
            if std::fs::symlink_metadata(to_extended(&p)).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0) {
                return Err(AppError { detail: Some("reparse_point".into()), ..AppError::new(ErrorCode::Unsupported, Some(&p)) });
            }
            vec![lock]
        }
        Request::List { path } => vec![lock_with_item(path)?],
        Request::Hello { .. } | Request::Cancel => Vec::new(),
    })
}

/// Valida, normalizza e blocca i percorsi (vedi `locks_for`), poi esegue con `LocalExecutor`. I
/// blocchi si rilasciano prima della risposta finale: con `Ok` nessuna sentinella resta nella cartella.
fn execute(req: &Request, out: &mut impl Write, cancel: &AtomicBool) -> Result<(), AppError> {
    validate(req)?;
    let locks = locks_for(req)?;
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
            // Le voci si inviano sotto blocco; le sentinelle non sono voci della cartella.
            for e in x.list_dir(&n(path))?.into_iter().filter(|e| !locks.iter().any(|l| l.hides(&e.name))) {
                // Pipe chiusa: anche la risposta finale non arriverà, e il ciclo principale esce.
                write_msg(out, &Response::Entry(e)).map_err(|e| AppError::from_io(&e, path))?;
            }
            Ok(())
        }
        Request::Hello { .. } | Request::Cancel => Err(AppError::new(ErrorCode::InvalidRequest, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::model::ListEntry;
    use std::fs;
    use std::path::Path;

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
        assert!(ok, "mklink /J");
    }

    /// Esegue `req` come l'helper (senza pipe) e restituisce l'esito e le voci inviate.
    fn exec(req: Request) -> (Result<(), AppError>, Vec<ListEntry>) {
        let mut out = Vec::new();
        let res = execute(&req, &mut out, &AtomicBool::new(false));
        let mut input = out.as_slice();
        let mut entries = Vec::new();
        while let Some(r) = read_msg::<Response>(&mut input).unwrap() {
            if let Response::Entry(e) = r {
                entries.push(e);
            }
        }
        (res, entries)
    }

    fn refused(req: Request, detail: &str) {
        let e = exec(req).0.unwrap_err();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some(detail)));
    }

    #[test]
    fn requests_through_a_junction_are_refused_and_change_nothing() {
        let d = tempfile::tempdir().unwrap();
        let protected = d.path().join("protected");
        fs::create_dir(&protected).unwrap();
        fs::write(protected.join("keep.txt"), b"x").unwrap();
        fs::create_dir(d.path().join("work")).unwrap();
        let link = d.path().join(r"work\link");
        junction(&link, &protected);

        refused(Request::Delete { path: link.join("keep.txt"), is_dir: false, permanent: true }, "reparse_point_in_path");
        assert!(protected.join("keep.txt").exists());
        refused(Request::Mkdir { path: link.join("new") }, "reparse_point_in_path");
        assert!(!protected.join("new").exists());
        let out = d.path().join("out.txt");
        refused(Request::Copy { src: link.join("keep.txt"), dst: out.clone(), overwrite: false, flush: false }, "reparse_point_in_path");
        assert!(!out.exists());
    }

    #[test]
    fn link_targets_follow_the_table() {
        let d = tempfile::tempdir().unwrap();
        let (real, link) = (d.path().join("real"), d.path().join("link"));
        fs::create_dir(&real).unwrap();
        fs::write(real.join("f.txt"), b"x").unwrap();
        junction(&link, &real);

        refused(Request::List { path: link.clone() }, "reparse_point");
        refused(Request::Delete { path: link.clone(), is_dir: true, permanent: false }, "reparse_point");
        assert!(link.exists());
        exec(Request::Delete { path: link.clone(), is_dir: true, permanent: true }).0.unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(real.join("f.txt").exists());
    }

    #[test]
    fn ordinary_requests_still_work() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join(r"a\b"));
        let ok = |req: Request| exec(req).0.unwrap_or_else(|e| panic!("{e:?}"));
        ok(Request::Mkdir { path: a.clone() });
        ok(Request::Mkdir { path: b.clone() });
        let f = a.join("f.txt");
        fs::write(&f, b"x").unwrap();
        ok(Request::Copy { src: f.clone(), dst: b.join("g.txt"), overwrite: false, flush: false });
        ok(Request::Copy { src: f.clone(), dst: b.join("g.txt"), overwrite: true, flush: true });
        ok(Request::Rename { src: b.join("g.txt"), dst: b.join("h.txt") });
        ok(Request::Move { src: b.join("h.txt"), dst: a.join("h.txt"), overwrite: false });
        let (res, entries) = exec(Request::List { path: a.clone() });
        res.unwrap();
        let mut names: Vec<_> = entries.into_iter().map(|e| e.name.into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["b", "f.txt", "h.txt"]);
        ok(Request::Delete { path: a.join("h.txt"), is_dir: false, permanent: true });
        // Cestino: un file e una cartella vuota.
        ok(Request::Delete { path: f.clone(), is_dir: false, permanent: false });
        ok(Request::Delete { path: b.clone(), is_dir: true, permanent: false });
        assert!(!f.exists() && !b.exists() && !a.join("h.txt").exists());
        // Nessuna sentinella rimasta dopo il rilascio dei blocchi.
        assert_eq!(fs::read_dir(&a).unwrap().count(), 0);
        ok(Request::Delete { path: a.clone(), is_dir: true, permanent: true });
        assert!(!a.exists());
    }
}
