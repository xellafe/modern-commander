//! Scansione incrementale di un albero per il salto rapido (Ctrl+P): visita in ampiezza, a lotti,
//! senza seguire i reparse point né le cartelle escluse. Annullabile con il flag del registro.

use super::listing::walk_dir;
use crate::error::{AppError, ErrorCode};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Voci per lotto consegnato a `emit`.
const BATCH: usize = 500;
/// Ogni quante voci si controlla l'annullamento dentro una cartella.
const CANCEL_EVERY: usize = 1000;

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_REPARSE: u32 = 0x400;

/// Cartelle che non si scandagliano (né si elencano).
pub const EXCLUDED_DIRS: [&str; 2] = [".git", "node_modules"]; // ponytail: elenco fisso, configurabile se serve

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanEntry {
    /// Percorso relativo alla radice, con `\`.
    pub rel: String,
    pub is_dir: bool,
}

pub struct ScanLimits {
    /// Profondità massima delle voci: 1 = solo il contenuto diretto della radice.
    pub max_depth: u32,
    pub max_entries: u32,
    /// Falso: si scartano le voci con attributo hidden o system.
    pub include_hidden: bool,
}

/// Elenca l'albero sotto `root` a lotti di 500 voci, prima tutta una profondità poi la successiva.
/// `Ok(true)` = troncata al limite di voci. Solo la radice illeggibile è un errore.
pub fn scan_tree(
    root: &Path,
    limits: &ScanLimits,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Vec<ScanEntry>),
) -> Result<bool, AppError> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let cancel_err = || AppError::new(ErrorCode::Cancelled, Some(root));
    let mut batch = Vec::with_capacity(BATCH);
    let (mut count, mut seen) = (0u32, 0usize);
    let mut truncated = false;
    // Cartelle ancora da leggere: percorso, `rel` del prefisso (vuoto per la radice) e profondità.
    let mut queue = VecDeque::from([(root.to_path_buf(), String::new(), 0u32)]);
    while let Some((dir, prefix, depth)) = queue.pop_front() {
        if cancelled() {
            return Err(cancel_err());
        }
        if depth >= limits.max_depth {
            break; // la coda è per profondità crescente: le altre sono oltre il limite
        }
        let walked = walk_dir(&dir, &mut |e| {
            seen += 1;
            if seen % CANCEL_EVERY == 0 && cancelled() {
                return false;
            }
            if !limits.include_hidden && e.attributes & (ATTR_HIDDEN | ATTR_SYSTEM) != 0 {
                return true;
            }
            if e.is_dir && EXCLUDED_DIRS.iter().any(|x| e.name.eq_ignore_ascii_case(x)) {
                return true;
            }
            if count >= limits.max_entries {
                truncated = true;
                return false;
            }
            count += 1;
            let rel = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}\\{}", e.name) };
            // Mai dentro i reparse point (giunzioni, collegamenti, segnaposto cloud): si elencano e basta.
            if e.is_dir && e.attributes & ATTR_REPARSE == 0 && depth + 1 < limits.max_depth {
                queue.push_back((dir.join(&e.name), rel.clone(), depth + 1));
            }
            batch.push(ScanEntry { rel, is_dir: e.is_dir });
            if batch.len() == BATCH {
                emit(std::mem::replace(&mut batch, Vec::with_capacity(BATCH)));
            }
            true
        });
        match walked {
            Ok(true) => {}
            Ok(false) if truncated => break,
            Ok(false) => return Err(cancel_err()),
            Err(e) if depth == 0 => return Err(e),
            Err(_) => {} // sottocartella illeggibile (accesso negato, sparita...): saltata
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    Ok(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
    }

    fn limits(max_depth: u32, max_entries: u32, include_hidden: bool) -> ScanLimits {
        ScanLimits { max_depth, max_entries, include_hidden }
    }

    /// Tutte le voci, nell'ordine di consegna, più l'esito.
    fn scan(root: &Path, l: &ScanLimits) -> (Vec<ScanEntry>, bool) {
        let mut all = Vec::new();
        let truncated = scan_tree(root, l, &AtomicBool::new(false), &mut |b| all.extend(b)).unwrap();
        (all, truncated)
    }

    fn rels(v: &[ScanEntry]) -> Vec<&str> {
        v.iter().map(|e| e.rel.as_str()).collect()
    }

    #[test]
    fn breadth_first_with_depth_limit() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join(r"a\b\c\d.txt"));
        touch(&d.path().join(r"a\x.txt"));
        touch(&d.path().join("top.txt"));
        let (v, truncated) = scan(d.path(), &limits(2, 100, true));
        assert!(!truncated);
        let depth = |e: &ScanEntry| e.rel.split('\\').count();
        assert!(v.iter().all(|e| depth(e) <= 2), "{:?}", rels(&v));
        // tutte le voci a profondità 1 prima di quelle a profondità 2
        let depths: Vec<usize> = v.iter().map(depth).collect();
        assert!(depths.windows(2).all(|w| w[0] <= w[1]), "{depths:?}");
        let mut got = rels(&v);
        got.sort();
        assert_eq!(got, [r"a", r"a\b", r"a\x.txt", "top.txt"]);
        assert!(v.iter().find(|e| e.rel == "a").unwrap().is_dir);
        assert!(!v.iter().find(|e| e.rel == "top.txt").unwrap().is_dir);
    }

    #[test]
    fn entry_limit_truncates() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..30 {
            touch(&d.path().join(format!("f{i}.txt")));
        }
        let (v, truncated) = scan(d.path(), &limits(8, 10, true));
        assert_eq!(v.len(), 10);
        assert!(truncated);
        // con esattamente tante voci quante il limite non si tronca
        let (v, truncated) = scan(d.path(), &limits(8, 30, true));
        assert_eq!(v.len(), 30);
        assert!(!truncated);
    }

    #[test]
    fn batches_of_500() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..1200 {
            touch(&d.path().join(format!("f{i}")));
        }
        let mut sizes = Vec::new();
        scan_tree(d.path(), &limits(8, 20_000, true), &AtomicBool::new(false), &mut |b| sizes.push(b.len())).unwrap();
        assert_eq!(sizes, [500, 500, 200]);
    }

    #[test]
    fn excluded_and_hidden() {
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
        use windows::core::HSTRING;
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join(r".git\cfg"));
        touch(&d.path().join(r"node_modules\m.js"));
        touch(&d.path().join("h.txt"));
        touch(&d.path().join("v.txt"));
        unsafe { SetFileAttributesW(&HSTRING::from(d.path().join("h.txt").as_path()), FILE_ATTRIBUTE_HIDDEN) }.unwrap();
        let (v, _) = scan(d.path(), &limits(8, 100, false));
        assert_eq!(rels(&v), ["v.txt"]);
        let (v, _) = scan(d.path(), &limits(8, 100, true));
        let mut got = rels(&v);
        got.sort();
        assert_eq!(got, ["h.txt", "v.txt"]); // .git e node_modules restano fuori anche con i nascosti
    }

    #[test]
    fn junction_is_listed_but_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        touch(&target.path().join("f.txt"));
        let link = d.path().join("link");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(target.path())
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        let (v, _) = scan(d.path(), &limits(8, 100, true));
        assert_eq!(rels(&v), ["link"]);
        assert!(v[0].is_dir);
    }

    #[test]
    fn cancel_stops() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.txt"));
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::Relaxed);
        let mut emitted = false;
        let e = scan_tree(d.path(), &limits(8, 100, true), &cancel, &mut |_| emitted = true).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert!(!emitted);
    }

    #[test]
    fn missing_root_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let e = scan_tree(&d.path().join("gone"), &limits(8, 10, true), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
    }
}
