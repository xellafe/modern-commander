//! Esecutore elevato di un job. L'helper gira in un'altra sessione di logon (unità `subst` e di rete
//! mappate non ci sono) e rifiuta ogni giunzione nel percorso: le radici del job si risolvono qui,
//! una sola volta, quando il job passa all'helper. Una giunzione comparsa dopo sotto una radice
//! arriva all'helper come giunzione e viene rifiutata; risolvere ogni percorso prima dell'invio la
//! seguirebbe.

use super::exec::Executor;
use super::model::{JobKind, JobSpec, ListEntry};
use super::plan::file_target;
use crate::error::AppError;
use crate::paths::{is_within, normalize_lexical, to_extended};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Arc;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFinalPathNameByHandleW, OPEN_EXISTING, VOLUME_NAME_DOS,
};
use windows::core::HSTRING;

/// Àncora di `dir`: `dir` stesso o, se non esiste, l'antenato esistente più profondo, con il suo
/// percorso risolto da GetFinalPathNameByHandleW (segue giunzioni, `subst` e unità di rete mappate).
/// `None` se non si risolve niente.
pub(crate) fn resolve_dir(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    dir.ancestors().find_map(|a| final_path(a).map(|r| (a.to_path_buf(), r)))
}

/// Percorso finale di una cartella esistente, solo in forma `C:\…` o `\\server\share\…` (quella che
/// l'helper accetta); `None` altrimenti.
fn final_path(dir: &Path) -> Option<PathBuf> {
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(to_extended(dir).as_path()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()?;
    let h = unsafe { OwnedHandle::from_raw_handle(h.0) };
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetFinalPathNameByHandleW(HANDLE(h.as_raw_handle()), &mut buf, VOLUME_NAME_DOS) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            let r = normalize_lexical(Path::new(&OsString::from_wide(&buf[..n])));
            let ok = matches!(r.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::UNC(..)));
            return ok.then_some(r);
        }
        buf.resize(n, 0); // buffer piccolo: `n` è la lunghezza necessaria
    }
}

/// Radici di un job, normalizzate: la cartella padre di ogni sorgente; `dest` se è la cartella di
/// destinazione di Copy/Move, altrimenti (nome di file, `file_target`, o Rename) la sua cartella padre.
/// Una radice non è mai l'elemento su cui si opera: risolverla seguirebbe un collegamento.
pub(crate) fn job_roots(spec: &JobSpec) -> Vec<PathBuf> {
    let parent = |p: &Path| normalize_lexical(p).parent().map(Path::to_path_buf);
    let dest_dir = spec.dest.as_deref().and_then(|d| match spec.kind {
        JobKind::Copy | JobKind::Move if !file_target(spec) => Some(normalize_lexical(d)),
        _ => parent(d),
    });
    spec.sources.iter().filter_map(|s| parent(s)).chain(dest_dir).collect()
}

/// Esecutore elevato di un job: ogni percorso che sta sotto l'àncora di una radice (la più lunga)
/// riceve il prefisso risolto di quell'àncora; gli altri passano invariati. L'àncora (`resolve_dir`)
/// è la radice o, se non esiste ancora, il suo antenato esistente più profondo: così si mappano
/// anche le cartelle intermedie che il job crea. Non è mai l'elemento su cui si opera. Le radici
/// si risolvono una sola volta, alla creazione.
#[derive(Clone)]
pub struct RootedExecutor {
    inner: Arc<dyn Executor>,
    /// (àncora normalizzata, àncora risolta)
    roots: Vec<(PathBuf, PathBuf)>,
}

impl RootedExecutor {
    pub fn new(inner: Arc<dyn Executor>, roots: Vec<PathBuf>) -> Self {
        let roots = roots.iter().filter_map(|r| resolve_dir(&normalize_lexical(r))).collect();
        Self { inner, roots }
    }

    /// Stesse radici con un altro esecutore (l'helper riavviato dopo la sua uscita).
    pub(crate) fn with_inner(mut self, inner: Arc<dyn Executor>) -> Self {
        self.inner = inner;
        self
    }

    fn map(&self, p: &Path) -> PathBuf {
        let n = normalize_lexical(p);
        let root = self.roots.iter().filter(|(r, _)| is_within(&n, r)).max_by_key(|(r, _)| r.components().count());
        match root {
            Some((r, resolved)) => {
                let mut out = resolved.clone();
                out.extend(n.components().skip(r.components().count()));
                out
            }
            None => p.to_path_buf(),
        }
    }
}

