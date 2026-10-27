//! Password delle connessioni nel Windows Credential Manager: target `ModernCommander:<id>`, tipo generico,
//! persistenza sul computer locale.

//! La password si salva come byte UTF-8 (non UTF-16): la legge solo quest'app.

use crate::error::AppError;
use crate::shell::hr_error;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn target(id: &str) -> Vec<u16> {
    wide(&format!("ModernCommander:{id}"))
}

/// Azzera un buffer che ha contenuto la password; le scritture volatili non si possono eliminare in compilazione.
fn wipe(buf: &mut [u8]) {
    for b in buf {
        // SAFETY: `b` è un riferimento valido e allineato.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

fn is_not_found(e: &windows::core::Error) -> bool {
    e.code() == ERROR_NOT_FOUND.to_hresult()
}

/// Nell'errore va solo l'HRESULT: mai la password.
pub fn store(id: &str, user: &str, secret: &str) -> Result<(), AppError> {
    let mut target = target(id);
    let mut user = wide(user);
    let mut blob = secret.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: tutti i puntatori di `cred` puntano a buffer vivi fino alla fine della chiamata.
    let res = unsafe { CredWriteW(&cred, 0) };
    wipe(&mut blob);
    res.map_err(|e| hr_error(&e, None))
}

pub fn load(id: &str) -> Result<Option<String>, AppError> {
    let target = target(id);
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target` è terminato da zero; `cred` riceve un buffer da liberare con `CredFree`.
    match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) } {
        Err(e) if is_not_found(&e) => return Ok(None),
        Err(e) => return Err(hr_error(&e, None)),
        Ok(()) => {}
    }
    // SAFETY: dopo un `CredReadW` riuscito `cred` è valido fino a `CredFree`, e il blob è lungo
    // `CredentialBlobSize` byte (o nullo).
    let secret = unsafe {
        let c = &*cred;
        let blob: &mut [u8] = if c.CredentialBlob.is_null() {
            &mut []
        } else {
            std::slice::from_raw_parts_mut(c.CredentialBlob, c.CredentialBlobSize as usize)
        };
        // `from_utf8` sul prestito: in caso di errore nessuna copia dei byte resta in giro.
        let s = std::str::from_utf8(blob).map(str::to_owned);
        wipe(blob);
        CredFree(cred as *const _);
        s
    };
    secret.map(Some).map_err(|_| AppError {
        detail: Some("credential not UTF-8".into()),
        ..AppError::new(crate::error::ErrorCode::Io, None)
    })
}

/// Eliminare una password che non c'è non è un errore.
pub fn delete(id: &str) -> Result<(), AppError> {
    let target = target(id);
    // SAFETY: `target` è terminato da zero.
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Err(e) if !is_not_found(&e) => Err(hr_error(&e, None)),
        _ => Ok(()),
    }
}

/// Archivio delle password: quello vero è il Credential Manager, i test ne usano uno in memoria.
pub trait Secrets: Send + Sync {
    fn store(&self, id: &str, user: &str, secret: &str) -> Result<(), AppError>;
    fn load(&self, id: &str) -> Result<Option<String>, AppError>;
    fn delete(&self, id: &str) -> Result<(), AppError>;
}

pub struct WindowsCredentials;

impl Secrets for WindowsCredentials {
    fn store(&self, id: &str, user: &str, secret: &str) -> Result<(), AppError> {
        store(id, user, secret)
    }
    fn load(&self, id: &str) -> Result<Option<String>, AppError> {
        load(id)
    }
    fn delete(&self, id: &str) -> Result<(), AppError> {
        delete(id)
    }
}

/// Archivio in memoria per i test: non tocca il Credential Manager.
#[cfg(test)]
#[derive(Default)]
pub struct MemSecrets(pub std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(test)]
impl Secrets for MemSecrets {
    fn store(&self, id: &str, _user: &str, secret: &str) -> Result<(), AppError> {
        self.0.lock().unwrap().insert(id.to_owned(), secret.to_owned());
        Ok(())
    }
    fn load(&self, id: &str) -> Result<Option<String>, AppError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }
    fn delete(&self, id: &str) -> Result<(), AppError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

/// Archivio che rifiuta di salvare, per i test: carica e cancella come uno vuoto.
#[cfg(test)]
pub struct Refuses;

#[cfg(test)]
impl Secrets for Refuses {
    fn store(&self, _: &str, _: &str, _: &str) -> Result<(), AppError> {
        Err(AppError::new(crate::error::ErrorCode::AccessDenied, None))
    }
    fn load(&self, _: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "tocca il Credential Manager vero"]
    fn credential_manager_roundtrip() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        // Niente `assert_eq!`: in caso di errore la password finirebbe nell'output del test.
        assert!(load(&id).unwrap().is_none());
        store(&id, "utente", "pàss wörd").unwrap();
        assert!(load(&id).unwrap().as_deref() == Some("pàss wörd"));
        delete(&id).unwrap();
        assert!(load(&id).unwrap().is_none());
        // Eliminare una password che non c'è non è un errore.
        delete(&id).unwrap();
    }
}
