//! Risoluzione dei conflitti di destinazione secondo la politica scelta.

use super::model::{ConflictPolicy, Decision, Meta};
use crate::error::{AppError, ErrorCode};
use crate::paths::{normalize_lexical, same_path, to_extended};
use std::ffi::OsString;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
};

/// Identità del file (numero di serie del volume, indice del file): è la stessa per tutti gli
/// hard link e per un collegamento simbolico e il suo bersaglio. `None` se non si può aprire.
fn file_id(p: &Path) -> Option<(u32, u64)> {
    let f = std::fs::OpenOptions::new()
        .access_mode(0) // nessun diritto di lettura/scrittura: bastano gli attributi
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0) // necessario per aprire anche le cartelle
        .open(to_extended(p))
        .ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info) }.ok()?;
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((info.dwVolumeSerialNumber, index))
}

/// Stesso file: se esistono entrambi si confronta l'identità del file (copre maiuscole, `..`,
/// collegamenti e hard link); altrimenti i percorsi normalizzati.
fn same_file(src: &Path, dst: &Path) -> bool {
    match (file_id(src), file_id(dst)) {
        (Some(a), Some(b)) => a == b,
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
    fn same_existing_file_via_alias_and_hard_link() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let m = Meta { size: 1, modified_ms: 1 };
        let alias = d.path().join("sub").join("..").join("F.TXT");
        assert_eq!(resolve(&ConflictPolicy::Overwrite, &f, &alias, &m, &m).unwrap_err().code, ErrorCode::SameFile);

        let b = d.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let link = b.join("f.txt");
        std::fs::hard_link(&f, &link).unwrap();
        assert_eq!(resolve(&ConflictPolicy::Overwrite, &f, &link, &m, &m).unwrap_err().code, ErrorCode::SameFile);
        // Due file distinti con lo stesso contenuto restano diversi; anche due cartelle.
        let other = b.join("g.txt");
        std::fs::write(&other, b"x").unwrap();
        assert_eq!(resolve(&ConflictPolicy::Overwrite, &f, &other, &m, &m).unwrap(), Some(Decision::Overwrite));
        assert_eq!(resolve(&ConflictPolicy::Skip, d.path(), &b, &m, &m).unwrap(), Some(Decision::Skip));
        assert_eq!(resolve(&ConflictPolicy::Skip, &b, &b.join("."), &m, &m).unwrap_err().code, ErrorCode::SameFile);
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
