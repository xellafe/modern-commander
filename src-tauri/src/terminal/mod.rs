//! Terminali integrati: una sessione ConPTY (`portable-pty`) per scheda del pannello terminale.
//!
//! Il nucleo (`spawn_session`, `Session`) non dipende da Tauri e si prova con `cmd.exe` vero; i comandi `term_*`
//! lo collegano al `Channel` dell'output e all'evento `term-exit`. Dal frontend arriva solo un `ShellKind`: mai
//! un percorso di programma o degli argomenti.

use crate::error::{AppError, ErrorCode};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, State};

/// Colonne e righe massime accettate da `term_spawn` e `term_resize`.
const MAX_DIM: u16 = 1000;
/// Dimensione dei blocchi letti dal terminale: ogni blocco parte così com'è verso il frontend.
const READ_BLOCK: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Pwsh,
    Powershell,
    Cmd,
    Wsl,
}

impl ShellKind {
    /// In ordine di dichiarazione: è l'ordine in cui `term_shells` li elenca.
    const ALL: [ShellKind; 4] = [ShellKind::Pwsh, ShellKind::Powershell, ShellKind::Cmd, ShellKind::Wsl];

    fn exe(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "pwsh.exe",
            ShellKind::Powershell => "powershell.exe",
            ShellKind::Cmd => "cmd.exe",
            ShellKind::Wsl => "wsl.exe",
        }
    }
}

/// Le shell il cui eseguibile si trova nel PATH, nell'ordine dell'enum.
pub fn available_shells() -> Vec<ShellKind> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // `symlink_metadata` e non `exists`: gli alias di app dello Store (pwsh) sono reparse point che non si aprono.
    ShellKind::ALL.into_iter().filter(|k| dirs.iter().any(|d| std::fs::symlink_metadata(d.join(k.exe())).is_ok())).collect()
}

/// Pwsh, poi Powershell, poi Cmd (sempre presente su Windows).
pub fn default_shell(available: &[ShellKind]) -> ShellKind {
    [ShellKind::Pwsh, ShellKind::Powershell].into_iter().find(|k| available.contains(k)).unwrap_or(ShellKind::Cmd)
}

pub fn shell_command(kind: ShellKind, cwd: &Path) -> CommandBuilder {
    let mut c = CommandBuilder::new(kind.exe());
    match kind {
        ShellKind::Pwsh | ShellKind::Powershell => {
            c.arg("-NoLogo");
            c.cwd(cwd);
        }
        ShellKind::Cmd => c.cwd(cwd),
        ShellKind::Wsl => {
            c.arg("--cd");
            c.arg(cwd);
        }
    }
    c
}

/// La cartella richiesta se esiste; altrimenti (vuota, scomparsa, percorso virtuale `drives:`) `%USERPROFILE%`.
fn resolve_cwd(cwd: &str) -> PathBuf {
    let p = Path::new(cwd);
    if p.is_dir() {
        return p.to_path_buf();
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\"))
}

fn pty_size(cols: u16, rows: u16) -> Result<PtySize, AppError> {
    if !(1..=MAX_DIM).contains(&cols) || !(1..=MAX_DIM).contains(&rows) {
        return Err(AppError {
            detail: Some(format!("dimensioni non valide: {cols}x{rows}")),
            ..AppError::new(ErrorCode::InvalidRequest, None)
        });
    }
    Ok(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
}

fn io_error(e: impl ToString) -> AppError {
    AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) }
}

/// Una sessione viva. Ogni parte ha il suo lucchetto: una scrittura bloccata non ferma `kill` né il ridimensionamento.
pub struct Session {
    /// `None` dopo l'uscita del processo: lasciarlo cadere chiude la pseudo-console, e il lettore vede la fine del flusso.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Alzato dal lettore appena prima di `on_exit`.
    done: AtomicBool,
}

