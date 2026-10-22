//! Importazione dei siti salvati di WinSCP (registro o `WinSCP.ini`), senza password.

use super::connections::Connection;
use super::http_index::percent_decode;
use crate::error::{AppError, ErrorCode};
use crate::vfs::loc::Scheme;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_ROUTINE_FLAGS, REG_VALUE_TYPE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub name: String,
    pub scheme: Option<Scheme>,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub start_dir: String,
    pub ftps_implicit: bool,
    /// Codice del motivo per cui non si importa (`webdav`, `s3`, `nohost`, `unknown`); il testo lo dà il frontend.
    pub skipped: Option<String>,
}

/// Le uniche chiavi di un sito che si leggono: la password (e ogni altra) non si guarda nemmeno.
const KEYS: [&str; 6] = ["FSProtocol", "Ftps", "PortNumber", "HostName", "UserName", "RemoteDirectory"];

/// Sito come lo dà WinSCP: nome ancora percent-encoded e valori in testo, da `.ini` o dal registro.
fn build(raw_name: &str, get: impl Fn(&str) -> Option<String>) -> Option<ImportCandidate> {
    // I nomi non ASCII hanno davanti il BOM UTF-8 (`%EF%BB%BF`); la `/` separa le cartelle.
    let name = percent_decode(raw_name).trim_start_matches('\u{feff}').to_owned();
    if name == "Default Settings" {
        return None;
    }
    let num = |k: &str| get(k).and_then(|v| v.trim().parse::<u32>().ok());
    let mut host = get("HostName").unwrap_or_default().trim().to_owned();
    let mut user = get("UserName").unwrap_or_default();
    // WinSCP accetta `utente@host` nel solo nome host (`SetHostName` divide all'ultima `@`).
    if user.is_empty()
        && let Some((u, h)) = host.clone().rsplit_once('@')
    {
        user = u.to_owned();
        host = h.to_owned();
    }
    // FSProtocol: 0 SCP, 1 SFTP (con ripiego su SCP), 2 solo SFTP, 5 FTP, 6 WebDAV, 7 S3; senza valore SFTP.
    // Ftps: 0 nessuno, 1 implicito, 2 e 3 esplicito (SSL, TLS).
    let ftps = num("Ftps").unwrap_or(0);
    let (scheme, implicit, default_port, skipped) = match num("FSProtocol").unwrap_or(1) {
        0..=2 => (Some(Scheme::Sftp), false, 22, None),
        5 if ftps == 0 => (Some(Scheme::Ftp), false, 21, None),
        5 => (Some(Scheme::Ftps), ftps == 1, if ftps == 1 { 990 } else { 21 }, None),
        6 => (None, false, 0, Some("webdav")),
        7 => (None, false, 0, Some("s3")),
        _ => (None, false, 0, Some("unknown")),
    };
    let skipped = skipped.or(host.is_empty().then_some("nohost"));
    let port = num("PortNumber").and_then(|p| u16::try_from(p).ok()).filter(|&p| p > 0).unwrap_or(default_port);
    let dir = get("RemoteDirectory").unwrap_or_default();
    Some(ImportCandidate {
        name: name.replace('/', " / "),
        scheme,
        host,
        port,
        user,
        start_dir: if dir.is_empty() { "/".into() } else { dir },
        ftps_implicit: implicit,
        skipped: skipped.map(str::to_owned),
    })
}

/// I siti di un `WinSCP.ini`: sezioni `[Sessions\<nome>]`.
pub fn from_ini(text: &str) -> Vec<ImportCandidate> {
    const PREFIX: &str = "Sessions\\";
    let mut sites: Vec<(&str, HashMap<&'static str, String>)> = Vec::new();
    let mut open = false;
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = sec
                .get(PREFIX.len()..)
                .filter(|n| sec[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) && !n.contains('\\'));
            open = name.is_some();
            if let Some(n) = name {
                sites.push((n, HashMap::new()));
            }
        } else if open
            && let Some((k, v)) = line.split_once('=')
            && let Some(key) = KEYS.iter().find(|c| c.eq_ignore_ascii_case(k.trim()))
        {
            sites.last_mut().unwrap().1.insert(key, v.to_owned());
        }
    }
    sites.into_iter().filter_map(|(n, vals)| build(n, |k| vals.get(k).cloned())).collect()
}

/// Chiude la chiave del registro quando esce di scope.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: la chiave è stata aperta da `RegOpenKeyExW` e non è condivisa.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn reg_err(e: WIN32_ERROR) -> AppError {
    AppError { detail: Some(format!("registro: {}", e.0)), ..AppError::new(ErrorCode::Io, None) }
}

