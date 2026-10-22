//! Copia di un file tra provider diversi (download, upload, da remoto a remoto): si scrive un temporaneo
//! `<nome>.<8 cifre esadecimali>.mcpart` accanto alla destinazione e lo si rinomina sul nome finale. Se il server
//! non lo permette, si scrive direttamente sul file, ma solo con la conferma dell'utente.

use super::loc::Loc;
use super::provider::{Purpose, stream_error};
use super::router::Vfs;
use crate::error::{AppError, ErrorCode};
use crate::ops::exec::PART_SUFFIX;
use crate::paths::invalid_windows_name;
use std::io::{Read, Write};
use std::path::Path;

const BLOCK: usize = 256 * 1024;

/// Copia `src` su `dst`; senza `replace` una destinazione esistente dà `ALREADY_EXISTS`. `progress(byte)` dopo
/// ogni blocco: con `false` si annulla (`CANCELLED`). Se la scrittura sicura sul remoto non si può fare (il
/// temporaneo è rifiutato con `ACCESS_DENIED`, oppure `dst` esiste e il server non rinomina sopra un file), si
/// scrive direttamente su `dst` solo se `confirm_unsafe(dst)` lo permette; altrimenti `CANCELLED` e `dst` resta
/// intatto. Si decide (e si chiede) prima di aprire la sorgente: un canale dati FTP non resta fermo intanto.
pub fn copy(
    vfs: &Vfs,
    src: &Loc,
    dst: &Loc,
    replace: bool,
    confirm_unsafe: &mut dyn FnMut(&Loc) -> bool,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), AppError> {
    // Un nome remoto che Windows non accetta non si rinomina in silenzio: errore per quell'elemento.
    if let Loc::Local(p) = dst
        && p.file_name().is_some_and(|n| invalid_windows_name(&n.to_string_lossy()))
    {
        return Err(dst.error(ErrorCode::InvalidName));
    }
    let (dp, dpath) = vfs.endpoint(dst, Purpose::Transfer)?;
    let existing = dp.stat(&dpath).ok();
    // I permessi POSIX del file che si sostituisce; in locale non ci sono.
    let mode = existing.as_ref().filter(|_| dst.is_remote()).and_then(|s| s.mode);
    let tmp = format!("{dpath}.{}{PART_SUFFIX}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let safe = if existing.is_some() && replace && !dp.caps().atomic_replace {
        None
    } else {
        match dp.writer(&tmp, true) {
            Ok(w) => Some(w),
            // Sul remoto la cartella può non essere scrivibile e il file sì; in locale resta un errore.
            Err(e) if e.code == ErrorCode::AccessDenied && dst.is_remote() => None,
            Err(e) => return Err(e),
        }
    };
    let used_tmp = safe.is_some();
    if !used_tmp && !confirm_unsafe(dst) {
        return Err(dst.error(ErrorCode::Cancelled));
    }
    // Il writer diretto si apre solo dopo la sorgente: aprirlo tronca `dst`, che così resta intatto se la
    // sorgente non si apre.
    let mut direct_created = false;
    let r = (|| {
        let (sp, spath) = vfs.endpoint(src, Purpose::Transfer)?;
        // La data prima di aprire la sorgente: in FTP uno stream aperto tiene occupata la connessione.
        let modified = sp.stat(&spath)?.modified_ms;
        let mut reader = sp.reader(&spath)?;
        let (target, w) = match safe {
            Some(w) => (tmp.as_str(), w),
            None => (dpath.as_str(), dp.writer(&dpath, !replace)?),
        };
        direct_created = !used_tmp && existing.is_none();
        // ponytail: nella scrittura diretta un annullamento o una connessione caduta lasciano incompleto un file
        // che c'era già; è il rischio che l'utente ha accettato.
        pump(&mut reader, w, &spath, target, progress)?;
        let _ = dp.set_meta(target, modified, mode);
        if used_tmp { dp.rename(&tmp, &dpath, replace) } else { Ok(()) }
    })();
    // Il temporaneo si elimina; nella scrittura diretta anche il file, se l'abbiamo creato noi.
    if r.is_err() && (used_tmp || direct_created) {
        let _ = dp.delete(if used_tmp { &tmp } else { &dpath }, false);
    }
    r
}

