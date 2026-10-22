//! Posizione di un elemento: percorso Windows oppure `<schema>://<conn>/<percorso POSIX>`.

use crate::error::{AppError, ErrorCode};
use crate::paths;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Sftp,
    Ftp,
    Ftps,
    Http,
    Https,
}

impl Scheme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }

    fn lookup(s: &str) -> Option<Scheme> {
        [Scheme::Sftp, Scheme::Ftp, Scheme::Ftps, Scheme::Http, Scheme::Https]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// Posizione: percorso locale oppure `<schema>://<conn>/<percorso POSIX>` su una connessione salvata.
/// Il percorso remoto è letterale UTF-8, sempre con `/` iniziale, senza `/` doppi né finali (salvo la radice).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Loc {
    Local(PathBuf),
    Remote { scheme: Scheme, conn: String, path: String },
}

fn invalid(s: &str) -> AppError {
    AppError { code: ErrorCode::InvalidRequest, path: Some(s.to_owned()), detail: None }
}

/// Riduce a un percorso POSIX assoluto: `/` doppi ridotti, nessun `/` finale.
fn clean(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    format!("/{}", parts.join("/"))
}

impl Loc {
    pub fn parse(s: &str) -> Result<Loc, AppError> {
        // Un prefisso `xx://` con almeno due caratteri alfanumerici è uno schema; `C://x` resta locale.
        let Some((head, rest)) = s.split_once("://") else { return Ok(Loc::Local(PathBuf::from(s))) };
        if head.len() < 2 || !head.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return Ok(Loc::Local(PathBuf::from(s)));
        }
        let scheme = Scheme::lookup(&head.to_ascii_lowercase()).ok_or_else(|| invalid(s))?;
        let (conn, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let valid = (1..=64).contains(&conn.len())
            && conn.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return Err(invalid(s));
        }
        Ok(Loc::Remote { scheme, conn: conn.to_owned(), path: clean(path) })
    }

    pub fn join(&self, name: &str) -> Loc {
        match self {
            Loc::Local(p) => Loc::Local(p.join(name)),
            Loc::Remote { scheme, conn, path } => Loc::Remote {
                scheme: *scheme,
                conn: conn.clone(),
                path: clean(&format!("{path}/{name}")),
            },
        }
    }

    pub fn parent(&self) -> Option<Loc> {
        match self {
            Loc::Local(p) => p.parent().map(|p| Loc::Local(p.to_path_buf())),
            Loc::Remote { scheme, conn, path } => {
                let i = path.rfind('/').filter(|_| path != "/")?;
                let path = if i == 0 { "/" } else { &path[..i] };
                Some(Loc::Remote { scheme: *scheme, conn: conn.clone(), path: path.to_owned() })
            }
        }
    }

    pub fn file_name(&self) -> Option<String> {
        match self {
            Loc::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
            Loc::Remote { path, .. } => path.rsplit('/').next().filter(|n| !n.is_empty()).map(str::to_owned),
        }
    }

    /// `true` se `self` coincide con `parent` o è al suo interno. Remoto: confronto esatto per componenti.
    pub fn is_within(&self, parent: &Loc) -> bool {
        match (self, parent) {
            (Loc::Local(a), Loc::Local(b)) => {
                paths::is_within(&paths::normalize_lexical(a), &paths::normalize_lexical(b))
            }
            (
                Loc::Remote { scheme: s1, conn: c1, path: a },
                Loc::Remote { scheme: s2, conn: c2, path: b },
            ) => {
                s1 == s2
                    && c1 == c2
                    && (a == b || b == "/" || a.strip_prefix(b.as_str()).is_some_and(|r| r.starts_with('/')))
            }
            _ => false,
        }
    }

    pub fn same(&self, other: &Loc) -> bool {
        match (self, other) {
            (Loc::Local(a), Loc::Local(b)) => {
                paths::same_path(&paths::normalize_lexical(a), &paths::normalize_lexical(b))
            }
            _ => self == other,
        }
    }

    /// Chiave del "volume": quello del percorso per il locale, `<schema>://<conn>` per il remoto.
    pub fn volume_key(&self) -> String {
        match self {
            Loc::Local(p) => paths::volume_key(p),
            Loc::Remote { scheme, conn, .. } => format!("{}://{conn}", scheme.as_str()),
        }
    }

    pub fn as_local(&self) -> Option<&Path> {
        match self {
            Loc::Local(p) => Some(p),
            Loc::Remote { .. } => None,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Loc::Remote { .. })
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Loc::Local(p) => write!(f, "{}", p.to_string_lossy()),
            Loc::Remote { scheme, conn, path } => write!(f, "{}://{conn}{path}", scheme.as_str()),
        }
    }
}

