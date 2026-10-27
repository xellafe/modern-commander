//! Hash di un file con BCrypt (CNG) di Windows: nessuna crate in più.

use super::content::open_shared;
use crate::error::{AppError, ErrorCode};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_MD5_ALG_HANDLE, BCRYPT_MD5_ALGORITHM, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCRYPT_SHA1_ALGORITHM,
    BCRYPT_SHA256_ALG_HANDLE, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHash, BCryptHashData, BCryptOpenAlgorithmProvider,
};

/// Blocchi da 1 MiB: l'annullamento si controlla a ogni blocco.
const BLOCK: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    Md5,
    Sha1,
    Sha256,
}

/// Provider BCrypt aperto per un algoritmo; si chiude al drop, su ogni percorso d'uscita.
struct Provider(BCRYPT_ALG_HANDLE);

/// Hash in corso; si distrugge al drop.
struct Hash(BCRYPT_HASH_HANDLE);

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}

impl Drop for Hash {
    fn drop(&mut self) {
        let _ = unsafe { BCryptDestroyHash(self.0) };
    }
}

fn check(status: NTSTATUS, path: &Path) -> Result<(), AppError> {
    if status.0 >= 0 {
        return Ok(());
    }
    Err(AppError {
        detail: Some(format!("BCrypt 0x{:08X}", status.0 as u32)),
        ..AppError::new(ErrorCode::Io, Some(path))
    })
}

/// Digest di `path` in esadecimale minuscolo; legge in condivisione piena (vedi `open_shared`).
pub fn hash_file(path: &Path, algo: HashAlgo, cancel: &AtomicBool) -> Result<String, AppError> {
    let (alg_id, digest_len) = match algo {
        HashAlgo::Md5 => (BCRYPT_MD5_ALGORITHM, 16),
        HashAlgo::Sha1 => (BCRYPT_SHA1_ALGORITHM, 20),
        HashAlgo::Sha256 => (BCRYPT_SHA256_ALGORITHM, 32),
    };
    let cancelled = || AppError::new(ErrorCode::Cancelled, Some(path));
    let mut file = open_shared(path).map_err(|e| AppError::from_io(&e, path))?;

    let mut alg = BCRYPT_ALG_HANDLE::default();
    check(unsafe { BCryptOpenAlgorithmProvider(&mut alg, alg_id, None, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)) }, path)?;
    let provider = Provider(alg);
    let mut h = BCRYPT_HASH_HANDLE::default();
    // Senza buffer proprio: l'oggetto hash lo alloca BCrypt e lo libera `BCryptDestroyHash`.
    check(unsafe { BCryptCreateHash(provider.0, &mut h, None, None, 0) }, path)?;
    let hash = Hash(h);

    let mut buf = vec![0u8; BLOCK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let n = file.read(&mut buf).map_err(|e| AppError::from_io(&e, path))?;
        if n == 0 {
            break;
        }
        check(unsafe { BCryptHashData(hash.0, &buf[..n], 0) }, path)?;
    }
    let mut digest = vec![0u8; digest_len];
    check(unsafe { BCryptFinishHash(hash.0, &mut digest, 0) }, path)?;
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// SHA-256 di un buffer in memoria (impronte delle chiavi e dei certificati, Digest HTTP).
pub fn sha256(data: &[u8]) -> Result<[u8; 32], AppError> {
    let mut digest = [0u8; 32];
    check(unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut digest) }, Path::new(""))?;
    Ok(digest)
}

/// MD5 di un buffer in memoria (Digest HTTP con `algorithm=MD5`).
pub fn md5(data: &[u8]) -> Result<[u8; 16], AppError> {
    let mut digest = [0u8; 16];
    check(unsafe { BCryptHash(BCRYPT_MD5_ALG_HANDLE, None, data, &mut digest) }, Path::new(""))?;
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("abc");
        std::fs::write(&f, b"abc").unwrap();
        let no = AtomicBool::new(false);
        assert_eq!(hash_file(&f, HashAlgo::Md5, &no).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_file(&f, HashAlgo::Sha1, &no).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hash_file(&f, HashAlgo::Sha256, &no).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let e = d.path().join("empty");
        std::fs::write(&e, b"").unwrap();
        assert_eq!(hash_file(&e, HashAlgo::Sha256, &no).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hash_file(&f, HashAlgo::Md5, &AtomicBool::new(true)).unwrap_err().code, ErrorCode::Cancelled);
    }

    #[test]
    fn hashes_across_blocks_and_reports_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("big");
        std::fs::write(&f, vec![b'a'; 2_621_440]).unwrap(); // 2,5 blocchi da 1 MiB
        let no = AtomicBool::new(false);
        assert_eq!(hash_file(&f, HashAlgo::Sha256, &no).unwrap(), "b19fda75b6c96f0cd6a27a6c371352dbe76e5a5f4ac1a10fb1352df725eb04be");
        assert_eq!(hash_file(&d.path().join("gone"), HashAlgo::Md5, &no).unwrap_err().code, ErrorCode::NotFound);
    }
}