impl Executor for RootedExecutor {
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, flush: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
        self.inner.copy_file(&self.map(src), &self.map(dst), overwrite, flush, progress)
    }

    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
        self.inner.move_path(&self.map(src), &self.map(dst), overwrite)
    }

    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
        self.inner.delete(&self.map(path), is_dir, permanent)
    }

    fn mkdir(&self, path: &Path) -> Result<(), AppError> {
        self.inner.mkdir(&self.map(path))
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
        self.inner.rename(&self.map(src), &self.map(dst))
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<ListEntry>, AppError> {
        self.inner.list_dir(&self.map(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elevation::{Request, validate};
    use crate::fs::listing::long_path;
    use crate::ops::model::ConflictPolicy;
    use crate::paths::same_path;
    use std::fs;
    use std::sync::Mutex;

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
        assert!(ok, "mklink /J");
    }

    #[test]
    fn resolve_dir_follows_junctions_and_keeps_unresolvable_paths() {
        let d = tempfile::tempdir().unwrap();
        let (real, link) = (d.path().join("real"), d.path().join("link"));
        fs::create_dir(&real).unwrap();
        junction(&link, &real);
        for p in [link.clone(), link.join(r"nuova\sotto")] {
            let (anchor, r) = resolve_dir(&p).unwrap();
            assert_eq!(anchor, link);
            assert!(same_path(&r, &long_path(&real)), "{r:?}");
            assert!(validate(&Request::Mkdir { path: r }).is_ok(), "{p:?}");
        }
        let absent = (b'D'..=b'Z').map(|c| format!(r"{}:\", c as char)).find(|r| !Path::new(r).exists()).expect("lettera libera");
        assert_eq!(resolve_dir(&Path::new(&absent).join("inesistente")), None);
    }

    /// Registra i percorsi ricevuti senza toccare il disco.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<PathBuf>>);

    impl Executor for Recorder {
        fn copy_file(&self, src: &Path, dst: &Path, _: bool, _: bool, _: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
            self.0.lock().unwrap().extend([src.to_path_buf(), dst.to_path_buf()]);
            Ok(())
        }
        fn move_path(&self, src: &Path, dst: &Path, _: bool) -> Result<(), AppError> {
            self.0.lock().unwrap().extend([src.to_path_buf(), dst.to_path_buf()]);
            Ok(())
        }
        fn delete(&self, _: &Path, _: bool, _: bool) -> Result<(), AppError> {
            unimplemented!()
        }
        fn mkdir(&self, path: &Path) -> Result<(), AppError> {
            self.0.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
        fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
            self.0.lock().unwrap().extend([src.to_path_buf(), dst.to_path_buf()]);
            Ok(())
        }
        fn list_dir(&self, _: &Path) -> Result<Vec<ListEntry>, AppError> {
            unimplemented!()
        }
    }

    #[test]
    fn rooted_executor_maps_under_roots_only_and_resolves_once() {
        let d = tempfile::tempdir().unwrap();
        let (real, link, altro, altrove) = (d.path().join("real"), d.path().join("link"), d.path().join("altro"), d.path().join("altrove"));
        for p in [&real, &altro, &altrove] {
            fs::create_dir(p).unwrap();
        }
        junction(&link, &real);
        let rec = Arc::new(Recorder::default());
        let x = RootedExecutor::new(rec.clone(), vec![link.clone(), altro.clone(), d.path().to_path_buf()]);
        x.mkdir(&link.join(r"a\b")).unwrap();
        let fuori = Path::new(r"\\srv\share\fuori\x");
        x.mkdir(fuori).unwrap();
        fs::remove_dir(&link).unwrap();
        junction(&link, &altrove);
        x.mkdir(&link.join("c")).unwrap();
        let got = rec.0.lock().unwrap().clone();
        assert!(same_path(&got[0], &long_path(&real).join(r"a\b")), "{got:?}");
        assert_eq!(got[1], fuori);
        assert!(same_path(&got[2], &long_path(&real).join("c")), "{got:?}");
    }

    /// Radice non ancora esistente sotto una giunzione legittima: si mappa dall'antenato risolto,
    /// anche per le cartelle intermedie che il job crea.
    #[test]
    fn missing_root_maps_through_the_junction_above() {
        let d = tempfile::tempdir().unwrap();
        let (real, link) = (d.path().join("real"), d.path().join("link"));
        fs::create_dir(&real).unwrap();
        junction(&link, &real);
        let rec = Arc::new(Recorder::default());
        let x = RootedExecutor::new(rec.clone(), vec![link.join(r"a\b")]);
        x.mkdir(&link.join("a")).unwrap();
        x.mkdir(&link.join(r"a\b\c")).unwrap();
        let got = rec.0.lock().unwrap().clone();
        assert!(same_path(&got[0], &long_path(&real).join("a")), "{got:?}");
        assert!(same_path(&got[1], &long_path(&real).join(r"a\b\c")), "{got:?}");
    }

    fn spec(kind: JobKind, src: PathBuf, dest: PathBuf) -> JobSpec {
        JobSpec { kind, sources: vec![src], dest: Some(dest), filter: None, policy: ConflictPolicy::Ask }
    }

    /// Una destinazione che è un symlink a file non diventa una radice: la mappatura non lo segue.
    #[test]
    fn file_destination_link_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let (f, target, link) = (d.path().join("f.txt"), d.path().join("target.txt"), d.path().join("link.txt"));
        fs::write(&f, b"x").unwrap();
        fs::write(&target, b"t").unwrap();
        if let Err(e) = std::os::windows::fs::symlink_file(&target, &link) {
            eprintln!("test saltato: symlink non creabile ({e})");
            return;
        }
        for kind in [JobKind::Copy, JobKind::Move] {
            let rec = Arc::new(Recorder::default());
            let x = RootedExecutor::new(rec.clone(), job_roots(&spec(kind, f.clone(), link.clone())));
            x.copy_file(&f, &link, true, false, &mut |_| true).unwrap();
            x.move_path(&f, &link, true).unwrap();
            let got = rec.0.lock().unwrap().clone();
            for dst in [&got[1], &got[3]] {
                assert!(same_path(dst, &long_path(d.path()).join("link.txt")), "{got:?}");
            }
        }
    }

    #[test]
    fn case_only_rename_keeps_the_new_casing() {
        let d = tempfile::tempdir().unwrap();
        let (a, upper) = (d.path().join("a"), d.path().join("A"));
        fs::create_dir(&a).unwrap();
        let rec = Arc::new(Recorder::default());
        let x = RootedExecutor::new(rec.clone(), job_roots(&spec(JobKind::Rename, a.clone(), upper.clone())));
        x.rename(&a, &upper).unwrap();
        let got = rec.0.lock().unwrap().clone();
        assert_eq!(got[0].file_name().unwrap(), "a", "{got:?}");
        assert_eq!(got[1].file_name().unwrap(), "A", "{got:?}");
    }
}