impl Session {
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        match self.writer.lock().unwrap().as_mut() {
            Some(w) => w.write_all(data).and_then(|()| w.flush()),
            None => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        match self.master.lock().unwrap().as_ref() {
            Some(m) => m.resize(size).map_err(io::Error::other),
            None => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    /// Ferma il processo; l'uscita arriva poi dal thread di lettura. Un processo già finito non è un errore.
    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }

    fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
}

/// Avvia `command` in una nuova pseudo-console. Un thread legge a blocchi e li passa a `on_output` (che risponde
/// `false` se nessuno ascolta più: il processo viene fermato e il flusso si svuota comunque); un altro aspetta il
/// processo. A flusso finito `on_exit` riceve il codice di uscita (`None` se non si è potuto leggere).
pub fn spawn_session(
    command: CommandBuilder,
    size: PtySize,
    mut on_output: impl FnMut(&[u8]) -> bool + Send + 'static,
    on_exit: impl FnOnce(Option<u32>) + Send + 'static,
) -> io::Result<Arc<Session>> {
    let pair = native_pty_system().openpty(size).map_err(io::Error::other)?;
    let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pair.master.take_writer().map_err(io::Error::other)?;
    let mut child = pair.slave.spawn_command(command).map_err(io::Error::other)?;
    drop(pair.slave); // il master e lo slave condividono la pseudo-console: va chiusa quando cadono entrambi
    let session = Arc::new(Session {
        master: Mutex::new(Some(pair.master)),
        writer: Mutex::new(Some(writer)),
        killer: Mutex::new(child.clone_killer()),
        done: AtomicBool::new(false),
    });

    // ConPTY non chiude il flusso di uscita alla morte del processo ma solo con la pseudo-console: è qui che si chiude.
    let (code_tx, code_rx) = mpsc::channel();
    let s = session.clone();
    thread::spawn(move || {
        let code = child.wait().ok().map(|st| st.exit_code());
        s.master.lock().unwrap().take();
        let _ = code_tx.send(code);
        s.writer.lock().unwrap().take(); // dopo: se una scrittura è rimasta appesa non trattiene il lettore
    });

    let s = session.clone();
    thread::spawn(move || {
        let mut buf = [0u8; READ_BLOCK];
        let mut listening = true;
        // Un errore di lettura (pipe rotta) vale come fine del flusso.
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if listening && !on_output(&buf[..n]) {
                listening = false;
                s.kill();
            }
        }
        s.kill(); // flusso chiuso con il processo ancora vivo: non si resta ad aspettarlo per sempre
        let code = code_rx.recv().ok().flatten();
        s.done.store(true, Ordering::SeqCst);
        on_exit(code);
    });
    Ok(session)
}

#[derive(Default)]
struct Registry {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Session>>>,
}

/// Stato Tauri: id → sessione. Il lucchetto del registro non resta mai preso durante I/O.
#[derive(Default, Clone)]
pub struct Terminals(Arc<Registry>);

impl Terminals {
    fn get(&self, id: u32) -> Result<Arc<Session>, AppError> {
        self.0.sessions.lock().unwrap().get(&id).cloned().ok_or_else(|| AppError {
            detail: Some(format!("terminale sconosciuto: {id}")),
            ..AppError::new(ErrorCode::InvalidRequest, None)
        })
    }

    fn remove(&self, id: u32) {
        self.0.sessions.lock().unwrap().remove(&id);
    }

    pub fn write(&self, id: u32, data: &[u8]) -> Result<(), AppError> {
        self.get(id)?.write(data).map_err(io_error)
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), AppError> {
        let size = pty_size(cols, rows)?;
        self.get(id)?.resize(size).map_err(io_error)
    }

    pub fn kill(&self, id: u32) -> Result<(), AppError> {
        self.get(id)?.kill();
        Ok(())
    }

    /// All'uscita dell'app: ferma tutti i processi (la pseudo-console si chiude con loro).
    pub fn kill_all(&self) {
        let all: Vec<_> = self.0.sessions.lock().unwrap().values().cloned().collect();
        for s in all {
            s.kill();
        }
    }
}

#[derive(Serialize, Clone)]
struct ExitEvent {
    id: u32,
    code: Option<u32>,
}

#[tauri::command]
pub async fn term_shells() -> Result<Vec<ShellKind>, AppError> {
    crate::fs::blocking(available_shells).await
}

