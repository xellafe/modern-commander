//! Protocollo tra l'app e l'helper elevato: un messaggio JSON per riga sulla named pipe.
//! Il primo messaggio, dall'helper all'app, è `Hello` con il token; poi l'app manda una richiesta
//! alla volta e l'helper risponde con zero o più `Progress` (solo le copie) e un `Ok`/`Err` finale.
//! Durante una copia l'app può mandare `Cancel`.

use crate::error::{AppError, ErrorCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};
use std::path::{Component, Path, PathBuf, Prefix};

/// Lunghezza massima di un messaggio, `\n` compreso: una riga più lunga è un errore e chiude la
/// connessione.
pub const MAX_LINE: usize = 64 * 1024;

/// Solo primitive su file: nessuna richiesta esegue comandi o carica codice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Hello { token: String },
    Copy { src: PathBuf, dst: PathBuf, overwrite: bool, flush: bool },
    Move { src: PathBuf, dst: PathBuf, overwrite: bool },
    Delete { path: PathBuf, is_dir: bool, permanent: bool },
    Mkdir { path: PathBuf },
    Rename { src: PathBuf, dst: PathBuf },
    /// Interrompe la copia in corso; altrimenti non ha effetto.
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Progress { bytes: u64 },
    Ok,
    Err(AppError),
}

/// Tutti i percorsi della richiesta devono essere sicuri (vedi `is_safe`).
pub fn validate(req: &Request) -> Result<(), AppError> {
    let paths: Vec<&Path> = match req {
        Request::Copy { src, dst, .. } | Request::Move { src, dst, .. } | Request::Rename { src, dst } => vec![src, dst],
        Request::Delete { path, .. } | Request::Mkdir { path } => vec![path],
        Request::Hello { .. } | Request::Cancel => vec![],
    };
    match paths.into_iter().find(|p| !is_safe(p)) {
        Some(p) => Err(AppError::new(ErrorCode::InvalidRequest, Some(p))),
        None => Ok(()),
    }
}

/// Assoluto su unità (`C:\`) o UNC (`\\srv\share\`), anche in forma estesa `\\?\`; senza
/// componenti `.`/`..` (con entrambi i separatori) e senza NUL. Rifiuta i percorsi di dispositivo
/// (`\\.\`, `\\?\GLOBALROOT`, `\\?\Volume{…}`) e quelli relativi a un'unità (`C:x`).
fn is_safe(p: &Path) -> bool {
    let s = p.as_os_str().to_string_lossy();
    if s.is_empty() || s.contains('\0') || s.split(['\\', '/']).any(|seg| seg == "." || seg == "..") {
        return false;
    }
    let mut comps = p.components();
    let prefix_ok = matches!(comps.next(), Some(Component::Prefix(pc))
        if matches!(pc.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)));
    prefix_ok && comps.next() == Some(Component::RootDir) && comps.all(|c| matches!(c, Component::Normal(_)))
}

fn too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "message_too_long")
}

pub fn write_msg(w: &mut impl Write, msg: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    if line.len() > MAX_LINE {
        return Err(too_long());
    }
    w.write_all(&line)
}

/// `None` a fine stream. Una riga oltre `MAX_LINE`, troncata o non valida è un errore.
pub fn read_msg<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = Vec::new();
    r.by_ref().take(MAX_LINE as u64).read_until(b'\n', &mut line)?;
    match line.last() {
        None => Ok(None),
        Some(b'\n') => Ok(Some(serde_json::from_slice(&line)?)),
        Some(_) => Err(too_long()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, ErrorCode};
    use std::path::PathBuf;

    #[test]
    fn validate_rejects_unsafe_paths() {
        for p in [r"relative\x", r"C:\a\..\Windows", r"\\.\PhysicalDrive0", r"\\?\GLOBALROOT\x", ""] {
            assert_eq!(validate(&Request::Mkdir { path: p.into() }).unwrap_err().code, ErrorCode::InvalidRequest, "{p}");
        }
        assert!(validate(&Request::Mkdir { path: r"C:\Program Files\X".into() }).is_ok());
        assert!(validate(&Request::Mkdir { path: r"\\srv\share\x".into() }).is_ok());
    }

    #[test]
    fn messages_roundtrip_as_json_lines() {
        let p = |s: &str| PathBuf::from(s);
        let reqs = [
            Request::Hello { token: "ab".repeat(32) },
            Request::Copy { src: p(r"C:\a"), dst: p(r"D:\b"), overwrite: true, flush: true },
            Request::Move { src: p(r"C:\a"), dst: p(r"C:\b"), overwrite: false },
            Request::Delete { path: p(r"C:\a"), is_dir: true, permanent: false },
            Request::Mkdir { path: p(r"C:\Program Files\X") },
            Request::Rename { src: p(r"C:\a"), dst: p(r"C:\A") },
            Request::Cancel,
        ];
        let resps = [
            Response::Progress { bytes: 42 },
            Response::Ok,
            Response::Err(AppError { detail: Some("d".into()), ..AppError::new(ErrorCode::AccessDenied, Some(&p(r"C:\a"))) }),
        ];
        let mut buf = Vec::new();
        for r in &reqs {
            write_msg(&mut buf, r).unwrap();
        }
        for r in &resps {
            write_msg(&mut buf, r).unwrap();
        }
        assert_eq!(buf.iter().filter(|&&b| b == b'\n').count(), reqs.len() + resps.len());
        let mut rd = &buf[..];
        for r in &reqs {
            assert_eq!(read_msg::<Request>(&mut rd).unwrap().as_ref(), Some(r));
        }
        for r in &resps {
            assert_eq!(read_msg::<Response>(&mut rd).unwrap().as_ref(), Some(r));
        }
        assert_eq!(read_msg::<Request>(&mut rd).unwrap(), None); // fine dello stream
    }

    #[test]
    fn oversized_message_is_rejected() {
        let mut line = b"\"Cancel\"".to_vec();
        line.resize(MAX_LINE - 1, b' ');
        line.push(b'\n');
        assert_eq!(read_msg::<Request>(&mut &line[..]).unwrap(), Some(Request::Cancel)); // al limite: valida
        line.insert(0, b' ');
        assert!(read_msg::<Request>(&mut &line[..]).is_err());
        // Una riga troncata (senza `\n`) non è un messaggio.
        assert!(read_msg::<Request>(&mut &b"\"Cancel\""[..]).is_err());
        // Neanche in uscita.
        let path = PathBuf::from(format!(r"C:\{}", "a".repeat(MAX_LINE)));
        assert!(write_msg(&mut Vec::new(), &Request::Mkdir { path }).is_err());
    }
}
