//! Finestra Proprietà della shell.

use super::{Selection, hr_error, on_main_thread};
use crate::error::AppError;
use std::path::Path;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{SHMultiFileProperties, SHOP_FILEPATH, SHObjectProperties};
use windows::core::{HSTRING, PCWSTR};

/// `window` è iniettato da Tauri: dal frontend gli argomenti restano solo `paths`.
#[tauri::command]
pub async fn show_properties(window: tauri::Window, paths: Vec<String>) -> Result<(), AppError> {
    on_main_thread(window, move |hwnd| show(hwnd, &paths)).await
}

fn show(hwnd: HWND, paths: &[String]) -> Result<(), AppError> {
    let sel = Selection::parse(paths)?;
    unsafe {
        if let [path] = paths {
            let ok = SHObjectProperties(
                Some(hwnd),
                SHOP_FILEPATH,
                &HSTRING::from(path.as_str()),
                PCWSTR::null(),
            );
            if ok.as_bool() {
                Ok(())
            } else {
                Err(hr_error(
                    &windows::core::Error::from_thread(),
                    Some(Path::new(path)),
                ))
            }
        } else {
            SHMultiFileProperties(&sel.data_object()?, 0).map_err(|e| hr_error(&e, None))
        }
    }
}
