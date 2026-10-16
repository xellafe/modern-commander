//! Apertura dei file: programma associato (verbo "open" della shell) o eseguibile scelto.

use super::{hr_error, on_main_thread};
use crate::error::{AppError, ErrorCode};
use std::path::Path;
use std::process::Command;
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Apre `path` (forma normale, non `\\?\`: la shell non la accetta) come il doppio click di Explorer.
///
/// Gira sul thread principale, già STA con message loop: la shell può mostrare con la finestra
/// come proprietaria le sue finestre ("Apri con" per i tipi senza associazione, messaggi d'errore).
/// `SEE_MASK_NOASYNC` fa attendere il completamento dell'avvio, così l'errore arriva qui.
/// `window` è iniettato da Tauri: dal frontend l'argomento resta solo `path`.
#[tauri::command]
pub async fn open_path(window: tauri::Window, path: String) -> Result<(), AppError> {
    on_main_thread(window, move |hwnd| {
        let file = HSTRING::from(path.as_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            hwnd,
            lpVerb: w!("open"),
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }.map_err(|e| hr_error(&e, Some(Path::new(&path))))
    })
    .await
}

/// Apre `path` con `exe` (un nome senza percorso si cerca nel PATH), senza shell e senza attendere.
#[tauri::command]
pub fn open_with(exe: String, path: String) -> Result<(), AppError> {
    editor_command(&exe, &path)?
        .spawn()
        .map(drop)
        .map_err(|e| AppError::from_io(&e, Path::new(exe.trim())))
}

/// Il percorso passa come unico argomento: lo quota `Command`, nessun interprete lo rilegge.
fn editor_command(exe: &str, path: &str) -> Result<Command, AppError> {
    let exe = exe.trim();
    if exe.is_empty() || path.is_empty() {
        return Err(AppError::new(ErrorCode::InvalidRequest, None));
    }
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
}
