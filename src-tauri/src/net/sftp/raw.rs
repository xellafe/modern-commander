//! Canale SFTP grezzo di libssh2, accanto a quello di `ssh2`, per le due cose che `ssh2` 0.9 non dà:
//! `posix-rename@openssh.com` e la lettura delle cartelle con i nomi in byte (`ssh2` fa `from_utf8(..).unwrap()`).
//!
//! Sincronizzazione: ogni chiamata grezza tiene la guardia di `Session::raw()`, cioè il mutex della `SessionInner` di
//! `ssh2`, lo stesso che `ssh2` prende in ogni chiamata su `Session`, `Sftp` e `File`. Le chiamate grezze non si
//! sovrappongono mai a quelle di `ssh2`. Dentro la guardia non si chiama `ssh2` (il mutex non è rientrante).

use super::{map_session_error, map_sftp_error};
use crate::error::ErrorCode;
use libssh2_sys as sys;
use ssh2::{FileStat, Session};
use std::ffi::{CString, c_char, c_int, c_uint, c_ulong};
use std::ptr::null_mut;

unsafe extern "C" {
    /// In `libssh2_sftp.h` della libssh2 inclusa in `libssh2-sys` (1.11), che però non la dichiara.
    fn libssh2_sftp_posix_rename_ex(
        sftp: *mut sys::LIBSSH2_SFTP,
        source_filename: *const c_char,
        source_filename_len: usize,
        dest_filename: *const c_char,
        dest_filename_len: usize,
    ) -> c_int;
}

/// libssh2 risponde con questo valore (positivo) quando il server non ha dichiarato `posix-rename`, senza inviare
/// nulla; lo stesso codice arriva come stato SFTP se è il server a rifiutare.
const FX_OP_UNSUPPORTED: c_int = sys::LIBSSH2_FX_OP_UNSUPPORTED;
const ERROR_SFTP_PROTOCOL: c_int = sys::LIBSSH2_ERROR_SFTP_PROTOCOL;
const ERROR_BUFFER_TOO_SMALL: c_int = sys::LIBSSH2_ERROR_BUFFER_TOO_SMALL;
/// Oltre questa lunghezza un nome non è un nome: risposta malformata.
const MAX_NAME: usize = 64 * 1024;

/// Il codice di un risultato negativo (o di `FX_OP_UNSUPPORTED`): `sftp_errno` conta solo con `ERROR_SFTP_PROTOCOL`.
pub(super) fn code(rc: c_int, sftp_errno: c_ulong) -> ErrorCode {
    match rc {
        FX_OP_UNSUPPORTED => ErrorCode::Unsupported,
        ERROR_SFTP_PROTOCOL => map_sftp_error(sftp_errno as i32),
        _ => map_session_error(rc),
    }
}

fn cstr(path: &str) -> Result<CString, ErrorCode> {
    CString::new(path).map_err(|_| ErrorCode::InvalidName)
}

pub(super) struct RawSftp {
    sess: Session,
    sftp: *mut sys::LIBSSH2_SFTP,
}

// SAFETY: il puntatore si usa solo con il mutex della sessione preso (vedi il commento del modulo); libssh2 non lega
// una sessione a un thread.
unsafe impl Send for RawSftp {}
unsafe impl Sync for RawSftp {}

impl RawSftp {
    /// Apre il canale su una sessione già autenticata (in modalità bloccante, quella di `ssh2` di default).
    pub(super) fn new(sess: &Session) -> Result<Self, ErrorCode> {
        let mut s = sess.raw();
        // SAFETY: `s` è la sessione viva, con il lock preso.
        let sftp = unsafe { sys::libssh2_sftp_init(&mut *s) };
        if sftp.is_null() {
            // SAFETY: come sopra.
            return Err(map_session_error(unsafe { sys::libssh2_session_last_errno(&mut *s) }));
        }
        Ok(Self { sess: sess.clone(), sftp })
    }

    /// Ultimo stato SFTP del canale. Da chiamare con il lock preso.
    fn errno(&self) -> c_ulong {
        // SAFETY: canale aperto; chi chiama tiene il lock.
        unsafe { sys::libssh2_sftp_last_error(self.sftp) }
    }

