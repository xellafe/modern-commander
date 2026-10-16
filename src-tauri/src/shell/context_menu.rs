//! Menu contestuale nativo di Explorer sugli elementi selezionati.

use super::{Selection, hr_error, on_main_thread};
use crate::error::AppError;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Shell::{
    CMF_NORMAL, CMINVOKECOMMANDINFO, DefSubclassProc, IContextMenu, IContextMenu3, IShellFolder,
    RemoveWindowSubclass, SHBindToParent, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, HMENU, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{Interface, PCSTR};

const ID_FIRST: u32 = 1;
const ID_LAST: u32 = 0x7FFF;
const SUBCLASS_ID: usize = 0x4D43; // "MC"

/// Mostra il menu contestuale della shell; `x`/`y` sono coordinate client in pixel fisici.
#[tauri::command]
pub async fn show_context_menu(
    window: tauri::Window,
    paths: Vec<String>,
    x: i32,
    y: i32,
) -> Result<(), AppError> {
    on_main_thread(window, move |hwnd| show(hwnd, &paths, x, y)).await
}

fn show(hwnd: HWND, paths: &[String], x: i32, y: i32) -> Result<(), AppError> {
    let sel = Selection::parse(paths)?;
    let fail = |e: windows::core::Error| hr_error(&e, None);
    unsafe {
        let folder: IShellFolder = SHBindToParent(sel.first(), None).map_err(fail)?;
        let menu: IContextMenu = folder
            .GetUIObjectOf(hwnd, &sel.children(), None)
            .map_err(fail)?;
        let hmenu = CreatePopupMenu().map_err(fail)?;
        let result = track_and_invoke(&menu, hmenu, hwnd, x, y);
        let _ = DestroyMenu(hmenu);
        result
    }
}

fn track_and_invoke(
    menu: &IContextMenu,
    hmenu: HMENU,
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Result<(), AppError> {
    let fail = |e: windows::core::Error| hr_error(&e, None);
    unsafe {
        menu.QueryContextMenu(hmenu, 0, ID_FIRST, ID_LAST, CMF_NORMAL)
            .ok()
            .map_err(fail)?;
        let mut pt = POINT { x, y };
        let _ = ClientToScreen(hwnd, &mut pt);

        // I sottomenu dinamici (es. "Apri con", verificato) si popolano solo se i messaggi del
        // menu arrivano all'IContextMenu3: la finestra proprietaria va sottoclassata per la
        // durata del menu.
        let menu3: Option<IContextMenu3> = menu.cast().ok();
        if let Some(m) = &menu3 {
            let _ = SetWindowSubclass(
                hwnd,
                Some(forward_menu_msg),
                SUBCLASS_ID,
                m.as_raw() as usize,
            );
        }
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
        let cmd = TrackPopupMenuEx(hmenu, flags.0, pt.x, pt.y, hwnd, None).0 as u32;
        if menu3.is_some() {
            let _ = RemoveWindowSubclass(hwnd, Some(forward_menu_msg), SUBCLASS_ID);
        }
        if cmd < ID_FIRST {
            return Ok(()); // menu chiuso senza scelta
        }

        let info = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR((cmd - ID_FIRST) as usize as *const u8), // MAKEINTRESOURCEA(offset)
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        menu.InvokeCommand(&info).map_err(fail)
    }
}

unsafe extern "system" fn forward_menu_msg(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if matches!(
        msg,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        let raw = data as *mut core::ffi::c_void;
        // Riferimento preso in prestito: l'IContextMenu3 resta vivo finché il menu è aperto.
        if let Some(menu3) = unsafe { IContextMenu3::from_raw_borrowed(&raw) } {
            let mut result = LRESULT(0);
            if unsafe { menu3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) }.is_ok() {
                return result;
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
