//! Configurazione TLS dei client (FTPS, HTTPS), con il provider crittografico `ring`. Il certificato si verifica
//! con l'archivio di Windows (una CA aziendale installata sul PC vale come nel browser); se Windows lo rifiuta
//! (autofirmato, CA sconosciuta, nome diverso) passa solo un certificato la cui impronta l'utente ha accettato per
//! quell'host e quella porta in `known_hosts`. Mai "qualunque certificato".

use super::sftp::fingerprint;
use super::trust::{self, Trust};
use crate::error::{AppError, ErrorCode};
use rustls::ClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::Verifier;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Impronta (`SHA256:<base64>`) dell'ultimo certificato rifiutato. Chi chiama, se la connessione fallisce e lo slot
/// è pieno, restituisce `CERT_UNTRUSTED` con l'impronta in `detail`: l'utente la conferma e si riprova.
pub type Slot = Arc<Mutex<Option<String>>>;

/// Configurazione per un server; lo slot si riempie quando il suo certificato viene rifiutato.
pub fn client_config(host: &str, port: u16) -> Result<(Arc<ClientConfig>, Slot), AppError> {
    client_config_in(&crate::config::current_config_dir()?, host, port)
}

pub(crate) fn client_config_in(dir: &Path, host: &str, port: u16) -> Result<(Arc<ClientConfig>, Slot), AppError> {
    build(dir, host, port, rustls::DEFAULT_VERSIONS)
}

/// Per FTPS: solo TLS 1.2. I canali dati riprendono la sessione TLS del controllo, e molti server la esigono
/// (vsftpd: `require_ssl_reuse`, attivo di default). In TLS 1.3 ogni ticket vale una volta sola e il server ne manda
/// di nuovi solo a chi legge dopo l'handshake: dopo un paio di upload non ne restano e il server rifiuta il canale
/// ("522 session reuse required"). La sessione TLS 1.2 invece si riprende quante volte serve.
pub(crate) fn ftps_config_in(dir: &Path, host: &str, port: u16) -> Result<(Arc<ClientConfig>, Slot), AppError> {
    build(dir, host, port, &[&rustls::version::TLS12])
}

fn build(
    dir: &Path,
    host: &str,
    port: u16,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> Result<(Arc<ClientConfig>, Slot), AppError> {
    let (v, seen) = verifier(dir, host, port)?;
    let config = ClientConfig::builder_with_provider(v.provider.clone())
        .with_protocol_versions(versions)
        .map_err(tls_error)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(v))
        .with_no_client_auth();
    Ok((Arc::new(config), seen))
}

fn tls_error(e: rustls::Error) -> AppError {
    AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) }
}

/// Verificatore di Windows, poi le impronte accettate.
#[derive(Debug)]
struct Pinned {
    platform: Verifier,
    provider: Arc<rustls::crypto::CryptoProvider>,
    dir: PathBuf,
    host: String,
    port: u16,
    seen: Slot,
}

fn verifier(dir: &Path, host: &str, port: u16) -> Result<(Pinned, Slot), AppError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let platform = Verifier::new(provider.clone()).map_err(tls_error)?;
    let seen = Slot::default();
    let v = Pinned {
        platform,
        provider,
        dir: dir.to_owned(),
        host: host.to_owned(),
        port,
        seen: seen.clone(),
    };
    Ok((v, seen))
}

impl ServerCertVerifier for Pinned {
    /// Un'impronta cambiata (`Trust::Changed`) si rifiuta come una sconosciuta: si chiede di nuovo.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let err = match self.platform.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        let fp = fingerprint(end_entity.as_ref()).map_err(|_| err.clone())?;
        if matches!(trust::check_in(&self.dir, "tls", &self.host, self.port, &fp), Ok(Trust::Known)) {
            return Ok(ServerCertVerified::assertion());
        }
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp);
        Err(err)
    }

    // Le firme dell'handshake si verificano con la chiave del certificato, accettato da Windows o dall'utente.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.platform.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.platform.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.platform.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_pinning_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (v, seen) = verifier(dir.path(), "localhost", 990).unwrap();
        let name = ServerName::try_from("localhost").unwrap();
        let verify = |cert: &CertificateDer| v.verify_server_cert(cert, &[], &name, &[], UnixTime::now());
        let self_signed = || rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap().cert;
        let (a, b) = (self_signed(), self_signed());

        // Windows non lo conosce e l'impronta non è accettata: rifiuto, con l'impronta nello slot.
        assert!(verify(a.der()).is_err());
        let fp = seen.lock().unwrap().take().expect("impronta");
        assert!(fp.starts_with("SHA256:") && fp.len() > 40, "{fp}");

        // Accettata per host e porta, lo stesso certificato passa.
        trust::accept_in(dir.path(), "tls", "localhost", 990, &fp).unwrap();
        assert!(verify(a.der()).is_ok());
        assert!(seen.lock().unwrap().is_none());

        // Un altro certificato (rinnovato, o di un altro server) si rifiuta e si chiede di nuovo.
        assert!(verify(b.der()).is_err());
        let other = seen.lock().unwrap().take().expect("impronta nuova");
        assert_ne!(other, fp);
        assert_eq!(trust::check_in(dir.path(), "tls", "localhost", 990, &other).unwrap(), Trust::Changed);

        // L'impronta vale solo per quella porta.
        let (v2, _) = verifier(dir.path(), "localhost", 21).unwrap();
        assert!(v2.verify_server_cert(a.der(), &[], &name, &[], UnixTime::now()).is_err());

        // La configurazione completa si costruisce con il provider `ring`.
        client_config_in(dir.path(), "localhost", 990).unwrap();
    }
}
