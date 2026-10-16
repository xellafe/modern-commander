//! Pipe autenticata dell'helper elevato. L'helper è l'eseguibile vero lanciato con
//! `--elevated-helper` tramite `ElevatedExecutor::spawn_with` e un normale `Command`, quindi
//! **senza** elevazione: si prova il protocollo e la sicurezza della pipe, non il prompt UAC
//! (verifica manuale).

use modern_commander_lib::elevation::ElevatedExecutor;
use modern_commander_lib::error::ErrorCode;
use modern_commander_lib::ops::exec::Executor;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_modern-commander");
const T: Duration = Duration::from_secs(10);

fn helper(pipe: &str, token: &str, idle_ms: Option<u64>) -> Child {
    let mut c = Command::new(BIN);
    c.args(["--elevated-helper", pipe, token]);
    if let Some(ms) = idle_ms {
        c.env("MC_HELPER_IDLE_MS", ms.to_string());
    }
    c.spawn().unwrap()
}

/// Helper avviato e autenticato: esecutore, processo, nome della pipe e token.
fn start(idle_ms: Option<u64>) -> (ElevatedExecutor, Child, String, String) {
    let (mut child, mut name, mut tok) = (None, String::new(), String::new());
    let x = ElevatedExecutor::spawn_with(|pipe, token| {
        let c = helper(pipe, token, idle_ms);
        let pid = c.id();
        (name, tok) = (pipe.to_owned(), token.to_owned());
        child = Some(c);
        Ok(pid)
    })
    .unwrap_or_else(|e| panic!("{e:?}"));
    (x, child.unwrap(), name, tok)
}

/// Stato d'uscita entro `within`; altrimenti termina il processo e dà `None`.
fn wait_exit(c: &mut Child, within: Duration) -> Option<ExitStatus> {
    let t = Instant::now();
    while t.elapsed() < within {
        if let Some(s) = c.try_wait().unwrap() {
            return Some(s);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = c.kill();
    None
}

#[test]
fn helper_executes_mkdir_and_copy() {
    let d = tempfile::tempdir().unwrap();
    let (x, mut child, _, _) = start(None);
    let dir = d.path().join("nuova");
    x.mkdir(&dir).unwrap();
    assert!(dir.is_dir());
    // Gli errori tornano come AppError.
    assert_eq!(x.mkdir(&dir).unwrap_err().code, ErrorCode::AlreadyExists);

    let src = d.path().join("a.bin");
    let data: Vec<u8> = (0..3 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    fs::write(&src, &data).unwrap();
    let mut seen = Vec::new();
    x.copy_file(&src, &dir.join("a.bin"), false, true, &mut |n| {
        seen.push(n);
        true
    })
    .unwrap();
    assert_eq!(fs::read(dir.join("a.bin")).unwrap(), data);
    assert!(seen.len() > 1 && seen.last() == Some(&(data.len() as u64)), "{seen:?}");

    // `Cancel` interrompe la copia in corso: nessuna destinazione, l'helper resta utilizzabile.
    let big = d.path().join("big.bin");
    fs::write(&big, vec![7u8; 64 * 1024 * 1024]).unwrap();
    let err = x.copy_file(&big, &dir.join("big.bin"), false, false, &mut |_| false).unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(!dir.join("big.bin").exists());

    x.rename(&dir.join("a.bin"), &dir.join("b.bin")).unwrap();
    x.move_path(&dir.join("b.bin"), &d.path().join("c.bin"), false).unwrap();
    x.delete(&d.path().join("c.bin"), false, true).unwrap();
    assert!(!d.path().join("c.bin").exists());
    // L'helper valida da sé i percorsi: il client li manda così come sono.
    assert_eq!(x.mkdir(Path::new(r"relativo\x")).unwrap_err().code, ErrorCode::InvalidRequest);

    // Pipe chiusa (app terminata) → l'helper esce subito, senza aspettare i 60 s di inattività.
    drop(x);
    assert!(wait_exit(&mut child, T).is_some_and(|s| s.success()));
}

#[test]
fn wrong_token_is_rejected() {
    let mut child = None;
    let res = ElevatedExecutor::spawn_with(|pipe, _token| {
        let c = helper(pipe, &"00".repeat(32), None);
        let pid = c.id();
        child = Some(c);
        Ok(pid)
    });
    let err = res.err().expect("token errato accettato");
    assert_eq!((err.code, err.detail.as_deref()), (ErrorCode::AccessDenied, Some("helper_auth_failed")));
    // Connessione chiusa dall'app: l'helper non riceve richieste ed esce.
    assert!(wait_exit(child.as_mut().unwrap(), T).is_some());
}

/// Prova due cose:
/// 1. un processo diverso da quello lanciato, anche se conosce nome della pipe **e** token, viene
///    rifiutato dal controllo `GetNamedPipeClientProcessId` (qui il "lanciato" è il processo di test);
/// 2. con l'helper legittimo connesso, un secondo processo non riesce nemmeno ad aprire la pipe
///    (nessuna istanza libera: `ERROR_PIPE_BUSY`) e la sessione legittima continua a funzionare.
///
/// Non prova l'ACL (stesso utente), il limite di istanze in sé (l'app non ne crea una seconda) né
/// il rifiuto dei client remoti.
#[test]
fn foreign_process_cannot_connect() {
    let mut foreign = None;
    let res = ElevatedExecutor::spawn_with(|pipe, token| {
        foreign = Some(helper(pipe, token, None));
        Ok(std::process::id())
    });
    let err = res.err().expect("client estraneo accettato");
    assert_eq!((err.code, err.detail.as_deref()), (ErrorCode::AccessDenied, Some("helper_pid_mismatch")));
    assert!(wait_exit(foreign.as_mut().unwrap(), T).is_some());

    let d = tempfile::tempdir().unwrap();
    let (x, mut child, pipe, token) = start(None);
    let mut intruder = helper(&pipe, &token, None);
    let st = wait_exit(&mut intruder, T).expect("l'intruso deve uscire");
    assert_eq!(st.code(), Some(1), "l'intruso non deve aprire la pipe");
    x.mkdir(&d.path().join("ancora")).unwrap();
    drop(x);
    assert!(wait_exit(&mut child, T).is_some());
}

#[test]
fn helper_exits_when_idle() {
    let d = tempfile::tempdir().unwrap();
    let (x, mut child, _, _) = start(Some(1000));
    let t = Instant::now();
    let st = wait_exit(&mut child, T).expect("l'helper inattivo deve uscire");
    assert!(st.success());
    assert!(t.elapsed() >= Duration::from_millis(500), "{:?}", t.elapsed());
    // Dopo l'uscita le operazioni falliscono subito, senza bloccarsi.
    assert!(x.mkdir(&d.path().join("x")).is_err());
}
