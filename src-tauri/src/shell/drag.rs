//! Drag & drop in uscita verso Explorer e altre applicazioni.
//!
//! Il crate `drag` (CrabNebula, MIT/Apache) è stato scartato: porta con sé `windows` 0.52 e
//! `windows-core` 0.58 in aggiunta allo 0.62 già in uso. `SHDoDragDrop` avvolge `DoDragDrop`
//! fornendo `IDropSource` predefinito e immagine di trascinamento della shell.

use super::{Selection, hr_error, on_main_thread};
use crate::error::AppError;
use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::SHDoDragDrop;

/// Avvia un drag modale (copia o spostamento) degli elementi; ritorna a drop concluso o annullato.
#[tauri::command]
pub async fn start_drag(window: tauri::Window, paths: Vec<String>) -> Result<(), AppError> {
    on_main_thread(window, move |hwnd| {
        let data = Selection::parse(&paths)?.data_object()?;
        // Con il pulsante già rilasciato DoDragDrop farebbe subito un drop sotto il cursore.
        let pressed = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
        if !pressed(VK_LBUTTON.0) && !pressed(VK_RBUTTON.0) {
            return Ok(());
        }
        unsafe { SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE) }
            .map(|_| ())
            .map_err(|e| hr_error(&e, None))
    })
    .await
}
