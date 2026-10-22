//! Integrazione con la shell di Windows: menu contestuale, proprietà, drag & drop.
//!
//! Threading COM: tutte le chiamate avvengono sul thread principale tramite `on_main_thread`.
//! Quel thread è già STA (wry chiama `CoInitializeEx(COINIT_APARTMENTTHREADED)`) e
//! OLE-inizializzato (tao chiama `OleInitialize` quando il drag & drop della finestra è attivo,
//! come di default): qui non si inizializza COM una seconda volta.

pub mod context_menu;
pub mod drag;
pub mod keys;
pub mod open;
pub mod properties;
pub mod thumbnail;
pub mod types;

use crate::error::{AppError, ErrorCode};
use crate::paths::has_trailing_dot_or_space;
use std::path::Path;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILClone, ILFindLastID, ILFree, ILIsParent, ILRemoveLastID, SHCreateDataObject,
    SHParseDisplayName,
};
use windows::core::HSTRING;

/// Errore COM/Win32: i codici Win32 noti mantengono il loro `ErrorCode`, l'HRESULT va in `detail`.
pub(crate) fn hr_error(err: &windows::core::Error, path: Option<&Path>) -> AppError {
    let hr = err.code().0 as u32;
    let code = match hr {
        0x8007_04C7 => ErrorCode::Cancelled, // ERROR_CANCELLED
        _ if hr >> 16 == 0x8007 => {
            let io = std::io::Error::from_raw_os_error((hr & 0xFFFF) as i32);
            AppError::from_io(&io, Path::new("")).code
        }
        _ => ErrorCode::Io,
    };
    AppError {
        detail: Some(format!("HRESULT 0x{hr:08X}")),
        ..AppError::new(code, path)
    }
}

/// La shell toglie punto e spazio finali dai nomi: "x." diventerebbe "x", e il menu, le Proprietà,
/// il drag o l'apertura agirebbero sul "gemello". Questi percorsi si rifiutano.
pub(crate) fn check_shell_name(path: &str) -> Result<(), AppError> {
    if has_trailing_dot_or_space(Path::new(path)) {
        return Err(AppError {
            detail: Some("shell_unsupported_name".into()),
            ..AppError::new(ErrorCode::InvalidName, Some(Path::new(path)))
        });
    }
    Ok(())
}

fn detail_error(detail: impl ToString) -> AppError {
    AppError {
        detail: Some(detail.to_string()),
        ..AppError::new(ErrorCode::Io, None)
    }
}

/// Esegue `f` sul thread principale con l'HWND della finestra e ne attende il risultato.
/// Il comando deve essere `async`: un comando sincrono gira già sul thread principale, dove
/// `run_on_main_thread` eseguirebbe `f` subito, dentro il callback IPC di WebView2.
pub(crate) async fn on_main_thread<F>(window: tauri::Window, f: F) -> Result<(), AppError>
where
    F: FnOnce(HWND) -> Result<(), AppError> + Send + 'static,
{
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = tx.try_send(w.hwnd().map_err(detail_error).and_then(f));
        })
        .map_err(detail_error)?;
    rx.recv()
        .await
        .unwrap_or_else(|| Err(detail_error("main thread task dropped")))
}

/// PIDL assoluto allocato dalla shell, liberato al drop.
struct Pidl(*mut ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) }
    }
}

/// Elementi della shell con la loro cartella padre comune.
pub(crate) struct Selection {
    parent: Pidl,
    items: Vec<Pidl>,
}