/// Apre un terminale; l'output arriva a blocchi grezzi su `on_output` (`ArrayBuffer` in JS). A fine sessione
/// parte l'evento `term-exit` `{ id, code }`.
///
/// Contratto per il frontend: il thread di lettura parte prima che la risposta con l'id arrivi, quindi i primi
/// blocchi (con la richiesta `ESC[6n`, a cui la shell resta appesa finché il terminale non risponde) e perfino
/// `term-exit` possono raggiungere JS prima che `term_spawn` si risolva. Chi chiama deve collegare l'input del
/// terminale PRIMA di chiamare, tenendo in coda le scritture finché l'id non è noto. Gli ultimi blocchi possono
/// arrivare anche dopo `term-exit`: alla sua ricezione il terminale non va distrutto.
#[tauri::command]
pub async fn term_spawn(
    app: AppHandle,
    terminals: State<'_, Terminals>,
    shell: Option<ShellKind>,
    cwd: String,
    cols: u16,
    rows: u16,
    on_output: Channel<InvokeResponseBody>,
) -> Result<u32, AppError> {
    let size = pty_size(cols, rows)?;
    let terminals = terminals.inner().clone();
    crate::fs::blocking(move || {
        let kind = shell.unwrap_or_else(|| default_shell(&available_shells()));
        let command = shell_command(kind, &resolve_cwd(&cwd));
        let id = terminals.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let t = terminals.clone();
        let session = spawn_session(
            command,
            size,
            // Canale chiuso (finestra distrutta): `false`, e il lettore ferma il processo.
            move |b| on_output.send(InvokeResponseBody::Raw(b.to_vec())).is_ok(),
            move |code| {
                t.remove(id);
                let _ = app.emit("term-exit", ExitEvent { id, code });
            },
        )
        .map_err(io_error)?;
        terminals.0.sessions.lock().unwrap().insert(id, session.clone());
        // Uscita avvenuta prima dell'inserimento: il suo `remove` non ha trovato nulla, lo si rifà qui.
        if session.is_done() {
            terminals.remove(id);
        }
        Ok(id)
    })
    .await?
}

// Scrittura, ridimensionamento e chiusura sono sincroni: girano nell'ordine di arrivo (i tasti non si scambiano)
// e il lavoro è un pezzo di pipe o una chiamata alla console.
#[tauri::command]
pub fn term_write(terminals: State<'_, Terminals>, id: u32, data: String) -> Result<(), AppError> {
    terminals.write(id, data.as_bytes())
}

#[tauri::command]
pub fn term_resize(terminals: State<'_, Terminals>, id: u32, cols: u16, rows: u16) -> Result<(), AppError> {
    terminals.resize(id, cols, rows)
}