/// `None` se la chiave non esiste.
fn open_key(parent: HKEY, sub: &str) -> Result<Option<Key>, AppError> {
    let mut h = HKEY::default();
    // SAFETY: puntatori validi per la durata della chiamata.
    let e = unsafe { RegOpenKeyExW(parent, &HSTRING::from(sub), None, KEY_READ, &mut h) };
    match e {
        ERROR_SUCCESS => Ok(Some(Key(h))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(reg_err(e)),
    }
}

/// Valore testo o numerico (letto come testo) di un sito; mai chiamata per la password.
fn reg_value(key: &Key, name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    let flags = REG_ROUTINE_FLAGS(RRF_RT_REG_SZ.0 | RRF_RT_REG_DWORD.0);
    let (mut ty, mut size) = (REG_VALUE_TYPE(0), 0u32);
    let mut buf;
    // SAFETY: due chiamate, la prima per la dimensione; il buffer ha `size` byte.
    unsafe {
        if RegGetValueW(key.0, PCWSTR::null(), &name, flags, Some(&mut ty), None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        buf = vec![0u8; size as usize];
        let data = Some(buf.as_mut_ptr().cast());
        if RegGetValueW(key.0, PCWSTR::null(), &name, flags, Some(&mut ty), data, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
    }
    buf.truncate(size as usize);
    if ty == REG_DWORD {
        return Some(u32::from_le_bytes(buf.get(..4)?.try_into().ok()?).to_string());
    }
    let wide: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&wide).trim_end_matches('\0').to_owned())
}

/// I siti in `HKCU\Software\Martin Prikryl\WinSCP 2\Sessions`; senza la chiave, nessuno.
pub fn from_registry() -> Result<Vec<ImportCandidate>, AppError> {
    let Some(sessions) = open_key(HKEY_CURRENT_USER, "Software\\Martin Prikryl\\WinSCP 2\\Sessions")? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for i in 0.. {
        // 255 caratteri è il massimo di una chiave del registro.
        let (mut buf, mut len) = ([0u16; 256], 256u32);
        // SAFETY: `buf` ha `len` caratteri.
        let e = unsafe { RegEnumKeyExW(sessions.0, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        match e {
            ERROR_NO_MORE_ITEMS => break,
            ERROR_SUCCESS => {}
            _ => continue,
        }
        let raw = String::from_utf16_lossy(&buf[..len as usize]);
        if let Some(site) = open_key(sessions.0, &raw)? {
            out.extend(build(&raw, |k| reg_value(&site, k)));
        }
    }
    Ok(out)
}

/// `WinSCP.ini` come lo cerca WinSCP: accanto a `WinSCP.exe` (nelle cartelle di installazione solite), poi in
/// `%APPDATA%`; se non c'è, il registro. ponytail: un'installazione in un'altra cartella non si trova: si legge il
/// registro, che per un'installazione normale è dove stanno i siti.
pub fn from_system() -> Result<Vec<ImportCandidate>, AppError> {
    let var = |v: &str| std::env::var_os(v).map(PathBuf::from);
    let dirs = [
        var("ProgramFiles").map(|p| p.join("WinSCP")),
        var("ProgramFiles(x86)").map(|p| p.join("WinSCP")),
        var("LOCALAPPDATA").map(|p| p.join("Programs").join("WinSCP")),
        var("APPDATA"),
    ];
    for dir in dirs.into_iter().flatten() {
        if let Ok(bytes) = std::fs::read(dir.join("WinSCP.ini")) {
            return Ok(from_ini(&String::from_utf8_lossy(&bytes)));
        }
    }
    from_registry()
}

impl ImportCandidate {
    /// La connessione da salvare (senza id né password); `None` per un sito da saltare.
    pub fn into_connection(self) -> Option<Connection> {
        let scheme = self.scheme.filter(|_| self.skipped.is_none() && !self.host.is_empty())?;
        Some(Connection {
            id: String::new(),
            name: self.name,
            scheme,
            host: self.host,
            port: self.port,
            user: self.user,
            start_dir: self.start_dir,
            ftps_implicit: self.ftps_implicit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/winscp.ini");

    fn site(name: &str, scheme: Option<Scheme>, host: &str, port: u16, user: &str, dir: &str, implicit: bool) -> ImportCandidate {
        ImportCandidate {
            name: name.into(), scheme, host: host.into(), port, user: user.into(), start_dir: dir.into(),
            ftps_implicit: implicit, skipped: None,
        }
    }

    #[test]
    fn parses_winscp_ini() {
        let got = from_ini(FIXTURE);
        let skipped = |c: ImportCandidate, why: &str| ImportCandidate { skipped: Some(why.into()), ..c };
        let want = vec![
            site("Web Prod", Some(Scheme::Sftp), "sftp.example.org", 2222, "deploy", "/var/www", false),
            site("Backup SCP", Some(Scheme::Sftp), "scp.example.org", 22, "backup", "/", false),
            site("Legacy FTP", Some(Scheme::Ftps), "ftp.example.org", 21, "ftpuser", "/pub", false),
            site("Banca FTPS", Some(Scheme::Ftps), "ftps.example.org", 990, "bank", "/", true),
            site("Clienti / Acme", Some(Scheme::Sftp), "acme.example.org", 22, "acme", "/data/acme", false),
            site("Caffè Mario", Some(Scheme::Sftp), "caffe.example.org", 22, "mario", "/", false),
            skipped(site("Intranet DAV", None, "dav.example.org", 443, "dav", "/", false), "webdav"),
            skipped(site("Archivio S3", None, "s3.example.org", 0, "", "/", false), "s3"),
            skipped(site("Vuoto", Some(Scheme::Sftp), "", 22, "nessuno", "/", false), "nohost"),
        ];
        assert_eq!(got, want);
        // Le password non si leggono: non c'è un campo che le porti e il JSON non le contiene.
        let json = serde_json::to_string(&got).unwrap();
        assert!(!json.contains("A35C4D1F") && !json.contains("Segreto-Da-Non-Importare"));
    }
}
