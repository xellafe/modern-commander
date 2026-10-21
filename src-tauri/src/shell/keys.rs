//! Etichette dei tasti nel layout di tastiera attivo: le scorciatoie sono legate al virtual key, e solo Windows
//! sa quale carattere porta quel tasto (`VK_OEM_3` è ò sul layout italiano, ` su quello americano).

use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_CHAR, MapVirtualKeyW};

/// Il carattere non modificato del virtual key `vk` nel layout attivo; None se il tasto non ne produce.
pub fn key_label(vk: u32) -> Option<String> {
    let mapped = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) };
    // Il bit alto segna un tasto morto (accento da comporre): il carattere sta nei 16 bit bassi.
    char::from_u32(mapped & 0xFFFF).filter(|c| *c != '\0').map(|c| c.to_string())
}

#[tauri::command]
pub fn key_labels(vks: Vec<u32>) -> Vec<Option<String>> {
    vks.into_iter().map(key_label).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_have_a_label_and_function_keys_none() {
        assert_eq!(key_label(0x41).as_deref(), Some("A")); // VK_A in ogni layout latino
        assert_eq!(key_label(0x70), None); // VK_F1
        assert!(key_label(0xC0).is_some()); // VK_OEM_3: il carattere dipende dal layout
    }
}