    /// Rinomina sostituendo `to` in un colpo solo. Senza l'estensione: `UNSUPPORTED`, senza inviare nulla.
    pub(super) fn posix_rename(&self, from: &str, to: &str) -> Result<(), ErrorCode> {
        let (f, t) = (cstr(from)?, cstr(to)?);
        let _s = self.sess.raw();
        // SAFETY: lock preso; `f` e `t` vivono per tutta la chiamata, lunghezze esplicite senza lo zero finale.
        let rc = unsafe {
            libssh2_sftp_posix_rename_ex(self.sftp, f.as_ptr(), f.as_bytes().len(), t.as_ptr(), t.as_bytes().len())
        };
        if rc == 0 { Ok(()) } else { Err(code(rc, self.errno())) }
    }

    /// Le voci di `path` senza `.` e `..`: nome in byte e attributi (come `lstat`). L'handle si chiude sempre.
    pub(super) fn readdir(&self, path: &str) -> Result<Vec<(Vec<u8>, FileStat)>, ErrorCode> {
        let p = cstr(path)?;
        let mut s = self.sess.raw();
        // SAFETY: lock preso; `p` vive per tutta la chiamata.
        let h = unsafe {
            sys::libssh2_sftp_open_ex(self.sftp, p.as_ptr(), p.as_bytes().len() as c_uint, 0, 0, sys::LIBSSH2_SFTP_OPENDIR)
        };
        if h.is_null() {
            // SAFETY: lock preso.
            let rc = unsafe { sys::libssh2_session_last_errno(&mut *s) };
            return Err(code(rc, self.errno()));
        }
        let r = self.entries(h);
        // SAFETY: `h` aperto sopra e chiuso una volta sola, con il lock ancora preso.
        unsafe { sys::libssh2_sftp_close_handle(h) };
        r
    }

    /// Da chiamare con il lock preso e `h` aperto.
    fn entries(&self, h: *mut sys::LIBSSH2_SFTP_HANDLE) -> Result<Vec<(Vec<u8>, FileStat)>, ErrorCode> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 1024];
        loop {
            let mut attrs =
                sys::LIBSSH2_SFTP_ATTRIBUTES { flags: 0, filesize: 0, uid: 0, gid: 0, permissions: 0, atime: 0, mtime: 0 };
            // SAFETY: `buf` è scrivibile per `buf.len()` byte (libssh2 vi scrive il nome e lo zero finale, e dà
            // `ERROR_BUFFER_TOO_SMALL` se non ci stanno, senza consumare la voce); niente `longentry`.
            let rc = unsafe {
                sys::libssh2_sftp_readdir_ex(h, buf.as_mut_ptr().cast(), buf.len(), null_mut(), 0, &mut attrs)
            };
            match rc {
                0 => return Ok(out),
                ERROR_BUFFER_TOO_SMALL if buf.len() < MAX_NAME => buf.resize(buf.len() * 2, 0),
                n if n > 0 => {
                    let name = &buf[..n as usize];
                    if name != b"." && name != b".." {
                        out.push((name.to_vec(), FileStat::from_raw(&attrs)));
                    }
                }
                n => return Err(code(n, self.errno())),
            }
        }
    }
}

impl Drop for RawSftp {
    /// Il canale si chiude prima della sessione, che `self.sess` tiene viva.
    fn drop(&mut self) {
        let _s = self.sess.raw();
        // SAFETY: canale aperto in `new`, nessun handle rimasto aperto (`readdir` chiude il suo), lock preso.
        unsafe { sys::libssh2_sftp_shutdown(self.sftp) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_status_codes() {
        // Estensione assente: libssh2 risponde 8 senza inviare nulla; il server può dare lo stesso stato.
        assert_eq!(code(FX_OP_UNSUPPORTED, 0), ErrorCode::Unsupported);
        assert_eq!(code(ERROR_SFTP_PROTOCOL, 8), ErrorCode::Unsupported);
        assert_eq!(code(ERROR_SFTP_PROTOCOL, 2), ErrorCode::NotFound);
        assert_eq!(code(ERROR_SFTP_PROTOCOL, 4), ErrorCode::Io);
        assert_eq!(code(sys::LIBSSH2_ERROR_TIMEOUT, 0), ErrorCode::Timeout);
        assert_eq!(code(sys::LIBSSH2_ERROR_SOCKET_RECV, 0), ErrorCode::ConnectionFailed);
        assert_eq!(cstr("a\0b").unwrap_err(), ErrorCode::InvalidName);
    }
}
