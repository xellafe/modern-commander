//! Posizione di un elemento: percorso Windows, `<schema>://<conn>/<percorso POSIX>` oppure
//! `arc://<contenitore>/<percorso POSIX>` dentro un archivio.

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

/// Posizione: percorso locale, `<schema>://<conn>/<percorso POSIX>` su una connessione salvata oppure
/// `arc://<contenitore codificato>/<percorso POSIX>` dentro un archivio. Il percorso remoto è letterale UTF-8,
/// sempre con `/` iniziale, senza `/` doppi né finali (salvo la radice).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Loc {
    Local(PathBuf),
    Remote { scheme: Scheme, conn: String, path: String },
    /// Dentro un archivio: `container` è `Local` o `Remote` (mai `Archive`) e ha un nome di file; `path` è POSIX pulito.
    Archive { container: Box<Loc>, path: String },
}

/// Senza eco dell'input: un URL incollato potrebbe contenere `utente:password@`.
fn invalid() -> AppError {
    AppError { code: ErrorCode::InvalidRequest, path: None, detail: None }
}

/// Riduce a un percorso POSIX assoluto: `/` doppi ridotti, nessun `/` finale, `.` tolti e `..`
/// risolti a livello testuale (non oltre la radice).
fn clean(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            n => parts.push(n),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Parent di un percorso POSIX pulito; `None` per la radice.
fn posix_parent(path: &str) -> Option<&str> {
    let i = path.rfind('/').filter(|_| path != "/")?;
    Some(if i == 0 { "/" } else { &path[..i] })
}

/// `a` coincide con `b` o è al suo interno, per componenti.
fn posix_within(a: &str, b: &str) -> bool {
    a == b || b == "/" || a.strip_prefix(b).is_some_and(|r| r.starts_with('/'))
}

/// Codifica del contenitore dentro `arc://`: solo `%` e `/` (il resto è già leggibile e non ambiguo).
fn encode_container(c: &Loc) -> String {
    c.to_string().replace('%', "%25").replace('/', "%2F")
}

/// Inverso di `encode_container`: accetta solo `%25` e `%2F`/`%2f`, ogni altro `%` è un errore.
fn decode_container(s: &str) -> Result<String, AppError> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let esc = rest.get(i + 1..i + 3).ok_or_else(invalid)?;
        out.push(match esc {
            "25" => '%',
            "2F" | "2f" => '/',
            _ => return Err(invalid()),
        });
        rest = &rest[i + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

impl Loc {
    /// Posizione `path` dentro l'archivio `container` (locale o remoto, mai un altro archivio).
    pub fn archive(container: Loc, path: &str) -> Loc {
        debug_assert!(!matches!(container, Loc::Archive { .. }));
        Loc::Archive { container: Box::new(container), path: clean(path) }
    }

    pub fn archive_container(&self) -> Option<&Loc> {
        match self {
            Loc::Archive { container, .. } => Some(container),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Result<Loc, AppError> {
        // Un prefisso `xx://` con almeno due caratteri alfanumerici è uno schema; `C://x` resta locale.
        let Some((head, rest)) = s.split_once("://") else { return Ok(Loc::Local(PathBuf::from(s))) };
        if head.len() < 2 || !head.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return Ok(Loc::Local(PathBuf::from(s)));
        }
        if head.eq_ignore_ascii_case("arc") {
            let (enc, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            let container = Loc::parse(&decode_container(enc)?)?;
            if matches!(container, Loc::Archive { .. }) || container.file_name().is_none() {
                return Err(invalid());
            }
            return Ok(Loc::archive(container, path));
        }
        let scheme = Scheme::lookup(&head.to_ascii_lowercase()).ok_or_else(invalid)?;
        let (conn, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let valid = (1..=64).contains(&conn.len())
            && conn.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return Err(invalid());
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
            Loc::Archive { container, path } => Loc::archive((**container).clone(), &format!("{path}/{name}")),
        }
    }

    pub fn parent(&self) -> Option<Loc> {
        match self {
            Loc::Local(p) => p.parent().map(|p| Loc::Local(p.to_path_buf())),
            Loc::Remote { scheme, conn, path } => {
                let path = posix_parent(path)?;
                Some(Loc::Remote { scheme: *scheme, conn: conn.clone(), path: path.to_owned() })
            }
            Loc::Archive { container, path } => Some(Loc::archive((**container).clone(), posix_parent(path)?)),
        }
    }

    pub fn file_name(&self) -> Option<String> {
        match self {
            Loc::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
            Loc::Remote { path, .. } | Loc::Archive { path, .. } => path.rsplit('/').next().filter(|n| !n.is_empty()).map(str::to_owned),
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
                s1 == s2 && c1 == c2 && posix_within(a, b)
            }
            (Loc::Archive { container: c1, path: a }, Loc::Archive { container: c2, path: b }) => {
                c1.same(c2) && posix_within(a, b)
            }
            _ => false,
        }
    }

    pub fn same(&self, other: &Loc) -> bool {
        match (self, other) {
            (Loc::Local(a), Loc::Local(b)) => {
                paths::same_path(&paths::normalize_lexical(a), &paths::normalize_lexical(b))
            }
            (Loc::Archive { container: c1, path: a }, Loc::Archive { container: c2, path: b }) => c1.same(c2) && a == b,
            _ => self == other,
        }
    }

    /// Chiave del "volume": quello del percorso per il locale, `<schema>://<conn>` per il remoto.
    pub fn volume_key(&self) -> String {
        match self {
            Loc::Local(p) => paths::volume_key(p),
            Loc::Remote { scheme, conn, .. } => format!("{}://{conn}", scheme.as_str()),
            // Il file contenitore, non il suo volume: due archivi sullo stesso disco hanno chiavi diverse, due
            // forme dello stesso file (`C:\A.ZIP`, `c:\a.zip`) la stessa.
            Loc::Archive { container, .. } => match &**container {
                Loc::Local(p) => format!("arc://{}", paths::normalize_lexical(p).to_string_lossy().to_lowercase()),
                c => format!("arc://{c}"),
            },
        }
    }

    pub fn as_local(&self) -> Option<&Path> {
        match self {
            Loc::Local(p) => Some(p),
            Loc::Remote { .. } | Loc::Archive { .. } => None,
        }
    }

    /// Fuori dal file system locale (remoto o dentro un archivio).
    pub fn is_remote(&self) -> bool {
        matches!(self, Loc::Remote { .. } | Loc::Archive { .. })
    }

    /// Errore su questa posizione: `path` è la sua forma testuale.
    pub fn error(&self, code: ErrorCode) -> AppError {
        AppError::new(code, Some(Path::new(&self.to_string())))
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Loc::Local(p) => write!(f, "{}", p.to_string_lossy()),
            Loc::Remote { scheme, conn, path } => write!(f, "{}://{conn}{path}", scheme.as_str()),
            Loc::Archive { container, path } => write!(f, "arc://{}{path}", encode_container(container)),
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
        Loc::parse(&s).map_err(|_| serde::de::Error::custom("posizione non valida"))
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

    #[test]
    fn parse_edge_cases() {
        let l = remote("SFTP://w/x");
        assert!(matches!(l, Loc::Remote { scheme: Scheme::Sftp, .. }));
        assert_eq!(l.to_string(), "sftp://w/x");
        assert_eq!(Loc::parse("C://x").unwrap(), Loc::Local(PathBuf::from("C://x")));
        let ok = format!("sftp://{}/x", "a".repeat(64));
        assert!(Loc::parse(&ok).is_ok());
        let long = format!("sftp://{}/x", "a".repeat(65));
        assert_eq!(Loc::parse(&long).unwrap_err().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn dot_segments_are_resolved() {
        assert_eq!(remote("sftp://w/a/./b").to_string(), "sftp://w/a/b");
        assert_eq!(remote("sftp://w/a/../b").to_string(), "sftp://w/b");
        assert_eq!(remote("sftp://w/../..").to_string(), "sftp://w/");
        assert_eq!(remote("sftp://w/a").join("..").to_string(), "sftp://w/");
        assert!(remote("sftp://w/a/../a").same(&remote("sftp://w/a")));
        assert!(!remote("sftp://w/a/../b").is_within(&remote("sftp://w/a")));
    }

    #[test]
    fn errors_do_not_echo_input() {
        let e = Loc::parse("sftp://u:pw@h/x").unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(!format!("{e:?}").contains("pw"));
        let e = serde_json::from_str::<Loc>("\"sftp://u:pw@h/x\"").unwrap_err();
        assert!(!e.to_string().contains("pw"));
    }

    #[test]
    fn archive_roundtrip_local_and_remote_container() {
        let l = Loc::parse(r"arc://C:\rel\app.zip/conf/a b%#.xml").unwrap();
        assert_eq!(l, Loc::Archive { container: Box::new(Loc::Local(r"C:\rel\app.zip".into())), path: "/conf/a b%#.xml".into() });
        assert_eq!(l.to_string(), r"arc://C:\rel\app.zip/conf/a b%#.xml");
        let r = Loc::archive(Loc::parse("sftp://w/rel/100%/app.zip").unwrap(), "/");
        assert_eq!(r.to_string(), "arc://sftp:%2F%2Fw%2Frel%2F100%25%2Fapp.zip/");
        assert_eq!(Loc::parse(&r.to_string()).unwrap(), r);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Loc>(&json).unwrap(), r);
    }

    #[test]
    fn archive_parse_rejects_nesting_and_bad_escapes() {
        let nested = format!("arc://{}/x", Loc::parse(r"arc://C:\a.zip/b.zip").unwrap().to_string().replace('%', "%25").replace('/', "%2F"));
        for bad in [nested.as_str(), "arc://C:\\a.zip%41/x", "arc:///x", "arc://sftp:%2F%2Fw%2F/x"] {
            assert_eq!(Loc::parse(bad).unwrap_err().code, ErrorCode::InvalidRequest, "{bad}");
        }
    }

    #[test]
    fn archive_navigation_and_keys() {
        let root = Loc::parse(r"arc://C:\a.zip/").unwrap();
        let f = root.join("d").join("..").join("e");
        assert_eq!(f.to_string(), r"arc://C:\a.zip/e");
        assert_eq!(f.parent().unwrap(), root);
        assert_eq!(root.parent(), None);
        assert_eq!(root.file_name(), None);
        assert!(f.is_within(&root) && !f.is_within(&Loc::parse(r"arc://C:\b.zip/").unwrap()));
        assert!(Loc::parse(r"arc://c:\A.ZIP/e").unwrap().same(&f));
        assert_eq!(Loc::parse(r"arc://c:\A.ZIP/").unwrap().volume_key(), root.volume_key());
        assert_ne!(root.volume_key(), Loc::parse(r"C:\a.zip").unwrap().volume_key());
        assert!(root.is_remote() && root.as_local().is_none());
    }
}
