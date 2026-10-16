//! Espansione di un `JobSpec` nella lista ordinata di elementi da elaborare.

use super::model::{FileItem, JobKind, JobSpec};
use crate::error::{AppError, ErrorCode};
use crate::mask::match_mask;
use crate::paths::{is_within, normalize_lexical, same_path, to_extended, volume_key};
use std::fs::Metadata;
use std::os::windows::fs::FileTypeExt;
use std::path::{Path, PathBuf};

fn invalid(p: Option<&Path>) -> AppError {
    AppError::new(ErrorCode::InvalidRequest, p)
}

fn stat(p: &Path) -> Result<Metadata, AppError> {
    std::fs::symlink_metadata(to_extended(p)).map_err(|e| AppError::from_io(&e, p))
}

/// Cartella vera oppure collegamento (giunzione / symlink) a una cartella.
fn is_dir_like(m: &Metadata) -> bool {
    m.is_dir() || m.file_type().is_symlink_dir()
}

fn item(src: &Path, dst: Option<PathBuf>, m: &Metadata) -> FileItem {
    let is_dir = is_dir_like(m);
    FileItem { src: src.to_path_buf(), dst, is_dir, size: if is_dir { 0 } else { m.len() } }
}

struct Walker<'a> {
    filter: Option<&'a str>,
    /// Delete: contenuto prima della cartella. Copy/Move: cartella prima del contenuto.
    post_order: bool,
    out: Vec<FileItem>,
}

impl Walker<'_> {
    /// `true` se `src` è rimasto nell'elenco (per le cartelle: se non è vuoto dopo il filtro).
    fn visit(&mut self, src: &Path, dst: Option<PathBuf>, m: &Metadata) -> Result<bool, AppError> {
        // `Metadata::is_dir` è falso per giunzioni e symlink: non si seguono mai.
        // ponytail: un collegamento a cartella è una foglia (is_dir, size 0); Copy/Move lo
        // passano all'executor che decide, Delete elimina il collegamento e mai il contenuto del
        // bersaglio. Seguirlo (con protezione dai cicli) se servirà copiare il contenuto.
        if !m.is_dir() {
            let keep = match self.filter {
                None => true,
                // Il filtro vale per i file; un collegamento a cartella con filtro non si tocca.
                Some(_) if is_dir_like(m) => false,
                Some(mask) => src.file_name().is_some_and(|n| match_mask(&n.to_string_lossy(), mask)),
            };
            if keep {
                self.out.push(item(src, dst, m));
            }
            return Ok(keep);
        }
        let mut children = Vec::new();
        for e in std::fs::read_dir(to_extended(src)).map_err(|e| AppError::from_io(&e, src))? {
            let e = e.map_err(|e| AppError::from_io(&e, src))?;
            let name = e.file_name();
            // Su Windows `DirEntry::metadata` non segue i collegamenti e non fa chiamate extra.
            let cm = e.metadata().map_err(|err| AppError::from_io(&err, &src.join(&name)))?;
            children.push((name, cm));
        }
        // Ordine stabile: prima i file, poi le cartelle, per nome.
        children.sort_by_cached_key(|(n, cm)| (is_dir_like(cm), n.to_string_lossy().to_lowercase()));

        let at = self.out.len();
        if !self.post_order {
            self.out.push(item(src, dst.clone(), m));
        }
        let mut any = false;
        for (name, cm) in children {
            any |= self.visit(&src.join(&name), dst.as_ref().map(|d| d.join(&name)), &cm)?;
        }
        let keep = self.filter.is_none() || any;
        if self.post_order {
            // Con un filtro le cartelle non si eliminano: conterrebbero i file esclusi.
            if self.filter.is_none() {
                self.out.push(item(src, dst, m));
            }
        } else if !keep {
            self.out.truncate(at); // cartella vuota dopo il filtro: non si crea
        }
        Ok(keep)
    }
}