impl Serialize for Loc {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Loc {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Loc, D::Error> {
        let s = String::deserialize(d)?;
        Loc::parse(&s).map_err(|e| serde::de::Error::custom(format!("posizione non valida: {:?}", e.path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(s: &str) -> Loc {
        Loc::parse(s).unwrap()
    }

    #[test]
    fn parse_local_and_remote() {
        assert_eq!(Loc::parse(r"C:\a").unwrap(), Loc::Local(PathBuf::from(r"C:\a")));
        assert_eq!(
            Loc::parse("sftp://web1/var/log/").unwrap(),
            Loc::Remote { scheme: Scheme::Sftp, conn: "web1".into(), path: "/var/log".into() }
        );
        assert_eq!(
            Loc::parse("sftp://web1//var//log").unwrap(),
            Loc::Remote { scheme: Scheme::Sftp, conn: "web1".into(), path: "/var/log".into() }
        );
        assert_eq!(
            Loc::parse("ftp://x").unwrap(),
            Loc::Remote { scheme: Scheme::Ftp, conn: "x".into(), path: "/".into() }
        );
        for bad in ["gopher://x/", "sftp://a b/", "sftp:///a", "sftp://a.b/"] {
            assert_eq!(Loc::parse(bad).unwrap_err().code, ErrorCode::InvalidRequest, "{bad}");
        }
    }

    #[test]
    fn join_parent_name() {
        assert_eq!(remote("sftp://w/var").join("log").to_string(), "sftp://w/var/log");
        assert_eq!(remote("sftp://w/").join("var").to_string(), "sftp://w/var");
        assert_eq!(remote("sftp://w/var/log").parent().unwrap().to_string(), "sftp://w/var");
        assert_eq!(remote("sftp://w/var").parent().unwrap().to_string(), "sftp://w/");
        assert_eq!(remote("sftp://w/").parent(), None);
        assert_eq!(remote("sftp://w/var/log").file_name().as_deref(), Some("log"));
        assert_eq!(remote("sftp://w/").file_name(), None);
        let l = Loc::parse(r"C:\a\b").unwrap();
        assert_eq!(l.join("c"), Loc::Local(PathBuf::from(r"C:\a\b").join("c")));
        assert_eq!(l.parent().unwrap(), Loc::Local(PathBuf::from(r"C:\a")));
        assert_eq!(l.file_name().as_deref(), Some("b"));
    }

    #[test]
    fn loc_roundtrip_keeps_unusual_names() {
        let s = "sftp://w/var/my app/è%20#1.log";
        let l = remote(s);
        assert_eq!(l.to_string(), s);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Loc>(&json).unwrap(), l);
        assert!(serde_json::from_str::<Loc>("\"gopher://x/\"").is_err());
    }

    #[test]
    fn volume_key_per_connection() {
        assert_eq!(remote("sftp://w/a").volume_key(), remote("sftp://w/b").volume_key());
        assert_ne!(remote("sftp://w/a").volume_key(), remote("sftp://v/a").volume_key());
        assert_ne!(remote("sftp://w/a").volume_key(), remote("ftp://w/a").volume_key());
        assert_eq!(Loc::parse(r"c:\x").unwrap().volume_key(), paths::volume_key(Path::new(r"c:\x")));
    }

    #[test]
    fn remote_never_local() {
        assert_eq!(remote("sftp://w/C:/x").as_local(), None);
        assert!(remote("sftp://w/C:/x").is_remote());
        assert!(Loc::parse(r"C:\x").unwrap().as_local().is_some());
    }

    #[test]
    fn same_and_within() {
        assert!(remote("sftp://w/a/b").is_within(&remote("sftp://w/a")));
        assert!(remote("sftp://w/a").is_within(&remote("sftp://w/a")));
        assert!(remote("sftp://w/a").is_within(&remote("sftp://w/")));
        assert!(!remote("sftp://w/ab").is_within(&remote("sftp://w/a")));
        assert!(!remote("sftp://w/a").is_within(&remote("sftp://v/a")));
        assert!(!remote("sftp://w/A").same(&remote("sftp://w/a"))); // POSIX distingue le maiuscole
        assert!(remote("sftp://w/a").same(&remote("sftp://w/a/")));
        let (a, b) = (Loc::parse(r"D:\A\b").unwrap(), Loc::parse(r"d:\a").unwrap());
        assert!(a.is_within(&b) && !a.same(&b));
        assert!(!a.is_within(&remote("sftp://w/")) && !remote("sftp://w/").same(&a));
    }
}
