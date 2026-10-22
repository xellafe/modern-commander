//! Autenticazione HTTP: Basic, oppure Digest (RFC 7616, MD5 e SHA-256 con gli hash BCrypt di `fs::hash`) se la
//! risposta `401` lo chiede in `WWW-Authenticate`. Il primo tentativo parte senza credenziali; scelto lo schema,
//! le richieste successive lo usano subito (Digest con lo stesso nonce e `nc` crescente).
//! ponytail: Basic su `http` semplice manda la password in chiaro (base64): è una scelta dell'utente, che configura
//! la connessione; nessun avviso dal backend. Niente `userhash` né `auth-int`: un server che li impone dà errore.

use crate::error::{AppError, ErrorCode};
use crate::fs::hash::{md5, sha256};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Algo {
    Md5,
    Sha256,
}

/// Una sfida Digest di `WWW-Authenticate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algo: Algo,
    /// `-sess`: HA1 comprende nonce e cnonce.
    pub sess: bool,
    /// `qop=auth`; senza, la risposta della RFC 2069.
    pub qop: bool,
}

fn h(algo: Algo, s: &str) -> Result<String, AppError> {
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02x}")).collect();
    Ok(match algo {
        Algo::Md5 => hex(&md5(s.as_bytes())?),
        Algo::Sha256 => hex(&sha256(s.as_bytes())?),
    })
}

