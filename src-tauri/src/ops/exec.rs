//! Esecutore locale: le primitive che toccano davvero il disco.

use super::conflict::same_file;
use crate::error::{AppError, ErrorCode};
use crate::paths::to_extended;
use std::ffi::c_void;
use std::os::windows::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, COPYPROGRESSROUTINE_PROGRESS, CopyFileExW, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_READONLY, FILE_FLAGS_AND_ATTRIBUTES, LPPROGRESS_ROUTINE_CALLBACK_REASON,
    MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MoveFileExW, PROGRESS_CANCEL, PROGRESS_CONTINUE,
    SetFileAttributesW,
};
use windows::core::HSTRING;

/// Suffisso del file temporaneo in cui si scrive una copia prima di darle il nome finale.
pub const PART_SUFFIX: &str = ".mcpart";

pub trait Executor: Send + Sync {
    /// `progress(byte_copiati)`; se restituisce `false` la copia si annulla.
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError>;
    /// Solo sulla stessa unità; tra unità diverse dà `NOT_SAME_DEVICE`.
    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError>;
    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError>;
    fn mkdir(&self, path: &Path) -> Result<(), AppError>;
    fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError>;
}

pub struct LocalExecutor;

type Progress<'a> = &'a mut dyn FnMut(u64) -> bool;

fn wide(p: &Path) -> HSTRING {
    HSTRING::from(to_extended(p).as_path())
}

/// Errore Win32 (HRESULT `0x8007xxxx`) → `AppError` con la stessa mappatura di `from_io`.
fn from_hresult(hr: i32, detail: String, path: &Path) -> AppError {
    let code = if (hr as u32) & 0xFFFF_0000 == 0x8007_0000 { hr & 0xFFFF } else { hr };
    AppError { detail: Some(detail), ..AppError::from_io(&std::io::Error::from_raw_os_error(code), path) }
}

fn from_win(e: &windows::core::Error, path: &Path) -> AppError {
    from_hresult(e.code().0, e.message(), path)
}

fn exists(p: &Path) -> bool {
    // `symlink_metadata`: anche un collegamento rotto occupa il nome.
    std::fs::symlink_metadata(to_extended(p)).is_ok()
}

/// Toglie l'attributo di sola lettura dall'elemento stesso (mai dal bersaglio di un collegamento).
fn clear_readonly(p: &Path, attrs: u32) -> Result<(), AppError> {
    if attrs & FILE_ATTRIBUTE_READONLY.0 == 0 {
        return Ok(());
    }
    // Solo gli attributi impostabili con SetFileAttributesW (ARCHIVE, HIDDEN, SYSTEM, TEMPORARY,
    // OFFLINE, NOT_CONTENT_INDEXED), meno READONLY; nessuno → NORMAL.
    let keep = attrs & 0x3126;
    let new = if keep == 0 { FILE_ATTRIBUTE_NORMAL } else { FILE_FLAGS_AND_ATTRIBUTES(keep) };
    unsafe { SetFileAttributesW(&wide(p), new) }.map_err(|e| from_win(&e, p))
}

/// Cancella il `.mcpart`, anche se ha ereditato la sola lettura dalla sorgente; assente = ok.
fn remove_part(part: &Path) {
    if let Ok(m) = std::fs::symlink_metadata(to_extended(part)) {
        let _ = clear_readonly(part, m.file_attributes());
        let _ = std::fs::remove_file(to_extended(part));
    }
}

fn move_file(src: &Path, dst: &Path, flags: MOVE_FILE_FLAGS) -> Result<(), AppError> {
    unsafe { MoveFileExW(&wide(src), &wide(dst), flags) }.map_err(|e| {
        let err = from_win(&e, src);
        match err.code {
            ErrorCode::AlreadyExists | ErrorCode::InvalidName => AppError { path: Some(dst.to_string_lossy().into_owned()), ..err },
            _ => err,
        }
    })
}

unsafe extern "system" fn on_progress(
    _total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    // SAFETY: `data` punta al `Progress` che `copy_file` tiene vivo per tutta la CopyFileExW.
    let progress = unsafe { &mut *(data as *mut Progress) };
    // Un panico non deve attraversare il confine FFI: lo si tratta come annullamento.
    match catch_unwind(AssertUnwindSafe(|| progress(transferred as u64))) {
        Ok(true) => PROGRESS_CONTINUE,
        _ => PROGRESS_CANCEL,
    }
}

