//! Apertura dei file: programma associato (verbo "open" della shell) o eseguibile scelto.

use super::{check_shell_name, detail_error, hr_error};
use crate::error::{AppError, ErrorCode};
use std::path::Path;
use std::process::Command;
use windows::Win32::Foundation::{ERROR_NO_ASSOCIATION, HWND};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Apre `path` (forma normale, non `\\?\`: la shell non la accetta) come il doppio click di Explorer.
///
/// Gira su un thread STA dedicato e di breve durata, non sul thread della UI: un avvio via DDE o una
/// condivisione di rete lenta non bloccano la finestra né l'IPC. La finestra principale fa da
/// proprietaria per "Apri con" e per l'eventuale UAC. `window` è iniettato da Tauri: dal frontend
/// l'argomento resta solo `path`.
#[tauri::command]
pub async fn open_path(window: tauri::Window, path: String) -> Result<(), AppError> {
    let owner = window.hwnd().map_err(detail_error)?.0 as isize; // HWND non è Send
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    std::thread::spawn(move || {
        let _ = tx.blocking_send(open_on_sta(HWND(owner as *mut _), &path));
    });
    rx.recv()
        .await
        .unwrap_or_else(|| Err(detail_error("open thread dropped")))
}

/// `SEE_MASK_NOASYNC`: il thread attende la fine dell'avvio (poi esce), così l'errore arriva qui.
/// `SEE_MASK_FLAG_NO_UI`: niente finestre d'errore della shell, l'unica segnalazione è il notice del
/// frontend. Senza associazione si riprova col verbo `openas`, cioè la finestra "Apri con".
fn open_on_sta(owner: HWND, path: &str) -> Result<(), AppError> {
    check_shell_name(path)?;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let res = match shell_execute(owner, w!("open"), path) {
            Err(e) if e.code() == ERROR_NO_ASSOCIATION.to_hresult() => {
                shell_execute(owner, w!("openas"), path)
            }
            r => r,
        };
        CoUninitialize();
        res.map_err(|e| hr_error(&e, Some(Path::new(path))))
    }
}

fn shell_execute(owner: HWND, verb: PCWSTR, path: &str) -> windows::core::Result<()> {
    let file = HSTRING::from(path);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        hwnd: owner,
        lpVerb: verb,
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
}

/// Apre `path` con `exe` (un nome senza percorso si cerca nel PATH), senza shell e senza attendere.
/// Ricerca nel PATH e `CreateProcess` girano fuori dal thread della UI.
#[tauri::command]
pub async fn open_with(exe: String, path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        editor_command(&exe, &path)?
            .spawn()
            .map(drop)
            .map_err(|e| AppError::from_io(&e, Path::new(exe.trim())))
    })
    .await
    .unwrap_or_else(|e| Err(detail_error(e)))
}

/// Il percorso passa come unico argomento: lo quota `Command`, nessun interprete lo rilegge.
fn editor_command(exe: &str, path: &str) -> Result<Command, AppError> {
    let exe = exe.trim();
    if exe.is_empty() || path.is_empty() {
        return Err(AppError::new(ErrorCode::InvalidRequest, None));
    }
    check_shell_name(path)?; // anche l'editor aprirebbe il "gemello"
    let mut cmd = Command::new(exe);
    cmd.arg(path);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn editor_command_validates_and_passes_the_path_as_one_argument() {
        for (exe, path) in [("", "C:\\a.txt"), ("  ", "C:\\a.txt"), ("notepad.exe", "")] {
            let err = editor_command(exe, path).err().unwrap();
            assert_eq!(err.code, ErrorCode::InvalidRequest, "{exe:?} {path:?}");
        }
        let cmd = editor_command(" notepad.exe ", "C:\\con spazi\\a b.txt").unwrap();
        assert_eq!(cmd.get_program(), "notepad.exe");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["C:\\con spazi\\a b.txt"]);
    }

    /// "a.exe." aperto dalla shell (o dall'editor) diventerebbe "a.exe": si rifiuta prima.
    #[test]
    fn names_the_shell_would_alter_are_refused() {
        for path in ["C:\\x\\a.exe.", "C:\\x\\a.txt ", "C:\\x.\\a.txt"] {
            let e = editor_command("notepad.exe", path).err().unwrap();
            assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::InvalidName, Some("shell_unsupported_name")), "{path}");
            let e = open_on_sta(HWND::default(), path).err().unwrap();
            assert_eq!((e.code, e.detail.as_deref(), e.path.as_deref()), (ErrorCode::InvalidName, Some("shell_unsupported_name"), Some(path)));
        }
    }

    /// Con `FLAG_NO_UI` un file mancante torna come errore senza finestre della shell (che qui,
    /// con `NOASYNC`, bloccherebbero il test).
    #[test]
    fn open_missing_file_reports_not_found_without_ui() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        let missing = missing.to_str().unwrap();
        let err = open_on_sta(HWND::default(), missing).err().unwrap();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.path.as_deref(), Some(missing));
    }
}