pub fn expand(spec: &JobSpec) -> Result<Vec<FileItem>, AppError> {
    let first = spec.sources.first().ok_or_else(|| invalid(None))?;
    let filter = spec.filter.as_deref().filter(|f| !f.trim().is_empty());
    let mut walker = Walker { filter, post_order: false, out: Vec::new() };
    match &spec.kind {
        JobKind::Mkdir => Ok(vec![FileItem { src: first.clone(), dst: None, is_dir: true, size: 0 }]),
        JobKind::Rename => {
            let dest = spec.dest.clone().ok_or_else(|| invalid(Some(first)))?;
            if spec.sources.len() != 1 {
                return Err(invalid(Some(first)));
            }
            let m = stat(first)?;
            let (ns, nd) = (normalize_lexical(first), normalize_lexical(&dest));
            // Cambiare solo le maiuscole (`a` → `A`) è lecito; finire dentro se stesso no.
            if is_within(&nd, &ns) && !same_path(&nd, &ns) {
                return Err(AppError::new(ErrorCode::IntoItself, Some(first)));
            }
            Ok(vec![item(first, Some(nd), &m)])
        }
        JobKind::Delete { .. } => {
            walker.post_order = true;
            for s in &spec.sources {
                // Mai un'intera unità o condivisione.
                if normalize_lexical(s).file_name().is_none() {
                    return Err(invalid(Some(s)));
                }
                walker.visit(s, None, &stat(s)?)?;
            }
            Ok(walker.out)
        }
        JobKind::Copy | JobKind::Move => {
            let is_move = spec.kind == JobKind::Move;
            let dest = spec.dest.as_ref().ok_or_else(|| invalid(Some(first)))?;
            // Forma comune (assoluta, senza `\\?\`, `..` risolti) per i confronti e per costruire
            // le destinazioni: in un percorso `\\?\` Windows non risolve `..`.
            let ndest = normalize_lexical(dest);
            let mut checked = Vec::with_capacity(spec.sources.len());
            for s in &spec.sources {
                let m = stat(s)?;
                let ns = normalize_lexical(s);
                let name = ns.file_name().ok_or_else(|| invalid(Some(s)))?.to_os_string();
                if is_within(&ndest, &ns) {
                    return Err(AppError::new(ErrorCode::IntoItself, Some(s)));
                }
                if is_move && ns.parent().is_some_and(|p| same_path(p, &ndest)) {
                    return Err(AppError::new(ErrorCode::SameFile, Some(s)));
                }
                // ponytail: stessa unità = stessa lettera/condivisione; un volume montato in una
                // cartella o un'unità `subst` danno ERROR_NOT_SAME_DEVICE e l'executor ripiega su copia.
                let vol = volume_key(&ns);
                let rename = is_move && filter.is_none() && !vol.is_empty() && vol == volume_key(&ndest);
                checked.push((s, m, ndest.join(name), rename));
            }
            for (s, m, dst, rename) in checked {
                if rename {
                    walker.out.push(item(s, Some(dst), &m)); // rename: nessuna espansione
                } else {
                    walker.visit(s, Some(dst), &m)?;
                }
            }
            Ok(walker.out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::ops::model::{ConflictPolicy, JobKind};
    use crate::paths::to_extended;
    use std::path::{Path, PathBuf};

    fn spec(kind: JobKind, sources: Vec<PathBuf>, dest: Option<PathBuf>, filter: Option<&str>) -> JobSpec {
        JobSpec { kind, sources, dest, filter: filter.map(str::to_string), policy: ConflictPolicy::Ask }
    }

    fn srcs(items: &[FileItem]) -> Vec<PathBuf> {
        items.iter().map(|i| i.src.clone()).collect()
    }

    /// a/{x.txt (3 byte), s/y.txt}
    fn tree(root: &Path) -> PathBuf {
        let a = root.join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        std::fs::write(a.join("x.txt"), b"abc").unwrap();
        std::fs::write(a.join("s").join("y.txt"), b"").unwrap();
        a
    }

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
    }

    #[test]
    fn copy_into_itself_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        let s = spec(JobKind::Copy, vec![a.clone()], Some(a.join("b")), None);
        assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself);
    }

    #[test]
    fn into_itself_detected_across_path_forms() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        for dest in [to_extended(&a.join("b")), a.clone(), a.join("b").join("..").join("B"), d.path().join("A\\")] {
            let s = spec(JobKind::Move, vec![a.clone()], Some(dest.clone()), None);
            assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself, "{dest:?}");
        }
        let s = spec(JobKind::Copy, vec![to_extended(&a)], Some(a.join("b")), None);
        assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself);
    }

    #[test]
    fn copy_into_own_parent_is_allowed() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().to_path_buf()), None)).unwrap();
        assert_eq!(items[0].dst.as_deref(), Some(a.as_path()));
    }

    #[test]
    fn destinations_are_built_from_normalized_dest() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = to_extended(d.path()).join("x").join("..").join("D");
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(dest), None)).unwrap();
        let da = d.path().join("D").join("a");
        assert_eq!(items[0].dst.as_deref(), Some(da.as_path()));
        assert_eq!(items[3].dst, Some(da.join("s").join("y.txt")));
        let renamed = d.path().join("x").join("..").join("b");
        let items = expand(&spec(JobKind::Rename, vec![a], Some(renamed), None)).unwrap();
        assert_eq!(items[0].dst, Some(d.path().join("b")));
    }

    #[test]
    fn move_into_same_parent_is_same_file() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = to_extended(d.path()).join(".");
        assert_eq!(expand(&spec(JobKind::Move, vec![a], Some(dest), None)).unwrap_err().code, ErrorCode::SameFile);
    }

    #[test]
    fn copy_expands_dirs_before_children_and_maps_dest() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt"), a.join("s"), a.join("s").join("y.txt")]);
        let dsts: Vec<_> = items.iter().map(|i| i.dst.clone().unwrap()).collect();
        let da = dest.join("a");
        assert_eq!(dsts, vec![da.clone(), da.join("x.txt"), da.join("s"), da.join("s").join("y.txt")]);
        assert_eq!(items.iter().map(|i| (i.is_dir, i.size)).collect::<Vec<_>>(), vec![(true, 0), (false, 3), (true, 0), (false, 0)]);
    }

    #[test]
    fn copy_keeps_empty_dirs_without_filter_and_accepts_files() {
        let d = tempfile::tempdir().unwrap();
        let e = d.path().join("empty");
        std::fs::create_dir(&e).unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"12").unwrap();
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Copy, vec![e.clone(), f.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(
            items,
            vec![
                FileItem { src: e, dst: Some(dest.join("empty")), is_dir: true, size: 0 },
                FileItem { src: f, dst: Some(dest.join("f.txt")), is_dir: false, size: 2 },
            ]
        );
    }

    #[test]
    fn delete_expands_children_before_dirs() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let items = expand(&spec(JobKind::Delete { permanent: false }, vec![a.clone()], None, None)).unwrap();
        assert_eq!(srcs(&items), vec![a.join("x.txt"), a.join("s").join("y.txt"), a.join("s"), a.clone()]);
        assert!(items.iter().all(|i| i.dst.is_none()));
    }

    #[test]
    fn filter_applies_to_files_only() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        for f in ["x.txt", "y.md", "s/z.md"] {
            std::fs::write(a.join(f), b"").unwrap();
        }
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().join("D")), Some("*.txt"))).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt")]);
        // Nessun file che passa il filtro: niente da creare.
        assert!(expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().join("D")), Some("*.zip"))).unwrap().is_empty());
        // Con filtro, Delete non elimina cartelle: conterrebbero file esclusi.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, Some("*.md"))).unwrap();
        assert_eq!(srcs(&del), vec![a.join("y.md"), a.join("s").join("z.md")]);
    }

    #[test]
    fn move_on_same_volume_is_one_item_per_source() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(items, vec![FileItem { src: a.clone(), dst: Some(dest.join("a")), is_dir: true, size: 0 }]);
        // Con un filtro il rename in blocco sposterebbe anche i file esclusi: si espande.
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(dest), Some("x.*"))).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt")]);
    }

    #[test]
    fn move_across_volumes_is_expanded() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let other = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
        if crate::paths::volume_key(d.path()) == crate::paths::volume_key(other.path()) {
            return; // temp e sorgenti sulla stessa unità: caso non riproducibile qui.
        }
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(other.path().to_path_buf()), None)).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[3].dst, Some(other.path().join("a").join("s").join("y.txt")));
    }

    #[test]
    fn missing_source_is_not_found_with_path() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        for kind in [JobKind::Copy, JobKind::Move, JobKind::Delete { permanent: false }, JobKind::Rename] {
            let e = expand(&spec(kind, vec![gone.clone()], Some(d.path().join("D")), None)).unwrap_err();
            assert_eq!(e.code, ErrorCode::NotFound);
            assert_eq!(e.path.as_deref(), Some(gone.to_str().unwrap()));
        }
    }

    #[test]
    fn invalid_requests() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let bad = [
            spec(JobKind::Copy, vec![a.clone()], None, None),
            spec(JobKind::Copy, vec![], Some(d.path().join("D")), None),
            spec(JobKind::Rename, vec![a.clone(), a.clone()], Some(d.path().join("b")), None),
            spec(JobKind::Mkdir, vec![], None, None),
            spec(JobKind::Copy, vec![PathBuf::from(r"C:\")], Some(d.path().join("D")), None),
            spec(JobKind::Delete { permanent: true }, vec![PathBuf::from(r"C:\")], None, None),
            spec(JobKind::Delete { permanent: false }, vec![PathBuf::from(r"\\?\C:\")], None, None),
            spec(JobKind::Delete { permanent: true }, vec![PathBuf::from(r"\\srv\sh\")], None, None),
        ];
        for s in bad {
            assert_eq!(expand(&s).unwrap_err().code, ErrorCode::InvalidRequest, "{s:?}");
        }
    }

    #[test]
    fn rename_and_mkdir_are_single_items() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let b = d.path().join("b");
        let items = expand(&spec(JobKind::Rename, vec![a.clone()], Some(b.clone()), None)).unwrap();
        assert_eq!(items, vec![FileItem { src: a.clone(), dst: Some(b), is_dir: true, size: 0 }]);
        let upper = d.path().join("A");
        assert_eq!(expand(&spec(JobKind::Rename, vec![a.clone()], Some(upper.clone()), None)).unwrap()[0].dst, Some(upper));
        let inside = spec(JobKind::Rename, vec![a.clone()], Some(a.join("s").join("a")), None);
        assert_eq!(expand(&inside).unwrap_err().code, ErrorCode::IntoItself);
        let n = d.path().join("new");
        let items = expand(&spec(JobKind::Mkdir, vec![n.clone()], None, None)).unwrap();
        assert_eq!(items, vec![FileItem { src: n, dst: None, is_dir: true, size: 0 }]);
    }

    #[test]
    fn junctions_are_leaves() {
        let d = tempfile::tempdir().unwrap();
        let target = tree(d.path());
        let p = d.path().join("p");
        std::fs::create_dir(&p).unwrap();
        let link = p.join("link");
        junction(&link, &target);

        // Eliminare la cartella che contiene la giunzione non tocca il contenuto del bersaglio.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![p.clone()], None, None)).unwrap();
        assert_eq!(srcs(&del), vec![link.clone(), p.clone()]);
        // Eliminare la giunzione stessa: un solo elemento, il collegamento.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![link.clone()], None, None)).unwrap();
        assert_eq!(del, vec![FileItem { src: link.clone(), dst: None, is_dir: true, size: 0 }]);

        let dest = d.path().join("D");
        let cp = expand(&spec(JobKind::Copy, vec![p.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(cp[1], FileItem { src: link, dst: Some(dest.join("p").join("link")), is_dir: true, size: 0 });
        assert_eq!(cp.len(), 2);
    }
}