impl Selection {
    /// Risolve i percorsi in PIDL; tutti devono avere la stessa cartella padre.
    pub fn parse(paths: &[String]) -> Result<Self, AppError> {
        // Le posizioni remote non arrivano alla shell: `UNSUPPORTED` se ce n'è anche una sola.
        for p in paths {
            crate::fs::local_only(p)?;
        }
        let invalid = |p: Option<&str>| AppError::new(ErrorCode::InvalidRequest, p.map(Path::new));
        let items = paths
            .iter()
            .map(|p| {
                check_shell_name(p)?;
                let mut pidl = std::ptr::null_mut();
                unsafe { SHParseDisplayName(&HSTRING::from(p.as_str()), None, &mut pidl, 0, None) }
                    .map(|_| Pidl(pidl))
                    .map_err(|e| hr_error(&e, Some(Path::new(p))))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let first = items.first().ok_or_else(|| invalid(None))?;
        let parent = Pidl(unsafe { ILClone(first.0) });
        if parent.0.is_null() {
            return Err(detail_error("ILClone"));
        }
        let _ = unsafe { ILRemoveLastID(Some(parent.0)) };
        for (item, p) in items.iter().zip(paths) {
            if !unsafe { ILIsParent(parent.0, item.0, true) }.as_bool() {
                return Err(invalid(Some(p)));
            }
        }
        Ok(Self { parent, items })
    }

    /// PIDL figli relativi alla cartella padre (puntano dentro i PIDL assoluti).
    pub fn children(&self) -> Vec<*const ITEMIDLIST> {
        self.items
            .iter()
            .map(|i| unsafe { ILFindLastID(i.0) } as *const _)
            .collect()
    }

    /// PIDL assoluto del primo elemento, per `SHBindToParent`.
    pub fn first(&self) -> *const ITEMIDLIST {
        self.items[0].0
    }

    /// `IDataObject` della shell (CF_HDROP e formati shell) sugli elementi.
    pub fn data_object(&self) -> Result<IDataObject, AppError> {
        unsafe { SHCreateDataObject(Some(self.parent.0), Some(&self.children()), None) }
            .map_err(|e| hr_error(&e, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    fn com() {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    }

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn selection_requires_common_parent() {
        com();
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        for p in [
            dir.path().join("a.txt"),
            dir.path().join("b.txt"),
            sub.join("c.txt"),
        ] {
            std::fs::write(p, "x").unwrap();
        }
        let sel = Selection::parse(&[s(&dir.path().join("a.txt")), s(&dir.path().join("b.txt"))])
            .unwrap();
        assert_eq!(sel.children().len(), 2);
        assert!(sel.data_object().is_ok());

        let err = Selection::parse(&[s(&dir.path().join("a.txt")), s(&sub.join("c.txt"))])
            .err()
            .unwrap();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        assert_eq!(
            Selection::parse(&[]).err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }

    /// La shell toglie punto o spazio finale: "x." diventerebbe "x", un altro file.
    #[test]
    fn names_the_shell_would_alter_are_refused() {
        com();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x"), "gemello").unwrap();
        for odd in ["x.", "x "] {
            let p = dir.path().join(odd);
            std::fs::write(crate::paths::to_extended(&p), "strano").unwrap();
            let err = Selection::parse(&[s(&dir.path().join("x")), s(&p)]).err().unwrap();
            assert_eq!((err.code, err.detail.as_deref()), (ErrorCode::InvalidName, Some("shell_unsupported_name")));
            assert_eq!(err.path.as_deref(), Some(s(&p).as_str()));
        }
    }

    #[test]
    fn missing_path_is_not_found() {
        com();
        let dir = tempfile::tempdir().unwrap();
        let missing = s(&dir.path().join("nope.txt"));
        let err = Selection::parse(std::slice::from_ref(&missing)).err().unwrap();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.path.as_deref(), Some(missing.as_str()));
        assert_eq!(err.detail.as_deref(), Some("HRESULT 0x80070002"));
    }

    #[test]
    fn hresult_mapping() {
        let code = |hr: u32| {
            hr_error(
                &windows::core::Error::from_hresult(windows::core::HRESULT(hr as i32)),
                None,
            )
            .code
        };
        assert_eq!(code(0x8007_0005), ErrorCode::AccessDenied);
        assert_eq!(code(0x8007_04C7), ErrorCode::Cancelled);
        assert_eq!(code(0x8000_4005), ErrorCode::Io);
    }
}
