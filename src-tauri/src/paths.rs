//! Utilità sui percorsi Windows: prefisso esteso, contenimento, chiave di volume.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf, Prefix};

/// Converte un percorso assoluto in forma estesa (`\\?\`) per superare il limite di 260 caratteri
/// e accettare nomi come "a." o "con spazio ". I percorsi relativi o già estesi restano invariati.
pub fn to_extended(p: &Path) -> PathBuf {
    let mut comps = p.components();
    let head = match comps.next() {
        Some(Component::Prefix(pc)) => pc,
        _ => return p.to_path_buf(),
    };
    let base: OsString = match head.kind() {
        Prefix::Disk(d) if p.has_root() => format!(r"\\?\{}:", d as char).into(),
        Prefix::UNC(server, share) => {
            let mut s = OsString::from(r"\\?\UNC\");
            s.push(server);
            s.push(r"\");
            s.push(share);
            s
        }
        _ => return p.to_path_buf(),
    };
    let mut out = PathBuf::from(base);
    // push di RootDir sostituisce la radice mantenendo il prefisso; i separatori `/` sono normalizzati.
    out.extend(comps);
    out
}

fn lowercase_components(p: &Path) -> Vec<String> {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// `true` se `child` coincide con `parent` o è al suo interno. Confronto per componenti,
/// senza distinguere maiuscole e minuscole.
pub fn is_within(child: &Path, parent: &Path) -> bool {
    let (c, p) = (lowercase_components(child), lowercase_components(parent));
    c.starts_with(&p)
}

/// Forma confrontabile di un percorso: assoluto, senza prefisso `\\?\` (`\\?\UNC\` diventa `\\`),
/// con `.` e `..` risolti a livello testuale e senza separatori finali. Non tocca il disco
/// (salvo `std::path::absolute` per i percorsi relativi) e non risolve i collegamenti.
pub fn normalize_lexical(p: &Path) -> PathBuf {
    let abs;
    let p = if p.is_absolute() {
        p
    } else {
        abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        &abs
    };
    let mut head = OsString::new();
    let mut rooted = false;
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for c in p.components() {
        match c {
            Component::Prefix(pc) => {
                head = match pc.kind() {
                    Prefix::VerbatimDisk(d) => format!("{}:", d as char).into(),
                    Prefix::VerbatimUNC(server, share) => {
                        let mut s = OsString::from(r"\\");
                        s.push(server);
                        s.push(r"\");
                        s.push(share);
                        s
                    }
                    _ => pc.as_os_str().to_os_string(),
                }
            }
            Component::RootDir => rooted = true,
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(n) => parts.push(n),
        }
    }
    let mut out = head;
    if rooted {
        out.push(r"\");
    }
    for (i, n) in parts.iter().enumerate() {
        if i > 0 {
            out.push(r"\");
        }
        out.push(n);
    }
    PathBuf::from(out)
}

/// Chiave del volume (`C:` oppure `\\SERVER\SHARE`) in maiuscolo; stringa vuota senza prefisso.
pub fn volume_key(p: &Path) -> String {
    match p.components().next() {
        Some(Component::Prefix(pc)) => match pc.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                format!("{}:", (d as char).to_ascii_uppercase())
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                r"\\{}\{}",
                server.to_string_lossy().to_uppercase(),
                share.to_string_lossy().to_uppercase()
            ),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_paths() {
        assert_eq!(to_extended(Path::new(r"C:\a\b")), PathBuf::from(r"\\?\C:\a\b"));
        assert_eq!(to_extended(Path::new(r"\\srv\sh\a")), PathBuf::from(r"\\?\UNC\srv\sh\a"));
        assert_eq!(to_extended(Path::new(r"\\?\C:\a")), PathBuf::from(r"\\?\C:\a"));
        assert_eq!(to_extended(Path::new(r"a\b")), PathBuf::from(r"a\b"));
    }

    #[test]
    fn within_is_case_insensitive_and_component_based() {
        assert!(is_within(Path::new(r"D:\A\b"), Path::new(r"d:\a")));
        assert!(is_within(Path::new(r"D:\a"), Path::new(r"D:\a")));
        assert!(!is_within(Path::new(r"D:\ab"), Path::new(r"D:\a")));
    }

    #[test]
    fn lexical_normalization() {
        let n = |s: &str| normalize_lexical(Path::new(s));
        assert_eq!(n(r"\\?\D:\a\b\"), PathBuf::from(r"D:\a\b"));
        assert_eq!(n(r"\\?\UNC\srv\sh\x"), PathBuf::from(r"\\srv\sh\x"));
        assert_eq!(n(r"D:\a\..\b"), PathBuf::from(r"D:\b"));
        assert_eq!(n(r"D:\a\.\b\\"), PathBuf::from(r"D:\a\b"));
        assert_eq!(n(r"D:\..\.."), PathBuf::from(r"D:\"));
        assert_eq!(n(r"\\srv\sh\a\"), PathBuf::from(r"\\srv\sh\a"));
        // Forme miste: il confronto di contenimento torna affidabile dopo la normalizzazione.
        assert!(is_within(&n(r"D:\a\b"), &n(r"\\?\D:\a\")));
        assert!(!is_within(&n(r"D:\a\..\b"), &n(r"D:\a")));
        assert!(n(r"rel\x").is_absolute());
    }

    #[test]
    fn volume_keys() {
        assert_eq!(volume_key(Path::new(r"c:\x")), "C:");
        assert_eq!(volume_key(Path::new(r"\\Srv\Share\x")), r"\\SRV\SHARE");
        assert_eq!(volume_key(Path::new(r"rel\x")), "");
    }

    #[test]
    fn long_path_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = dir.path().to_path_buf();
        for _ in 0..30 {
            p.push("cartella_lunga"); // > 260 caratteri
        }
        std::fs::create_dir_all(to_extended(&p)).unwrap();
        let f = to_extended(&p.join("nome con punto finale."));
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"x");
    }
}
