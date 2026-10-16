//! Risoluzione dei conflitti di destinazione secondo la politica scelta.

use super::model::{ConflictPolicy, Decision, Meta};
use crate::error::{AppError, ErrorCode};
use crate::paths::{is_within, normalize_lexical, to_extended};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn same_path(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

/// Stesso file: `canonicalize` di entrambi (risolve maiuscole, `..` e collegamenti simbolici);
/// se uno dei due non esiste, confronto dei percorsi normalizzati.
// ponytail: due hard link dello stesso file non vengono riconosciuti; servirebbe confrontare
// volume serial + file index (GetFileInformationByHandle) se l'executor sovrascrive troncando.
fn same_file(src: &Path, dst: &Path) -> bool {
    match (std::fs::canonicalize(to_extended(src)), std::fs::canonicalize(to_extended(dst))) {
        (Ok(a), Ok(b)) => same_path(&a, &b),
        _ => same_path(&normalize_lexical(src), &normalize_lexical(dst)),
    }
}

/// `None` = la decisione spetta all'utente. Se sorgente e destinazione sono lo stesso file
/// l'unica politica ammessa è `RenameNew`; ogni altra dà `SAME_FILE` (sovrascrivere un file con
/// se stesso lo distruggerebbe).
pub fn resolve(
    policy: &ConflictPolicy,
    src: &Path,
    dst: &Path,
    src_meta: &Meta,
    dst_meta: &Meta,
) -> Result<Option<Decision>, AppError> {
    let rename = || Some(Decision::Rename(next_free_name(dst)));
    if same_file(src, dst) {
        return match policy {
            ConflictPolicy::RenameNew => Ok(rename()),
            _ => Err(AppError::new(ErrorCode::SameFile, Some(src))),
        };
    }
    let pick = |overwrite: bool| Some(if overwrite { Decision::Overwrite } else { Decision::Skip });
    Ok(match policy {
        ConflictPolicy::Ask => None,
        ConflictPolicy::Overwrite => Some(Decision::Overwrite),
        ConflictPolicy::Skip => Some(Decision::Skip),
        ConflictPolicy::OverwriteIfNewer => pick(src_meta.modified_ms > dst_meta.modified_ms),
        ConflictPolicy::OverwriteIfLarger => pick(src_meta.size > dst_meta.size),
        ConflictPolicy::RenameNew => rename(),
    })
}

/// Primo `nome (n).ext` libero accanto a `dst`, con n da 2.
pub fn next_free_name(dst: &Path) -> PathBuf {
    let stem = dst.file_stem().unwrap_or_default();
    let ext = dst.extension();
    (2u64..)
        .map(|n| {
            let mut name = OsString::from(stem);
            name.push(format!(" ({n})"));
            if let Some(e) = ext {
                name.push(".");
                name.push(e);
            }
            dst.with_file_name(name)
        })
        // `symlink_metadata`: anche un collegamento rotto occupa il nome.
        .find(|c| std::fs::symlink_metadata(to_extended(c)).is_err())
        .expect("intervallo infinito")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn same_file_never_overwritten() {
        let p = Path::new(r"C:\x\f.txt");
        let m = Meta { size: 1, modified_ms: 1 };
        assert_eq!(resolve(&ConflictPolicy::Overwrite, p, p, &m, &m).unwrap_err().code, ErrorCode::SameFile);
        assert!(matches!(resolve(&ConflictPolicy::RenameNew, p, p, &m, &m).unwrap(), Some(Decision::Rename(_))));
        // Forme diverse dello stesso percorso inesistente.
        let q = Path::new(r"\\?\c:\X\.\f.txt");
        assert_eq!(resolve(&ConflictPolicy::Ask, p, q, &m, &m).unwrap_err().code, ErrorCode::SameFile);
    }

    #[test]
    fn same_existing_file_via_canonicalize() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let m = Meta { size: 1, modified_ms: 1 };
        let alias = d.path().join("sub").join("..").join("F.TXT");
        assert_eq!(resolve(&ConflictPolicy::Overwrite, &f, &alias, &m, &m).unwrap_err().code, ErrorCode::SameFile);
    }

    #[test]
    fn policies() {
        let (old, new) = (Meta { size: 1, modified_ms: 1 }, Meta { size: 2, modified_ms: 2 });
        let (s, d) = (Path::new(r"C:\a\f"), Path::new(r"D:\a\f"));
        assert_eq!(resolve(&ConflictPolicy::OverwriteIfNewer, s, d, &new, &old).unwrap(), Some(Decision::Overwrite));
        assert_eq!(resolve(&ConflictPolicy::OverwriteIfNewer, s, d, &old, &new).unwrap(), Some(Decision::Skip));
        assert_eq!(resolve(&ConflictPolicy::OverwriteIfLarger, s, d, &old, &new).unwrap(), Some(Decision::Skip));
        assert_eq!(resolve(&ConflictPolicy::OverwriteIfLarger, s, d, &new, &old).unwrap(), Some(Decision::Overwrite));
        assert_eq!(resolve(&ConflictPolicy::Ask, s, d, &old, &new).unwrap(), None);
        assert_eq!(resolve(&ConflictPolicy::Overwrite, s, d, &old, &new).unwrap(), Some(Decision::Overwrite));
        assert_eq!(resolve(&ConflictPolicy::Skip, s, d, &old, &new).unwrap(), Some(Decision::Skip));
    }

    #[test]
    fn free_name() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"").unwrap();
        std::fs::write(d.path().join("f (2).txt"), b"").unwrap();
        assert_eq!(next_free_name(&f), d.path().join("f (3).txt"));
        let n = d.path().join("noext");
        std::fs::create_dir(&n).unwrap();
        assert_eq!(next_free_name(&n), d.path().join("noext (2)"));
    }

    #[test]
    fn rename_new_uses_free_name() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"").unwrap();
        let m = Meta { size: 0, modified_ms: 0 };
        let got = resolve(&ConflictPolicy::RenameNew, Path::new(r"C:\other\f.txt"), &f, &m, &m).unwrap();
        assert_eq!(got, Some(Decision::Rename(d.path().join("f (2).txt"))));
    }
}
