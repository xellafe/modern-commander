//! Blocco dei componenti di un percorso: le API Win32 seguono giunzioni e collegamenti nei
//! componenti intermedi, quindi chi li sostituisce tra la validazione e l'uso redirige
//! un'operazione elevata. Qui ogni cartella antenata si apre (senza seguire i collegamenti), si
//! rifiutano i reparse point e si tengono gli handle aperti senza `FILE_SHARE_DELETE`: finché
//! vivono, quelle cartelle non si possono rinominare né eliminare né sostituire.

use super::{is, owned, raw, win_io};
use crate::error::{AppError, ErrorCode};
use crate::paths::{normalize_lexical, to_extended};
use std::ffi::{OsStr, OsString};
use std::os::windows::io::OwnedHandle;
use std::path::{Component, Path, PathBuf};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_DELETE_ON_CLOSE,
    FILE_READ_ATTRIBUTES, FILE_SHARE_NONE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_TRAVERSE, GetFileInformationByHandle, OPEN_EXISTING,
};
use windows::core::HSTRING;

/// Handle aperti su ogni cartella da sotto la radice fino al padre di un percorso (e, se richiesto,
/// sull'elemento stesso). Finché vive, quelle cartelle non si possono rinominare né eliminare.
/// Le sentinelle (`.mc-lock-<uuid>`) tengono non vuota la cartella più profonda, che altrimenti
/// si potrebbe trasformare sul posto in giunzione (`FSCTL_SET_REPARSE_POINT` richiede la cartella
/// vuota e solo `FILE_WRITE_ATTRIBUTES`, che la condivisione non esclude).
pub struct PathLock {
    _handles: Vec<OwnedHandle>,
    sentinels: Vec<OsString>,
}

impl PathLock {
    /// `true` se `name` è una delle sentinelle di questo blocco (l'elenco non deve mostrarla).
    pub fn hides(&self, name: &OsStr) -> bool {
        self.sentinels.iter().any(|s| s == name)
    }
}

/// Crea in `dir` il file sentinella: nessuno lo elimina finché l'handle vive, sparisce alla chiusura.
fn sentinel(dir: &Path) -> Result<(OwnedHandle, OsString), AppError> {
    let name = OsString::from(format!(".mc-lock-{}", uuid::Uuid::new_v4()));
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(to_extended(&dir.join(&name)).as_path()),
            DELETE.0 | FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_NONE,
            None,
            CREATE_NEW,
            FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE,
            None,
        )
    }
    .map_err(|e| AppError::from_io(&win_io(e), dir))?;
    Ok((unsafe { owned(h) }, name))
}

fn err(code: ErrorCode, path: &Path, detail: &str) -> AppError {
    AppError { detail: Some(detail.into()), ..AppError::new(code, Some(path)) }
}

fn open(path: &Path, access: u32) -> windows::core::Result<HANDLE> {
    unsafe {
        CreateFileW(
            &HSTRING::from(to_extended(path).as_path()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
}

/// Apre `path` e ne restituisce handle e attributi.
fn open_checked(path: &Path) -> Result<(OwnedHandle, u32), AppError> {
    let h = match open(path, FILE_LIST_DIRECTORY.0) {
        Err(e) if is(&e, ERROR_ACCESS_DENIED) => open(path, FILE_TRAVERSE.0),
        r => r,
    }
    .map_err(|e| AppError::from_io(&win_io(e), path))?;
    let h = unsafe { owned(h) };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(raw(&h), &mut info) }.map_err(|e| AppError::from_io(&win_io(e), path))?;
    Ok((h, info.dwFileAttributes))
}

fn lock(path: &Path, with_item: bool) -> Result<PathLock, AppError> {
    let norm = normalize_lexical(path);
    debug_assert!(norm.is_absolute(), "percorso relativo: la validazione lo rifiuta prima");
    let (mut cur, mut parts) = (PathBuf::new(), Vec::new());
    for c in norm.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => cur.push(c),
            Component::Normal(n) => parts.push(n),
            _ => {}
        }
    }
    let n = parts.len();
    let (mut handles, mut sentinels) = (Vec::with_capacity(n + 2), Vec::new());
    for (i, name) in parts.into_iter().enumerate() {
        let last = i + 1 == n;
        if last && i > 0 {
            // `cur` è il padre, già bloccato: la sentinella lo tiene non vuoto.
            let (h, s) = sentinel(&cur)?;
            handles.push(h);
            sentinels.push(s);
        }
        cur.push(name);
        if last && !with_item {
            break;
        }
        let (h, attrs) = open_checked(&cur)?;
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(err(ErrorCode::Unsupported, &cur, if last { "reparse_point" } else { "reparse_point_in_path" }));
        }
        if !last && attrs & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
            return Err(AppError::new(ErrorCode::NotFound, Some(&cur)));
        }
        handles.push(h);
        if last && attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            let (h, s) = sentinel(&cur)?;
            handles.push(h);
            sentinels.push(s);
        }
    }
    Ok(PathLock { _handles: handles, sentinels })
}

/// Blocca ogni cartella antenata di `path` (la radice dell'unità o della condivisione no).
pub fn lock_ancestors(path: &Path) -> Result<PathLock, AppError> {
    lock(path, false)
}

