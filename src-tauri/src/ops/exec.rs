//! Esecutore locale: le primitive che toccano davvero il disco.

use super::conflict::same_file;
use crate::error::{AppError, ErrorCode};
use crate::paths::{to_extended, volume_key};
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, COPYPROGRESSROUTINE_PROGRESS, CopyFileExW, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_READONLY, FILE_FLAGS_AND_ATTRIBUTES, FlushFileBuffers, GetDriveTypeW, GetVolumePathNameW,
    LPPROGRESS_ROUTINE_CALLBACK_REASON,
    MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MoveFileExW, PROGRESS_CANCEL, PROGRESS_CONTINUE,
    SetFileAttributesW,
};
use windows::core::HSTRING;

/// Suffisso del file temporaneo in cui si scrive una copia prima di darle il nome finale.
pub const PART_SUFFIX: &str = ".mcpart";

pub trait Executor: Send + Sync {
    /// `progress(byte_copiati)`; se restituisce `false` la copia si annulla.
    /// Con `overwrite` (come in `move_path`) si sostituisce anche una destinazione di sola lettura.
    /// Con `flush` (o quando si sostituisce) la copia è scritta su disco prima di prendere il nome
    /// finale: serve al Move tra volumi, che subito dopo elimina il sorgente.
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, flush: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError>;
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

/// Prima di sostituire `dst`: MoveFileExW rifiuta di sostituire un file di sola lettura. Gli
/// attributi del file nuovo vengono comunque dalla sorgente. Su una cartella la sola lettura è
/// un marcatore della Shell e la sostituzione fallirebbe comunque: non si tocca.
fn clear_readonly_of(dst: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(to_extended(dst)) {
        Ok(m) if !m.is_dir() => clear_readonly(dst, m.file_attributes()),
        _ => Ok(()),
    }
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

/// Limite NTFS di un singolo componente, in unità UTF-16.
const MAX_COMPONENT: usize = 255;

/// FNV-1a a 64 bit: stabile tra versioni (a differenza di `DefaultHasher`), così un `.mcpart`
/// lasciato da una versione precedente si ritrova.
fn fnv1a(units: impl Iterator<Item = u16>) -> u64 {
    units.fold(0xcbf2_9ce4_8422_2325, |h, u| (h ^ u64::from(u)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// File temporaneo della copia, accanto a `dst`: `nome.mcpart`. Se il nome supererebbe i 255
/// caratteri si usa `prefisso~hash.mcpart`, deterministico rispetto al nome completo.
fn part_path(dst: &Path) -> PathBuf {
    let name = dst.file_name().unwrap_or_default();
    let name_len = name.encode_wide().count();
    if name_len + PART_SUFFIX.len() <= MAX_COMPONENT {
        let mut s = name.to_os_string();
        s.push(PART_SUFFIX);
        return dst.with_file_name(s);
    }
    let tail = format!("~{:016x}{PART_SUFFIX}", fnv1a(name.encode_wide()));
    let budget = MAX_COMPONENT - tail.len();
    let (mut prefix, mut used) = (String::new(), 0);
    for c in name.to_string_lossy().chars() {
        used += c.len_utf16(); // mai a metà di una coppia surrogata
        if used > budget {
            break;
        }
        prefix.push(c);
    }
    dst.with_file_name(prefix + &tail)
}

/// Il Cestino esiste in modo affidabile solo sui dischi fissi: su rete, rimovibili, CD e RAM disk
/// la Shell (con `FOF_NO_UI`) cancellerebbe in modo definitivo senza dirlo.
fn recycle_bin_supported(drive_type: u32) -> bool {
    drive_type == 3 // DRIVE_FIXED
}

/// Componente che termina con `.` o spazio: la Shell lo normalizza e cestinerebbe un altro file.
fn has_trailing_dot_or_space(p: &Path) -> bool {
    p.components().any(|c| match c {
        Component::Normal(n) => n.to_string_lossy().ends_with(['.', ' ']),
        _ => false,
    })
}

/// Radice del volume che contiene davvero `p`, con il `\` finale (`C:\` oppure `C:\mnt\usb\` per un
/// volume montato in una cartella), senza prefisso esteso. `None` se il sistema non la risolve.
fn volume_root(p: &Path) -> Option<String> {
    let mut buf = vec![0u16; 32768];
    unsafe { GetVolumePathNameW(&wide(p), &mut buf) }.ok()?;
    let len = buf.iter().position(|&c| c == 0)?;
    let root = String::from_utf16_lossy(&buf[..len]);
    Some(root.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(root))
}

fn move_to_recycle_bin(path: &Path) -> Result<(), AppError> {
    if has_trailing_dot_or_space(path) {
        return Err(AppError {
            detail: Some("recycle_bin_unsupported_name".into()),
            ..AppError::new(ErrorCode::InvalidName, Some(path))
        });
    }
    // I percorsi UNC (`\\server\share`) non hanno Cestino: si rifiutano senza toccare la rete.
    let root = if volume_key(path).len() == 2 { volume_root(path) } else { None };
    let drive_type = root.map_or(0, |r| unsafe { GetDriveTypeW(&HSTRING::from(r)) });
    if !recycle_bin_supported(drive_type) {
        return Err(AppError::new(ErrorCode::NoRecycleBin, Some(path)));
    }
    trash::delete(path).map_err(|e| match e {
        trash::Error::Os { code, description } => from_hresult(code, description, path),
        other => AppError { detail: Some(other.to_string()), ..AppError::new(ErrorCode::Io, Some(path)) },
    })
}

#[cfg(test)]
thread_local! {
    /// Numero di flush eseguiti sul thread corrente (il callback gira sul thread di CopyFileExW).
    static FLUSHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn flush(h: HANDLE) -> windows::core::Result<()> {
    #[cfg(test)]
    FLUSHES.with(|c| c.set(c.get() + 1));
    unsafe { FlushFileBuffers(h) }
}

/// Stato condiviso col callback di CopyFileExW.
struct CopyCtx<'a> {
    progress: Progress<'a>,
    /// Scrivere su disco la copia completa prima che sostituisca un `dst` esistente.
    flush: bool,
    flush_err: Option<windows::core::Error>,
}

unsafe extern "system" fn on_progress(
    _total: i64,
    transferred: i64,
    stream_size: i64,
    stream_transferred: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _src: HANDLE,
    dst: HANDLE,
    data: *const c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    // SAFETY: `data` punta al `CopyCtx` che `copy_file` tiene vivo per tutta la CopyFileExW.
    let ctx = unsafe { &mut *(data as *mut CopyCtx) };
    // Un panico non deve attraversare il confine FFI: lo si tratta come annullamento.
    if !matches!(catch_unwind(AssertUnwindSafe(|| (ctx.progress)(transferred as u64))), Ok(true)) {
        return PROGRESS_CANCEL;
    }
    // Fine di uno stream (dati principali o ADS come Zone.Identifier; uno stream vuoto è già
    // completo al cambio di stream): `dst` è l'handle di scrittura dello stream corrente e
    // FlushFileBuffers scrive solo quello, quindi va fatto per ogni stream. L'handle funziona
    // anche se il `.mcpart` ha ereditato la sola lettura.
    if ctx.flush
        && stream_transferred == stream_size
        && let Err(e) = flush(dst)
    {
        ctx.flush_err = Some(e);
        return PROGRESS_CANCEL;
    }
    PROGRESS_CONTINUE
}

impl Executor for LocalExecutor {
    /// La copia si scrive in `dst.mcpart` e prende il nome finale solo quando è completa:
    /// una copia interrotta non lascia mai un `dst` troncato, e `dst` non viene mai riscritto sul posto.
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, flush: bool, progress: Progress) -> Result<(), AppError> {
        let dst_exists = exists(dst);
        if dst_exists && !overwrite {
            return Err(AppError::new(ErrorCode::AlreadyExists, Some(dst)));
        }
        if dst_exists && same_file(src, dst) {
            return Err(AppError::new(ErrorCode::SameFile, Some(src)));
        }
        let part = part_path(dst);
        remove_part(&part); // residuo di un crash precedente: è nostro per nome
        // Il flush serve quando si sostituisce (si perderebbe la versione precedente) o quando il
        // chiamante sta per eliminare il sorgente; altrimenti un'interruzione di corrente non può
        // far perdere dati.
        let mut ctx = CopyCtx { progress, flush: flush || dst_exists, flush_err: None };
        let data = &mut ctx as *mut CopyCtx as *const c_void;
        let copied = unsafe {
            CopyFileExW(&wide(src), &wide(&part), Some(on_progress), Some(data), None, COPY_FILE_FAIL_IF_EXISTS)
        };
        if let Err(e) = copied {
            remove_part(&part);
            if let Some(fe) = ctx.flush_err {
                return Err(from_win(&fe, dst));
            }
            // Se la sorgente si apre il problema è dal lato della destinazione.
            let blame = if std::fs::File::open(to_extended(src)).is_ok() { dst } else { src };
            return Err(from_win(&e, blame));
        }
        // Difesa in profondità: `dst` potrebbe essere cambiato durante la copia.
        if overwrite && same_file(src, dst) {
            remove_part(&part);
            return Err(AppError::new(ErrorCode::SameFile, Some(src)));
        }
        if overwrite && let Err(e) = clear_readonly_of(dst) {
            remove_part(&part);
            return Err(e);
        }
        let flags = if overwrite { MOVEFILE_REPLACE_EXISTING } else { MOVE_FILE_FLAGS(0) };
        move_file(&part, dst, flags).map_err(|e| {
            remove_part(&part);
            AppError { path: Some(dst.to_string_lossy().into_owned()), ..e }
        })
    }

    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
        if overwrite {
            if same_file(src, dst) {
                return Err(AppError::new(ErrorCode::SameFile, Some(src)));
            }
            clear_readonly_of(dst)?;
        }
        // Senza MOVEFILE_COPY_ALLOWED: tra unità diverse fallisce con NOT_SAME_DEVICE.
        move_file(src, dst, if overwrite { MOVEFILE_REPLACE_EXISTING } else { MOVE_FILE_FLAGS(0) })
    }

    /// Le cartelle vanno cancellate già vuote: `expand` mette i figli prima. Un collegamento a
    /// cartella (giunzione/symlink) si cancella come cartella e sparisce solo il collegamento.
    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
        let meta = std::fs::symlink_metadata(to_extended(path)).map_err(|e| AppError::from_io(&e, path))?;
        if !permanent {
            return move_to_recycle_bin(path);
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

    const X: LocalExecutor = LocalExecutor;

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
        X.copy_file(&src, &dst, false, false, &mut |n| {
            seen.push(n);
            true
        })
        .unwrap();
        assert!(seen.len() > 1, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert!(seen.iter().any(|&n| n > 0 && n < data.len() as u64), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), data.len() as u64);
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert!(!exists(&part_path(&dst)));
    }

    #[test]
    fn cancelled_copy_leaves_nothing_at_destination() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a.bin"), d.path().join("b.bin"));
        fs::write(&src, vec![7u8; 1024 * 1024]).unwrap();
        let err = X.copy_file(&src, &dst, false, false, &mut |_| false).unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(!exists(&dst) && !exists(&part_path(&dst)));
        // Un panico nel callback non attraversa l'FFI: diventa un annullamento.
        let err = X.copy_file(&src, &dst, false, false, &mut |_| panic!("prova")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(!exists(&dst) && !exists(&part_path(&dst)));
    }

    #[test]
    fn overwrite_and_move_copy_flush_every_stream() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("scaricato.bin"), d.path().join("vecchio.bin"));
        fs::write(&src, vec![1u8; 64 * 1024]).unwrap();
        let mut ads = to_extended(&src).into_os_string();
        ads.push(":Zone.Identifier");
        fs::write(&ads, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        // Senza dst da sostituire e senza `flush` non si fa alcun flush.
        FLUSHES.with(|c| c.set(0));
        X.copy_file(&src, &dst, false, false, &mut |_| true).unwrap();
        assert_eq!(FLUSHES.with(|c| c.get()), 0);
        // Sostituendo, un flush per stream: dati principali + Zone.Identifier.
        X.copy_file(&src, &dst, true, false, &mut |_| true).unwrap();
        assert_eq!(FLUSHES.with(|c| c.get()), 2);
        // Copia per uno spostamento (dst nuovo, `flush`): ancora un flush per stream.
        let moved = d.path().join("spostato.bin");
        X.copy_file(&src, &moved, false, true, &mut |_| true).unwrap();
        assert_eq!(FLUSHES.with(|c| c.get()), 4);
        assert_eq!(fs::read(&moved).unwrap(), vec![1u8; 64 * 1024]);
        assert_eq!(fs::read(&dst).unwrap(), vec![1u8; 64 * 1024]);
        let mut dst_ads = to_extended(&dst).into_os_string();
        dst_ads.push(":Zone.Identifier");
        assert_eq!(fs::read(&dst_ads).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[test]
    fn copy_without_overwrite_fails_on_existing() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a"), d.path().join("b"));
        fs::write(&src, b"nuovo").unwrap();
        fs::write(&dst, b"vecchio").unwrap();
        let err = X.copy_file(&src, &dst, false, false, &mut |_| true).unwrap_err();
        assert_eq!(err.code, ErrorCode::AlreadyExists);
        assert_eq!(fs::read(&dst).unwrap(), b"vecchio");
        assert!(!exists(&part_path(&dst)));
    }

    #[test]
    fn overwrite_replaces_atomically() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a"), d.path().join("b"));
        fs::write(&src, b"nuovi byte").unwrap();
        fs::write(&dst, b"old").unwrap();
        // Un `.mcpart` rimasto da un crash precedente è nostro: si sostituisce.
        fs::write(part_path(&dst), b"residuo").unwrap();
        X.copy_file(&src, &dst, true, false, &mut |_| true).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"nuovi byte");
        assert!(!exists(&part_path(&dst)));
        // Sovrascrivere un file con se stesso (qui via hard link) è vietato.
        let link = d.path().join("link");
        fs::hard_link(&src, &link).unwrap();
        assert_eq!(X.copy_file(&src, &link, true, false, &mut |_| true).unwrap_err().code, ErrorCode::SameFile);
        assert_eq!(fs::read(&link).unwrap(), b"nuovi byte");
        assert!(!exists(&part_path(&link)));
    }

    #[test]
    fn overwrite_replaces_readonly_destination() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("a"), d.path().join("ro"));
        fs::write(&src, b"nuovo").unwrap();
        let set_ro = |p: &Path| {
            fs::write(p, b"vecchio").unwrap();
            let mut perm = fs::metadata(p).unwrap().permissions();
            perm.set_readonly(true);
            fs::set_permissions(p, perm).unwrap();
        };
        set_ro(&dst);
        X.copy_file(&src, &dst, true, false, &mut |_| true).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"nuovo");
        assert!(!fs::metadata(&dst).unwrap().permissions().readonly());
        fs::remove_file(&dst).unwrap();
        set_ro(&dst);
        X.move_path(&src, &dst, true).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"nuovo");
        assert!(!exists(&src));
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
        X.copy_file(&src, &dst, false, false, &mut |_| true).unwrap();
        assert_eq!(fs::read(to_extended(&dst)).unwrap(), b"x");
        X.delete(&dst, false, true).unwrap();
        assert!(!exists(&dst));
    }

    // Manda un file vero nel Cestino dell'utente: si esegue a mano con
    // `cargo test --manifest-path src-tauri/Cargo.toml delete_to_recycle_bin -- --ignored`.
    #[test]
    #[ignore]
    fn delete_to_recycle_bin_removes_from_disk() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cestino.txt");
        fs::write(&f, b"x").unwrap();
        X.delete(&f, false, false).unwrap();
        assert!(!exists(&f));
        assert_eq!(X.delete(&f, false, false).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn recycle_bin_refuses_names_the_shell_would_alter() {
        let d = tempfile::tempdir().unwrap();
        for (odd, twin) in [("x.", "x"), ("y ", "y")] {
            let (odd, twin) = (d.path().join(odd), d.path().join(twin));
            fs::write(to_extended(&odd), b"strano").unwrap();
            fs::write(&twin, b"gemello").unwrap();
            let err = X.delete(&odd, false, false).unwrap_err();
            assert_eq!((err.code, err.detail.as_deref()), (ErrorCode::InvalidName, Some("recycle_bin_unsupported_name")));
            assert!(exists(&odd) && exists(&twin));
        }
    }

    #[test]
    fn recycle_bin_only_on_fixed_drives() {
        // GetDriveTypeW: 0 sconosciuto, 1 nessuna radice, 2 rimovibile, 3 fisso, 4 rete, 5 CD, 6 RAM disk.
        let allowed: Vec<u32> = (0..=6).filter(|&t| recycle_bin_supported(t)).collect();
        assert_eq!(allowed, vec![3]);
        let unc = Path::new(r"\\server\share\f.txt");
        assert_eq!(move_to_recycle_bin(unc).unwrap_err().code, ErrorCode::NoRecycleBin);
        // La radice si risolve dal percorso (anche esteso e lungo), con il separatore finale.
        let d = tempfile::tempdir().unwrap();
        let mut deep = d.path().to_path_buf();
        for _ in 0..30 {
            deep.push("cartella_lunga");
        }
        assert_eq!(volume_root(&deep), Some(volume_key(d.path()) + "\\"));
    }

    #[test]
    fn part_name_fits_ntfs_component_limit() {
        let d = tempfile::tempdir().unwrap();
        let name = "n".repeat(252);
        let (src, dst) = (d.path().join("s"), d.path().join(&name));
        let part = part_path(&dst);
        let part_name = part.file_name().unwrap().to_string_lossy().into_owned();
        assert!(part_name.encode_utf16().count() <= 255 && part_name.ends_with(PART_SUFFIX), "{part_name}");
        assert_eq!(part.parent(), dst.parent());
        assert_eq!(part_path(&dst), part); // deterministico: il residuo si ritrova
        assert_ne!(part_path(&d.path().join("n".repeat(251) + "m")), part);
        fs::write(&src, b"x").unwrap();
        fs::write(&part, b"residuo").unwrap();
        X.copy_file(&src, &dst, false, false, &mut |_| true).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"x");
        assert!(!exists(&part));
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
