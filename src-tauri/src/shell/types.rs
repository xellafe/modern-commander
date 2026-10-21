//! Descrizione di Windows del tipo di file ("Documento di testo", "Cartella di file") per estensione.
//! Con `SHGFI_USEFILEATTRIBUTES` la shell risponde dal solo nome fittizio: niente accesso al disco.

use crate::error::AppError;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW};
use windows::core::HSTRING;

/// Chiave `"/"` = cartella, `""` = file senza estensione, altrimenti l'estensione senza punto.
/// None se la chiave non è un'estensione valida o la shell non risponde.
pub fn type_name(key: &str) -> Option<String> {
    let (name, attr) = match key {
        "/" => ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY),
        "" => ("file".to_string(), FILE_ATTRIBUTE_NORMAL),
        _ if key.chars().any(|c| "\\/:*?\"<>|\0".contains(c)) => return None,
        _ => (format!("file.{key}"), FILE_ATTRIBUTE_NORMAL),
    };
    let mut info = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            &HSTRING::from(name),
            attr,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES,
        )
    };
    let len = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(info.szTypeName.len());
    (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&info.szTypeName[..len]))
}

/// Testi nello stesso ordine di `keys`; `""` dove non c'è risposta.
#[tauri::command]
pub async fn type_names(keys: Vec<String>) -> Result<Vec<String>, AppError> {
    crate::fs::blocking(move || unsafe {
        // Come per le miniature: S_FALSE (già inizializzato) va bilanciato, un errore no.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let out = keys.iter().map(|k| type_name(k).unwrap_or_default()).collect();
        if com {
            CoUninitialize();
        }
        out
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_and_extensions_have_type_names() {
        let folder = type_name("/").unwrap();
        let txt = type_name("txt").unwrap();
        assert!(!folder.is_empty() && !txt.is_empty() && folder != txt); // testi localizzati: niente confronto letterale
        assert!(type_name("").is_some_and(|s| !s.is_empty()));
        assert_eq!(type_name("a\\b"), None);
        assert_eq!(type_name("x:y"), None);
    }
}