/// Il valore `response` di una risposta Digest, in esadecimale minuscolo.
pub fn digest_response(
    ch: &Challenge,
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> Result<String, AppError> {
    let mut ha1 = h(ch.algo, &format!("{user}:{}:{password}", ch.realm))?;
    if ch.sess {
        ha1 = h(ch.algo, &format!("{ha1}:{}:{cnonce}", ch.nonce))?;
    }
    let ha2 = h(ch.algo, &format!("{method}:{uri}"))?;
    let data = match ch.qop {
        true => format!("{}:{nc:08x}:{cnonce}:auth:{ha2}", ch.nonce),
        false => format!("{}:{ha2}", ch.nonce),
    };
    h(ch.algo, &format!("{ha1}:{data}"))
}

type Params = Vec<(String, String)>;

/// Le sfide di un valore di `WWW-Authenticate`: schema in minuscolo e parametri (nomi in minuscolo). Un valore può
/// contenerne più d'una (`Digest realm="a", nonce="b", Basic realm="a"`). Testo del server: mai un panic.
fn parse_challenges(value: &str) -> Vec<(String, Params)> {
    let mut out: Vec<(String, Params)> = Vec::new();
    let mut it = value.chars().peekable();
    loop {
        while it.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        let mut token = String::new();
        while let Some(c) = it.next_if(|c| !c.is_whitespace() && !matches!(c, ',' | '=')) {
            token.push(c);
        }
        if token.is_empty() && it.peek().is_none() {
            return out;
        }
        while it.next_if(|c| c.is_whitespace()).is_some() {}
        if it.next_if_eq(&'=').is_none() {
            out.push((token.to_ascii_lowercase(), Vec::new()));
            continue;
        }
        while it.next_if(|c| c.is_whitespace()).is_some() {}
        let mut val = String::new();
        if it.next_if_eq(&'"').is_some() {
            while let Some(c) = it.next() {
                match c {
                    '"' => break,
                    '\\' => val.extend(it.next()),
                    c => val.push(c),
                }
            }
        } else {
            while let Some(c) = it.next_if(|c| *c != ',' && !c.is_whitespace()) {
                val.push(c);
            }
        }
        // Un parametro prima di ogni schema (o un token68 come `abc==`) resta senza sfida: si ignora.
        if let Some((_, params)) = out.last_mut() {
            params.push((token.to_ascii_lowercase(), val));
        }
    }
}

/// Lo schema da usare: Digest (SHA-256 prima di MD5) se il server lo offre in una forma che sappiamo calcolare,
/// altrimenti Basic. `stale`: la sfida Digest dice che le credenziali erano buone e il nonce è solo scaduto.
fn pick(values: &[&str]) -> Option<(State, bool)> {
    let mut basic = false;
    let mut best: Option<(Challenge, bool)> = None;
    for (scheme, params) in values.iter().flat_map(|v| parse_challenges(v)) {
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        match scheme.as_str() {
            "basic" => basic = true,
            "digest" => {
                let (algo, sess) = match get("algorithm").map(str::to_ascii_uppercase).as_deref() {
                    None | Some("MD5") => (Algo::Md5, false),
                    Some("MD5-SESS") => (Algo::Md5, true),
                    Some("SHA-256") => (Algo::Sha256, false),
                    Some("SHA-256-SESS") => (Algo::Sha256, true),
                    Some(_) => continue,
                };
                let qop = match get("qop") {
                    None => false,
                    Some(q) if q.split(',').any(|q| q.trim().eq_ignore_ascii_case("auth")) => true,
                    Some(_) => continue,
                };
                let Some(nonce) = get("nonce") else { continue };
                let stale = get("stale").is_some_and(|s| s.eq_ignore_ascii_case("true"));
                let ch = Challenge {
                    realm: get("realm").unwrap_or_default().to_owned(),
                    nonce: nonce.to_owned(),
                    opaque: get("opaque").map(str::to_owned),
                    algo,
                    sess,
                    qop,
                };
                if best.as_ref().is_none_or(|(b, _)| b.algo == Algo::Md5 && algo == Algo::Sha256) {
                    best = Some((ch, stale));
                }
            }
            _ => {}
        }
    }
    match best {
        Some((ch, stale)) => Some((State::Digest { ch, nc: 0 }, stale)),
        None => basic.then_some((State::Basic, false)),
    }
}

enum State {
    /// Nessuna sfida ancora: la richiesta parte senza credenziali.
    None,
    Basic,
    Digest { ch: Challenge, nc: u32 },
}

/// Utente e password di una connessione, con lo schema scelto. Niente `Debug`: contiene la password.
pub struct Credentials {
    user: String,
    password: String,
    state: Mutex<State>,
}

/// Valore tra virgolette di un parametro Digest.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Credentials {
    pub fn new(user: &str, password: &str) -> Self {
        Credentials { user: user.to_owned(), password: password.to_owned(), state: Mutex::new(State::None) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// L'intestazione `Authorization` per una richiesta (`uri`: il percorso codificato), se lo schema è già scelto.
    pub fn header(&self, method: &str, uri: &str) -> Result<Option<String>, AppError> {
        let mut s = self.state();
        Ok(match &mut *s {
            State::None => None,
            State::Basic => Some(format!("Basic {}", STANDARD.encode(format!("{}:{}", self.user, self.password)))),
            State::Digest { ch, nc } => {
                *nc = nc.wrapping_add(1);
                let mut b = [0u8; 16];
                getrandom::fill(&mut b).map_err(|_| AppError::new(ErrorCode::Io, None))?;
                let cnonce: String = b.iter().map(|b| format!("{b:02x}")).collect();
                let response = digest_response(ch, &self.user, &self.password, method, uri, *nc, &cnonce)?;
                let algo = match (ch.algo, ch.sess) {
                    (Algo::Md5, false) => "MD5",
                    (Algo::Md5, true) => "MD5-sess",
                    (Algo::Sha256, false) => "SHA-256",
                    (Algo::Sha256, true) => "SHA-256-sess",
                };
                let mut v = format!(
                    "Digest username={}, realm={}, uri={}, algorithm={algo}, nonce={}, response=\"{response}\"",
                    quoted(&self.user),
                    quoted(&ch.realm),
                    quoted(uri),
                    quoted(&ch.nonce),
                );
                if ch.qop {
                    v += &format!(", qop=auth, nc={:08x}, cnonce=\"{cnonce}\"", *nc);
                }
                if let Some(o) = &ch.opaque {
                    v += &format!(", opaque={}", quoted(o));
                }
                Some(v)
            }
        })
    }

    /// Una risposta `401` con le sue sfide. `Ok`: lo schema è aggiornato e la richiesta va ripetuta una volta.
    /// Credenziali già mandate e rifiutate (senza `stale`) danno `AUTH_FAILED`; uno schema che non sappiamo fare
    /// (NTLM, Negotiate, ...) `UNSUPPORTED`, con gli schemi offerti in `detail`.
    pub fn challenge(&self, values: &[&str], sent: bool) -> Result<(), AppError> {
        let Some((next, stale)) = pick(values) else {
            let offered: Vec<String> = values.iter().flat_map(|v| parse_challenges(v)).map(|(s, _)| s).collect();
            return Err(AppError { detail: Some(offered.join(", ")), ..AppError::new(ErrorCode::Unsupported, None) });
        };
        if sent && !stale {
            return Err(AppError::new(ErrorCode::AuthFailed, None));
        }
        *self.state() = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Il vettore d'esempio della RFC 7616, §3.9.1: stessa richiesta, con SHA-256 e con MD5.
    #[test]
    fn digest_rfc7616_vector() {
        let mut ch = Challenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
            opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
            algo: Algo::Sha256,
            sess: false,
            qop: true,
        };
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let r = |ch: &Challenge| digest_response(ch, "Mufasa", "Circle of Life", "GET", "/dir/index.html", 1, cnonce).unwrap();
        assert_eq!(r(&ch), "753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1");
        ch.algo = Algo::Md5;
        assert_eq!(r(&ch), "8ca523f5e9506fed4657c9700eebdbec");
    }

    /// Le due sfide della RFC 7616 §3.9.1 in un'intestazione sola, più Basic: vince SHA-256; il nonce scaduto
    /// (`stale`) ripete senza `AUTH_FAILED`; NTLM da solo non si sa fare.
    #[test]
    fn http_auth_challenges() {
        let both = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS", Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let c = Credentials::new("Mufasa", "Circle of Life");
        assert_eq!(c.header("GET", "/").unwrap(), None);
        c.challenge(&[both, r#"Basic realm="x""#], false).unwrap();
        let hdr = c.header("GET", "/dir/index.html").unwrap().unwrap();
        assert!(hdr.starts_with(r#"Digest username="Mufasa", realm="http-auth@example.org", uri="/dir/index.html", algorithm=SHA-256, "#), "{hdr}");
        assert!(hdr.contains("qop=auth, nc=00000001, cnonce=\"") && hdr.ends_with(r#"opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#));
        assert!(c.header("GET", "/").unwrap().unwrap().contains("nc=00000002"));
        // Credenziali mandate e rifiutate; poi lo stesso con `stale=true`, che invece si ripete.
        assert_eq!(c.challenge(&[both], true).unwrap_err().code, ErrorCode::AuthFailed);
        let stale = r#"Digest realm="r", nonce="n2", stale=TRUE"#;
        c.challenge(&[stale], true).unwrap();
        assert!(c.header("GET", "/").unwrap().unwrap().contains(r#"algorithm=MD5, nonce="n2""#));

        let b = Credentials::new("u", "p");
        b.challenge(&[r#"Basic realm="files", charset="UTF-8""#], false).unwrap();
        assert_eq!(b.header("GET", "/").unwrap().unwrap(), "Basic dTpw");
        let e = b.challenge(&["NTLM", "Negotiate"], false).unwrap_err();
        assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("ntlm, negotiate")));
        // Testo strano: niente panic.
        for v in ["", "=", "\"", "Digest", "Digest nonce", "Digest nonce=\"", "Digest nonce=\"a\\", ",,= =,", "abc=="] {
            let _ = parse_challenges(v);
            let _ = pick(&[v]);
        }
    }
}