/// Copia a blocchi; il writer si chiude prima di tornare (anche dopo un errore).
fn pump(
    r: &mut dyn Read,
    mut w: Box<dyn Write + Send>,
    spath: &str,
    dpath: &str,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), AppError> {
    let mut buf = vec![0u8; BLOCK];
    let mut done = 0u64;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(stream_error(e, spath)),
        };
        w.write_all(&buf[..n]).map_err(|e| stream_error(e, dpath))?;
        done += n as u64;
        if !progress(done) {
            return Err(AppError::new(ErrorCode::Cancelled, Some(Path::new(dpath))));
        }
    }
    w.flush().map_err(|e| stream_error(e, dpath))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::vfs::local::LocalProvider;
    use crate::vfs::mem::{MemProvider, vfs};
    use crate::vfs::provider::Provider;
    use std::collections::HashSet;
    use std::sync::Arc;

    fn loc(s: &str) -> Loc {
        Loc::parse(s).unwrap()
    }

    fn names(m: &MemProvider, dir: &str) -> Vec<String> {
        m.list(dir).unwrap().into_iter().map(|s| s.name).collect()
    }

    /// File da 1 MiB: quattro blocchi.
    fn big() -> Vec<u8> {
        (0..1u32 << 20).map(|i| i as u8).collect()
    }

    #[test]
    fn download_and_upload_roundtrip() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/a.txt", b"contenuto");
        m.set_meta("/a.txt", Some(1_600_000_000_123), None).unwrap();
        let v = vfs(&[("m", &m)]);
        let d = tempfile::tempdir().unwrap();
        let local = d.path().join("a.txt");
        copy(&v, &loc("sftp://m/a.txt"), &Loc::Local(local.clone()), false, &mut |_| unreachable!(), &mut |_| true).unwrap();
        assert_eq!(std::fs::read(&local).unwrap(), b"contenuto");
        assert_eq!(LocalProvider.stat(&local.to_string_lossy()).unwrap().modified_ms, Some(1_600_000_000_123));
        copy(&v, &Loc::Local(local), &loc("sftp://m/b.txt"), false, &mut |_| unreachable!(), &mut |_| true).unwrap();
        assert_eq!(m.data("/b.txt").unwrap(), b"contenuto");
        assert_eq!(m.stat("/b.txt").unwrap().modified_ms, Some(1_600_000_000_123));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1, "nessun .mcpart");
        assert_eq!(names(&m, "/"), ["a.txt", "b.txt"]);
    }

    #[test]
    fn transfer_failure_midway_leaves_no_partial_file() {
        let mut m = MemProvider::new();
        m.fail_read_after = Some(1000);
        let m = Arc::new(m);
        m.add_file("/big", &big());
        m.add_dir("/out");
        let v = vfs(&[("m", &m)]);
        let d = tempfile::tempdir().unwrap();
        let e = copy(&v, &loc("sftp://m/big"), &Loc::Local(d.path().join("big")), false, &mut |_| unreachable!(), &mut |_| true);
        assert_eq!(e.unwrap_err().code, ErrorCode::ConnectionFailed);
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0, "né il file né il .mcpart");
        let e = copy(&v, &loc("sftp://m/big"), &loc("sftp://m/out/big"), false, &mut |_| unreachable!(), &mut |_| true);
        assert_eq!(e.unwrap_err().code, ErrorCode::ConnectionFailed);
        assert!(names(&m, "/out").is_empty());
    }

    #[test]
    fn cancel_removes_temp() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/big", &big());
        m.add_dir("/out");
        let v = vfs(&[("m", &m)]);
        let mut blocks = 0;
        let mut progress = |_| {
            blocks += 1;
            blocks < 2
        };
        let e = copy(&v, &loc("sftp://m/big"), &loc("sftp://m/out/big"), false, &mut |_| unreachable!(), &mut progress);
        assert_eq!(e.unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!(blocks, 2);
        assert!(names(&m, "/out").is_empty());
    }

    /// Senza scrittura sicura: con `false` la destinazione resta intatta, con `true` si sovrascrive. Mai un temporaneo.
    fn unsafe_write_asks(m: MemProvider) {
        let m = Arc::new(m);
        m.add_file("/src", b"nuovo");
        m.add_file("/d/f", b"vecchio");
        let v = vfs(&[("m", &m)]);
        let (src, dst) = (loc("sftp://m/src"), loc("sftp://m/d/f"));
        let mut asked = Vec::new();
        let e = copy(&v, &src, &dst, true, &mut |l| { asked.push(l.to_string()); false }, &mut |_| true);
        assert_eq!(e.unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!((m.data("/d/f").unwrap(), names(&m, "/d")), (b"vecchio".to_vec(), vec!["f".to_owned()]));
        copy(&v, &src, &dst, true, &mut |l| { asked.push(l.to_string()); true }, &mut |_| true).unwrap();
        assert_eq!((m.data("/d/f").unwrap(), names(&m, "/d")), (b"nuovo".to_vec(), vec!["f".to_owned()]));
        assert_eq!(asked, ["sftp://m/d/f", "sftp://m/d/f"]);
    }

    #[test]
    fn unsafe_fallback_needs_confirmation() {
        let mut m = MemProvider::new();
        m.atomic_replace = false;
        unsafe_write_asks(m);
        // La cartella non accetta il temporaneo, il file si riscrive.
        let mut m = MemProvider::new();
        m.writable_dirs = Some(HashSet::new());
        unsafe_write_asks(m);
    }

    #[test]
    fn unsafe_write_is_asked_before_opening_the_source() {
        let mut m = MemProvider::new();
        m.atomic_replace = false;
        let m = Arc::new(m);
        m.add_file("/d/f", b"vecchio");
        let v = vfs(&[("m", &m)]);
        // La sorgente non esiste: se si aprisse prima di chiedere, l'errore sarebbe `NOT_FOUND`.
        let mut asked = 0;
        let e = copy(&v, &loc("sftp://m/manca"), &loc("sftp://m/d/f"), true, &mut |_| { asked += 1; false }, &mut |_| true);
        assert_eq!((e.unwrap_err().code, asked), (ErrorCode::Cancelled, 1));
        assert_eq!(m.data("/d/f").unwrap(), b"vecchio");
    }

    #[test]
    fn unsafe_write_keeps_dst_when_the_source_cannot_be_opened() {
        let mut m = MemProvider::new();
        m.atomic_replace = false;
        let m = Arc::new(m);
        m.add_file("/d/f", b"vecchio");
        let v = vfs(&[("m", &m)]);
        // Confermata la scrittura diretta, la sorgente non si apre: il vecchio file non si tronca.
        let e = copy(&v, &loc("sftp://m/manca"), &loc("sftp://m/d/f"), true, &mut |_| true, &mut |_| true);
        assert_eq!(e.unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(m.data("/d/f").unwrap(), b"vecchio");
    }
}