#[tauri::command]
pub fn term_kill(terminals: State<'_, Terminals>, id: u32) -> Result<(), AppError> {
    terminals.kill(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::CommandBuilder;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(5);

    const DSR: &[u8] = b"\x1b[6n";
    /// Risposta con la posizione del cursore: quella che xterm.js dà da sé alla richiesta `ESC[6n` con cui ConPTY
    /// apre ogni sessione (senza, conhost non va avanti).
    const CURSOR_REPORT: &[u8] = b"\x1b[1;1R";

    /// Avvia `cmd.exe` con gli argomenti dati: restituisce la sessione, i byte raccolti e il canale del codice di uscita.
    /// La risposta alla richiesta del cursore la dà un thread di appoggio, perché la sessione nasce dopo la callback.
    fn run_cmd(args: &[&str]) -> (Arc<Session>, Arc<Mutex<Vec<u8>>>, Receiver<Option<u32>>) {
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(args);
        let out = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel();
        let (dsr_tx, dsr_rx) = channel();
        let sink = out.clone();
        let session = spawn_session(
            cmd,
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |b| {
                if b.windows(DSR.len()).any(|w| w == DSR) {
                    let _ = dsr_tx.send(());
                }
                sink.lock().unwrap().extend_from_slice(b);
                true
            },
            move |code| {
                let _ = tx.send(code);
            },
        )
        .unwrap();
        let s = session.clone();
        std::thread::spawn(move || {
            while dsr_rx.recv().is_ok() {
                let _ = s.write(CURSOR_REPORT);
            }
        });
        (session, out, rx)
    }

    fn text(out: &Mutex<Vec<u8>>) -> String {
        String::from_utf8_lossy(&out.lock().unwrap()).into_owned()
    }

    #[test]
    fn shell_commands() {
        let c = shell_command(ShellKind::Wsl, Path::new(r"C:\x"));
        assert_eq!(c.get_argv(), &["wsl.exe", "--cd", r"C:\x"].map(OsString::from));
        assert_eq!(shell_command(ShellKind::Pwsh, Path::new(r"C:\")).get_argv(), &["pwsh.exe", "-NoLogo"].map(OsString::from));
        assert_eq!(shell_command(ShellKind::Powershell, Path::new(r"C:\")).get_argv(), &["powershell.exe", "-NoLogo"].map(OsString::from));
        assert_eq!(shell_command(ShellKind::Cmd, Path::new(r"C:\")).get_argv(), &["cmd.exe"].map(OsString::from));
        // Le shell Windows partono nella cartella; per WSL la passa `--cd`.
        assert_eq!(shell_command(ShellKind::Cmd, Path::new(r"C:\x")).get_cwd(), Some(&OsString::from(r"C:\x")));
    }

    #[test]
    fn default_shell_priority() {
        assert_eq!(default_shell(&[ShellKind::Cmd, ShellKind::Powershell]), ShellKind::Powershell);
        assert_eq!(default_shell(&[ShellKind::Cmd, ShellKind::Pwsh, ShellKind::Powershell]), ShellKind::Pwsh);
        assert_eq!(default_shell(&[]), ShellKind::Cmd);
    }

    #[test]
    fn shell_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_value(ShellKind::Powershell).unwrap(), "powershell");
        assert_eq!(serde_json::from_value::<ShellKind>(serde_json::json!("wsl")).unwrap(), ShellKind::Wsl);
    }

    #[test]
    fn cwd_falls_back_to_home() {
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        let d = tempfile::tempdir().unwrap();
        assert_eq!(resolve_cwd(d.path().to_str().unwrap()), d.path());
        assert_eq!(resolve_cwd(""), home);
        assert_eq!(resolve_cwd("drives:"), home);
        assert_eq!(resolve_cwd(d.path().join("gone").to_str().unwrap()), home);
    }

    #[test]
    fn size_is_validated() {
        assert!(pty_size(80, 24).is_ok());
        for (c, r) in [(0, 24), (80, 0), (MAX_DIM + 1, 24), (80, MAX_DIM + 1)] {
            assert_eq!(pty_size(c, r).unwrap_err().code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn unknown_id_is_invalid_request() {
        let t = Terminals::default();
        assert_eq!(t.write(9, b"x").unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(t.resize(9, 80, 24).unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(t.kill(9).unwrap_err().code, ErrorCode::InvalidRequest);
        t.kill_all(); // registro vuoto: nessun effetto
    }

    #[test]
    fn pty_runs_a_command_and_reports_exit() {
        let (_s, out, rx) = run_cmd(&["/c", "echo mc-pty-ok"]);
        let code = rx.recv_timeout(TIMEOUT).expect("uscita entro 5 s");
        assert_eq!(code, Some(0));
        assert!(text(&out).contains("mc-pty-ok"), "output: {}", text(&out));
    }

    #[test]
    fn pty_accepts_input() {
        let (s, out, rx) = run_cmd(&[]);
        s.write(b"echo mc-in-%OS%\r\nexit\r\n").unwrap();
        rx.recv_timeout(TIMEOUT).expect("uscita entro 5 s");
        assert!(text(&out).contains("mc-in-Windows_NT"), "output: {}", text(&out));
    }

    #[test]
    fn kill_all_stops_every_registered_session() {
        let t = Terminals::default();
        t.kill_all(); // registro vuoto (primo caricamento della pagina): nessun effetto, nessun panic
        let (a, _oa, rx_a) = run_cmd(&[]);
        let (b, _ob, rx_b) = run_cmd(&[]);
        t.0.sessions.lock().unwrap().extend([(1, a), (2, b)]);
        t.kill_all();
        rx_a.recv_timeout(TIMEOUT).expect("uscita della prima entro 5 s");
        rx_b.recv_timeout(TIMEOUT).expect("uscita della seconda entro 5 s");
    }

    #[test]
    fn kill_ends_the_session() {
        let (s, _out, rx) = run_cmd(&[]);
        s.kill();
        rx.recv_timeout(TIMEOUT).expect("uscita entro 5 s dopo kill");
    }

    #[test]
    fn closed_sink_kills_the_child() {
        // `on_output` che risponde false (webview chiusa): il thread di lettura ferma il processo e conclude.
        let mut cmd = CommandBuilder::new("cmd.exe");
        let (tx, rx) = channel();
        let _s = spawn_session(
            {
                cmd.arg("/k");
                cmd
            },
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            |_| false,
            move |code| {
                let _ = tx.send(code);
            },
        )
        .unwrap();
        rx.recv_timeout(TIMEOUT).expect("uscita entro 5 s con canale chiuso");
    }
}