impl Executor for LocalExecutor {
    /// La copia si scrive in `dst.mcpart` e prende il nome finale solo quando è completa:
    /// una copia interrotta non lascia mai un `dst` troncato, e `dst` non viene mai riscritto sul posto.
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, mut progress: Progress) -> Result<(), AppError> {
        if !overwrite && exists(dst) {
            return Err(AppError::new(ErrorCode::AlreadyExists, Some(dst)));
        }
        let mut part = dst.as_os_str().to_os_string();
        part.push(PART_SUFFIX);
        let part = PathBuf::from(part);
        remove_part(&part); // residuo di un crash precedente: è nostro per nome
        let data = &mut progress as *mut Progress as *const c_void;
        let copied = unsafe {
            CopyFileExW(&wide(src), &wide(&part), Some(on_progress), Some(data), None, COPY_FILE_FAIL_IF_EXISTS)
        };
        if let Err(e) = copied {
            remove_part(&part);
            // Se la sorgente si apre il problema è dal lato della destinazione.
            let blame = if std::fs::File::open(to_extended(src)).is_ok() { dst } else { src };
            return Err(from_win(&e, blame));
        }
        if overwrite && same_file(src, dst) {
            remove_part(&part);
            return Err(AppError::new(ErrorCode::SameFile, Some(src)));
        }
        let flags = if overwrite { MOVEFILE_REPLACE_EXISTING } else { MOVE_FILE_FLAGS(0) };
        move_file(&part, dst, flags).map_err(|e| {
            remove_part(&part);
            AppError { path: Some(dst.to_string_lossy().into_owned()), ..e }
        })
    }

    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
        if overwrite && same_file(src, dst) {
            return Err(AppError::new(ErrorCode::SameFile, Some(src)));
        }
        // Senza MOVEFILE_COPY_ALLOWED: tra unità diverse fallisce con NOT_SAME_DEVICE.
        move_file(src, dst, if overwrite { MOVEFILE_REPLACE_EXISTING } else { MOVE_FILE_FLAGS(0) })
    }

    /// Le cartelle vanno cancellate già vuote: `expand` mette i figli prima. Un collegamento a
    /// cartella (giunzione/symlink) si cancella come cartella e sparisce solo il collegamento.
    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
        let meta = std::fs::symlink_metadata(to_extended(path)).map_err(|e| AppError::from_io(&e, path))?;
        if !permanent {
            return trash::delete(path).map_err(|e| match e {
                trash::Error::Os { code, description } => from_hresult(code, description, path),
                other => AppError { detail: Some(other.to_string()), ..AppError::new(ErrorCode::Io, Some(path)) },
            });
        }
        clear_readonly(path, meta.file_attributes())?;
        let ext = to_extended(path);
        let res = if is_dir { std::fs::remove_dir(ext) } else { std::fs::remove_file(ext) };
        res.map_err(|e| AppError::from_io(&e, path))
    }

    fn mkdir(&self, path: &Path) -> Result<(), AppError> {
        std::fs::create_dir(to_extended(path)).map_err(|e| AppError::from_io(&e, path))
    }

    /// Senza sostituzione; le rinomine che cambiano solo maiuscole/minuscole funzionano.
    fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
        move_file(src, dst, MOVE_FILE_FLAGS(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{to_extended, volume_key};
    use std::fs;
    use std::path::PathBuf;

    const X: LocalExecutor = LocalExecutor;

    fn part(dst: &Path) -> PathBuf {
        let mut s = dst.as_os_str().to_os_string();
        s.push(PART_SUFFIX);
        PathBuf::from(s)
    }

    fn exists(p: &Path) -> bool {
        fs::symlink_metadata(to_extended(p)).is_ok()
    }

    #[test]
    fn copy_writes_full_file_and_reports_progress() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("big.bin"), d.path().join("copia.bin"));
        let data: Vec<u8> = (0..3 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let mut seen = Vec::new();
        X.copy_file(&src, &dst, false, &mut |n| {
            seen.push(n);
            true
        })
        .unwrap();
        assert!(seen.len() > 1, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert!(seen.iter().any(|&n| n > 0 && n < data.len() as u64), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), data.len() as u64);
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert!(!exists(&part(&dst)));
    }

    #[test]
    fn cancelled_copy_leaves_nothing_at_destination() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a.bin"), d.path().join("b.bin"));
        fs::write(&src, vec![7u8; 1024 * 1024]).unwrap();
        let err = X.copy_file(&src, &dst, false, &mut |_| false).unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(!exists(&dst) && !exists(&part(&dst)));
        // Un panico nel callback non attraversa l'FFI: diventa un annullamento.
        let err = X.copy_file(&src, &dst, false, &mut |_| panic!("prova")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(!exists(&dst) && !exists(&part(&dst)));
    }

    #[test]
    fn copy_without_overwrite_fails_on_existing() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a"), d.path().join("b"));
        fs::write(&src, b"nuovo").unwrap();
        fs::write(&dst, b"vecchio").unwrap();
        let err = X.copy_file(&src, &dst, false, &mut |_| true).unwrap_err();
        assert_eq!(err.code, ErrorCode::AlreadyExists);
        assert_eq!(fs::read(&dst).unwrap(), b"vecchio");
        assert!(!exists(&part(&dst)));
    }

    #[test]
    fn overwrite_replaces_atomically() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a"), d.path().join("b"));
        fs::write(&src, b"nuovi byte").unwrap();
        fs::write(&dst, b"old").unwrap();
        // Un `.mcpart` rimasto da un crash precedente è nostro: si sostituisce.
        fs::write(part(&dst), b"residuo").unwrap();
        X.copy_file(&src, &dst, true, &mut |_| true).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"nuovi byte");
        assert!(!exists(&part(&dst)));
        // Sovrascrivere un file con se stesso (qui via hard link) è vietato.
        let link = d.path().join("link");
        fs::hard_link(&src, &link).unwrap();
        assert_eq!(X.copy_file(&src, &link, true, &mut |_| true).unwrap_err().code, ErrorCode::SameFile);
        assert_eq!(fs::read(&link).unwrap(), b"nuovi byte");
        assert!(!exists(&part(&link)));
    }

    #[test]
    fn long_path_and_trailing_dot_copy_and_delete() {
        let d = tempfile::tempdir().unwrap();
        let mut dir = d.path().to_path_buf();
        for _ in 0..30 {
            dir.push("cartella_lunga"); // > 260 caratteri
        }
        fs::create_dir_all(to_extended(&dir)).unwrap();
        let src = d.path().join("s.txt");
        fs::write(&src, b"x").unwrap();
        let dst = dir.join("x.");
        X.copy_file(&src, &dst, false, &mut |_| true).unwrap();
        assert_eq!(fs::read(to_extended(&dst)).unwrap(), b"x");
        X.delete(&dst, false, true).unwrap();
        assert!(!exists(&dst));
    }

    #[test]
    fn delete_to_recycle_bin_removes_from_disk() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cestino.txt");
        fs::write(&f, b"x").unwrap();
        X.delete(&f, false, false).unwrap();
        assert!(!exists(&f));
        assert_eq!(X.delete(&f, false, false).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn readonly_file_permanent_delete() {
        let d = tempfile::tempdir().unwrap();
        let (f, sub) = (d.path().join("ro.txt"), d.path().join("ro_dir"));
        fs::write(&f, b"x").unwrap();
        fs::create_dir(&sub).unwrap();
        for p in [&f, &sub] {
            let mut perm = fs::metadata(p).unwrap().permissions();
            perm.set_readonly(true);
            fs::set_permissions(p, perm).unwrap();
        }
        X.delete(&f, false, true).unwrap();
        X.delete(&sub, true, true).unwrap();
        assert!(!exists(&f) && !exists(&sub));
        assert_eq!(X.delete(&f, false, true).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn permanent_delete_of_junction_keeps_target() {
        let d = tempfile::tempdir().unwrap();
        let (target, link) = (d.path().join("bersaglio"), d.path().join("giunzione"));
        fs::create_dir(&target).unwrap();
        fs::write(target.join("dentro.txt"), b"x").unwrap();
        let ok = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        X.delete(&link, true, true).unwrap();
        assert!(!exists(&link));
        assert_eq!(fs::read(target.join("dentro.txt")).unwrap(), b"x");
    }

    #[test]
    fn mkdir_and_rename() {
        let d = tempfile::tempdir().unwrap();
        let n = d.path().join("nuova");
        X.mkdir(&n).unwrap();
        assert!(n.is_dir());
        assert_eq!(X.mkdir(&n).unwrap_err().code, ErrorCode::AlreadyExists);

        let (a, b) = (d.path().join("a.txt"), d.path().join("b.txt"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        assert_eq!(X.rename(&a, &b).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(fs::read(&b).unwrap(), b"b");
        // Rinomina che cambia solo maiuscole/minuscole.
        let upper = d.path().join("A.txt");
        X.rename(&a, &upper).unwrap();
        let names: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().any(|n| n == "A.txt") && !names.iter().any(|n| n == "a.txt"), "{names:?}");
    }

    #[test]
    fn move_same_volume_and_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::write(&a, b"nuovo").unwrap();
        fs::write(&b, b"vecchio").unwrap();
        assert_eq!(X.move_path(&a, &b, false).unwrap_err().code, ErrorCode::AlreadyExists);
        X.move_path(&a, &b, true).unwrap();
        assert!(!exists(&a));
        assert_eq!(fs::read(&b).unwrap(), b"nuovo");
    }

    #[test]
    fn move_across_volumes_is_reported() {
        // Serve un secondo volume: la cartella del progetto se non è sull'unità dei temporanei.
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        if volume_key(src_dir.path()) == volume_key(dst_dir.path()) {
            return;
        }
        let a = src_dir.path().join("a");
        fs::write(&a, b"x").unwrap();
        let err = X.move_path(&a, &dst_dir.path().join("a"), false).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSameDevice);
        assert!(exists(&a));
    }
}
