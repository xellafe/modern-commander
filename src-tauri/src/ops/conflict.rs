//! Risoluzione dei conflitti di destinazione secondo la politica scelta.

use super::model::{ConflictPolicy, Decision, Meta};
use crate::error::{AppError, ErrorCode};
use crate::paths::{normalize_lexical, same_path, to_extended};
use crate::vfs::loc::Loc;
use crate::vfs::provider::Purpose;
use crate::vfs::router::Vfs;
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
    known_id(info.dwVolumeSerialNumber, index)
}

/// Un numero di serie o un indice 0 non identificano niente (file system che non li forniscono):
/// due file diversi sembrerebbero lo stesso.
fn known_id(serial: u32, index: u64) -> Option<(u32, u64)> {
    (serial != 0 && index != 0).then_some((serial, index))
}

/// Stesso file: se esistono entrambi si confronta l'identità del file (copre maiuscole, `..`,
/// collegamenti e hard link); altrimenti i percorsi normalizzati.
pub(crate) fn same_file(src: &Path, dst: &Path) -> bool {
    match (file_id(src), file_id(dst)) {
        (Some(a), Some(b)) => a == b,
        _ => same_path(&normalize_lexical(src), &normalize_lexical(dst)),
    }
}

/// Stesso elemento: in locale come `same_file`; sul remoto si decide sulla posizione, senza aprire niente.
pub(crate) fn same_loc(src: &Loc, dst: &Loc) -> bool {
    match (src, dst) {
        (Loc::Local(s), Loc::Local(d)) => same_file(s, d),
        _ => src.same(dst),
    }
}

/// `None` = la decisione spetta all'utente. Se sorgente e destinazione sono lo stesso file
/// l'unica politica ammessa è `RenameNew`; ogni altra dà `SAME_FILE` (sovrascrivere un file con
/// se stesso lo distruggerebbe).
pub fn resolve(
    vfs: &Vfs,
    policy: &ConflictPolicy,
    src: &Loc,
    dst: &Loc,
    src_meta: &Meta,
    dst_meta: &Meta,
) -> Result<Option<Decision>, AppError> {
    let rename = || Some(Decision::Rename(free_name(vfs, dst)));
    if same_loc(src, dst) {
        return match policy {
            ConflictPolicy::RenameNew => Ok(rename()),
            _ => Err(src.error(ErrorCode::SameFile)),
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

/// Come `next_free_name`, anche sul remoto: lì un nome è libero se `stat` non lo trova (con la connessione
/// caduta lo sembrano tutti, ma la scrittura non sostituisce mai un file che non ha deciso di sovrascrivere).
pub fn free_name(vfs: &Vfs, dst: &Loc) -> Loc {
    if let Loc::Local(p) = dst {
        return Loc::Local(next_free_name(p));
    }
    free_name_by(dst, |c| vfs.remote(c, Purpose::Transfer).and_then(|(p, path)| p.stat(&path)).is_ok())
}

/// Come `free_name` per una posizione non locale, con `taken` a dire se un nome è occupato.
pub(crate) fn free_name_by(dst: &Loc, taken: impl Fn(&Loc) -> bool) -> Loc {
    let name = dst.file_name().unwrap_or_default();
    // Come `Path::file_stem`/`extension`: `.bashrc` non ha estensione.
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    let parent = dst.parent().unwrap_or_else(|| dst.clone());
    (2u64..)
        .map(|n| parent.join(&format!("{stem} ({n}){ext}")))
        .find(|c| !taken(c))
        .expect("intervallo infinito")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::vfs::mem::{self, MemProvider};
    use std::sync::Arc;

    /// I test locali restano sui percorsi: si avvolgono in `Loc::Local`.
    fn resolve(policy: &ConflictPolicy, src: &Path, dst: &Path, sm: &Meta, dm: &Meta) -> Result<Option<Decision>, AppError> {
        let (src, dst) = (Loc::Local(src.into()), Loc::Local(dst.into()));
        super::resolve(&mem::vfs(&[]), policy, &src, &dst, sm, dm)
    }

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
    fn zero_volume_serial_or_file_index_is_an_unknown_identity() {
        // Alcuni file system (rete, FAT, driver di terze parti) danno 0: si confrontano i percorsi.
        assert_eq!(known_id(0, 7), None);
        assert_eq!(known_id(7, 0), None);
        assert_eq!(known_id(1, 2), Some((1, 2)));
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
        assert_eq!(got, Some(Decision::Rename(Loc::Local(d.path().join("f (2).txt")))));
    }

    #[test]
    fn remote_conflicts_use_the_location() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/d/f.txt", b"");
        m.add_file("/d/f (2).txt", b"");
        m.add_file("/d/.bashrc", b"");
        let vfs = mem::vfs(&[("m", &m)]);
        let l = |s: &str| Loc::parse(s).unwrap();
        assert_eq!(super::free_name(&vfs, &l("sftp://m/d/f.txt")), l("sftp://m/d/f (3).txt"));
        assert_eq!(super::free_name(&vfs, &l("sftp://m/d/.bashrc")), l("sftp://m/d/.bashrc (2)"));
        let mm = Meta { size: 1, modified_ms: 1 };
        let same = super::resolve(&vfs, &ConflictPolicy::Overwrite, &l("sftp://m/d/f.txt"), &l("sftp://m/d/./f.txt"), &mm, &mm);
        assert_eq!(same.unwrap_err().code, ErrorCode::SameFile);
        // POSIX distingue le maiuscole: due file diversi.
        let other = super::resolve(&vfs, &ConflictPolicy::Overwrite, &l("sftp://m/d/F.txt"), &l("sftp://m/d/f.txt"), &mm, &mm);
        assert_eq!(other.unwrap(), Some(Decision::Overwrite));
    }
}