/// Come `lock_ancestors`, più l'elemento stesso, che non deve essere un reparse point.
pub fn lock_with_item(path: &Path) -> Result<PathLock, AppError> {
    lock(path, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec::{Executor, LocalExecutor};
    use std::fs;

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
        assert!(ok, "mklink /J");
    }

    #[test]
    fn junction_in_the_middle_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        fs::create_dir(&real).unwrap();
        junction(&d.path().join("link"), &real);
        let e = lock_ancestors(&d.path().join(r"link\f.txt")).err().unwrap();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("reparse_point_in_path")));
        assert_eq!(e.path.as_deref().map(Path::new), Some(d.path().join("link").as_path()));
        assert!(lock_ancestors(&real.join("f.txt")).is_ok());
    }

    #[test]
    fn locked_ancestors_cannot_be_renamed_or_swapped() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        fs::create_dir_all(a.join("b")).unwrap();
        let lock = lock_ancestors(&a.join(r"b\f.txt")).unwrap();
        let err = fs::rename(&a, d.path().join("a2")).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(32)); // ERROR_SHARING_VIOLATION
        assert!(fs::rename(a.join("b"), a.join("b2")).is_err());
        drop(lock);
        fs::rename(&a, d.path().join("a2")).unwrap();
    }

    #[test]
    fn primitives_work_under_the_lock() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join(r"a\b");
        fs::create_dir_all(&b).unwrap();
        let f = b.join("f.txt");
        fs::write(&f, b"x").unwrap();
        let mut perm = fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&f, perm).unwrap();
        let x = b.join("x");
        let _lock = lock_ancestors(&x).unwrap();
        let ex = LocalExecutor;
        let (g, h) = (b.join("g.txt"), b.join("h.txt"));
        ex.mkdir(&x).unwrap();
        ex.copy_file(&f, &g, false, false, &mut |_| true).unwrap();
        ex.copy_file(&f, &g, true, false, &mut |_| true).unwrap();
        ex.rename(&g, &h).unwrap();
        ex.move_path(&h, &x.join("h.txt"), false).unwrap();
        ex.delete(&x.join("h.txt"), false, false).unwrap();
        ex.delete(&f, false, true).unwrap();
        ex.delete(&x, true, true).unwrap();
        ex.list_dir(&b).unwrap();
    }

    #[test]
    fn item_lock_refuses_a_link_and_pins_the_item() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        fs::create_dir(&real).unwrap();
        junction(&d.path().join("link"), &real);
        let e = lock_with_item(&d.path().join("link")).err().unwrap();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("reparse_point")));
        let lock = lock_with_item(&real).unwrap();
        assert!(fs::rename(&real, d.path().join("moved")).is_err());
        drop(lock);
    }

    #[test]
    fn sentinel_keeps_the_deepest_folder_non_empty() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        fs::create_dir(&a).unwrap();
        let names = |p: &Path| fs::read_dir(p).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>();
        let lock = lock_ancestors(&a.join("x")).unwrap();
        let n = names(&a);
        assert_eq!(n.len(), 1);
        assert!(lock.hides(&n[0]) && !lock.hides(OsStr::new("altro")));
        assert_eq!(fs::remove_file(a.join(&n[0])).unwrap_err().raw_os_error(), Some(32));
        drop(lock);
        assert!(names(&a).is_empty());
        let lock = lock_with_item(&a).unwrap();
        assert_eq!(names(&a).len(), 1);
        let in_d: Vec<_> = names(d.path()).into_iter().filter(|n| lock.hides(n)).collect();
        assert_eq!(in_d.len(), 1);
    }

    /// Toglie all'utente corrente i permessi negati con icacls, anche se il test fallisce.
    struct AclGuard(PathBuf, String);
    impl Drop for AclGuard {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls").arg(&self.0).arg("/remove:d").arg(&self.1).output();
        }
    }

    #[test]
    fn traverse_fallback_and_access_denied() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("a").join("b"));
        fs::create_dir_all(&b).unwrap();
        let user = format!(r"{}\{}", std::env::var("USERDOMAIN").unwrap(), std::env::var("USERNAME").unwrap());
        let _guard = AclGuard(a.clone(), user.clone());
        let deny = |rights: &str| {
            let ok = std::process::Command::new("icacls").arg(&a).arg("/deny").arg(format!("{user}:({rights})")).output().unwrap().status.success();
            assert!(ok, "icacls /deny {rights}");
        };
        deny("RD");
        assert!(lock_ancestors(&b.join("x")).is_ok());
        deny("X");
        let e = lock_ancestors(&b.join("x")).err().unwrap();
        assert_eq!(e.code, ErrorCode::AccessDenied);
        assert_eq!(e.path.as_deref().map(Path::new), Some(a.as_path()));
    }

    #[test]
    fn missing_or_file_component() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("file.txt"), b"x").unwrap();
        for p in [r"file.txt\x", r"nope\x"] {
            assert_eq!(lock_ancestors(&d.path().join(p)).err().unwrap().code, ErrorCode::NotFound, "{p}");
        }
    }
}
