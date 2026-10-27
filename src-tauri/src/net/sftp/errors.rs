//! Traduzione degli errori di `russh` e `russh-sftp` in `AppError`.
//!
//! Nessuno di questi errori porta la password: `detail` contiene solo il testo dell'errore.

use crate::error::{AppError, ErrorCode};
use russh::AlgorithmKind;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;

fn app_error(code: ErrorCode, path: Option<&str>, detail: impl Into<String>) -> AppError {
    AppError { code, path: path.map(str::to_owned), detail: Some(detail.into()) }
}

/// Lo stato di una risposta SFTP; quelli generici (`Failure`, per esempio un file che c'è già in SFTP v3) danno `Io`.
pub(super) fn status_code(code: StatusCode) -> ErrorCode {
    match code {
        StatusCode::NoSuchFile => ErrorCode::NotFound,
        StatusCode::PermissionDenied => ErrorCode::AccessDenied,
        StatusCode::OpUnsupported => ErrorCode::Unsupported,
        StatusCode::NoConnection | StatusCode::ConnectionLost => ErrorCode::ConnectionFailed,
        _ => ErrorCode::Io,
    }
}

/// Errore del client SFTP. `russh-sftp` produce `UnexpectedBehavior("SendError: ..")` e `("RecvError: ..")` dalle
/// `From` di `mpsc::SendError` e `oneshot::RecvError`: il canale verso il task della sessione non c'è più, quindi
/// la connessione è caduta (il prefisso è l'unico modo di distinguerli dagli altri `UnexpectedBehavior`).
pub(super) fn sftp_error(e: SftpError, path: Option<&str>) -> AppError {
    let code = match &e {
        SftpError::Status(s) => return app_error(status_code(s.status_code), path, s.error_message.clone()),
        SftpError::Timeout => ErrorCode::Timeout,
        SftpError::IO(_) => ErrorCode::ConnectionFailed,
        SftpError::UnexpectedBehavior(m) if m.starts_with("SendError: ") || m.starts_with("RecvError: ") => {
            ErrorCode::ConnectionFailed
        }
        _ => ErrorCode::Io,
    };
    app_error(code, path, e.to_string())
}

/// Errore di `russh`. Nessuna chiave dell'host in comune è `HostKeyUnsupported` (`detail`: i tipi offerti dal
/// server); gli altri algoritmi mancanti, le cadute e la chiusura del canale sono `ConnectionFailed`.
pub(super) fn ssh_error(e: &russh::Error) -> AppError {
    use russh::Error as E;
    let code = match e {
        E::NoCommonAlgo { kind: AlgorithmKind::Key, theirs, .. } => {
            return app_error(ErrorCode::HostKeyUnsupported, None, theirs.join(", "));
        }
        E::IO(io) if io.kind() == std::io::ErrorKind::TimedOut => ErrorCode::Timeout,
        E::ConnectionTimeout | E::KeepaliveTimeout | E::InactivityTimeout | E::Elapsed(_) => ErrorCode::Timeout,
        E::NoCommonAlgo { .. } | E::IO(_) | E::Disconnect | E::HUP | E::SendError | E::RecvError => {
            ErrorCode::ConnectionFailed
        }
        _ => ErrorCode::Io,
    };
    app_error(code, None, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_sftp::protocol::Status;

    #[test]
    fn sftp_status_mapping() {
        assert_eq!(status_code(StatusCode::NoSuchFile), ErrorCode::NotFound);
        assert_eq!(status_code(StatusCode::PermissionDenied), ErrorCode::AccessDenied);
        assert_eq!(status_code(StatusCode::OpUnsupported), ErrorCode::Unsupported);
        assert_eq!(status_code(StatusCode::NoConnection), ErrorCode::ConnectionFailed);
        assert_eq!(status_code(StatusCode::ConnectionLost), ErrorCode::ConnectionFailed);
        assert_eq!(status_code(StatusCode::Failure), ErrorCode::Io);

        let s = Status { id: 1, status_code: StatusCode::NoSuchFile, error_message: "gone".into(), language_tag: "en".into() };
        let e = sftp_error(SftpError::Status(s), Some("/a"));
        assert_eq!((e.code, e.path.as_deref(), e.detail.as_deref()), (ErrorCode::NotFound, Some("/a"), Some("gone")));
    }

    #[test]
    fn sftp_timeout_and_channel_errors() {
        assert_eq!(sftp_error(SftpError::Timeout, None).code, ErrorCode::Timeout);
        assert_eq!(sftp_error(SftpError::IO("x".into()), None).code, ErrorCode::ConnectionFailed);
        let send = SftpError::from(tokio::sync::mpsc::error::SendError(()));
        assert_eq!(sftp_error(send, None).code, ErrorCode::ConnectionFailed);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        drop(tx);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let recv = SftpError::from(rt.block_on(rx).unwrap_err());
        assert_eq!(sftp_error(recv, None).code, ErrorCode::ConnectionFailed);
        assert_eq!(sftp_error(SftpError::UnexpectedBehavior("altro".into()), None).code, ErrorCode::Io);
        assert_eq!(sftp_error(SftpError::UnexpectedPacket, None).code, ErrorCode::Io);
    }

    fn no_common(kind: AlgorithmKind) -> russh::Error {
        russh::Error::NoCommonAlgo { kind, ours: vec!["ssh-ed25519".into()], theirs: vec!["ssh-dss".into(), "ssh-rsa".into()] }
    }

    #[test]
    fn no_common_host_key_is_host_key_unsupported() {
        let e = ssh_error(&no_common(AlgorithmKind::Key));
        assert_eq!(e.code, ErrorCode::HostKeyUnsupported);
        assert_eq!(e.detail.as_deref(), Some("ssh-dss, ssh-rsa"));
    }

    #[test]
    fn no_common_cipher_is_connection_failed() {
        let e = ssh_error(&no_common(AlgorithmKind::Cipher));
        assert_eq!(e.code, ErrorCode::ConnectionFailed);
        assert!(e.detail.unwrap().contains("Cipher"));
    }

    #[test]
    fn ssh_io_timeouts_and_drops() {
        let io = |k| russh::Error::IO(std::io::Error::from(k));
        assert_eq!(ssh_error(&io(std::io::ErrorKind::TimedOut)).code, ErrorCode::Timeout);
        assert_eq!(ssh_error(&io(std::io::ErrorKind::ConnectionReset)).code, ErrorCode::ConnectionFailed);
        assert_eq!(ssh_error(&russh::Error::ConnectionTimeout).code, ErrorCode::Timeout);
        assert_eq!(ssh_error(&russh::Error::KeepaliveTimeout).code, ErrorCode::Timeout);
        assert_eq!(ssh_error(&russh::Error::InactivityTimeout).code, ErrorCode::Timeout);
        for e in [russh::Error::Disconnect, russh::Error::HUP, russh::Error::SendError, russh::Error::RecvError] {
            assert_eq!(ssh_error(&e).code, ErrorCode::ConnectionFailed);
        }
        assert_eq!(ssh_error(&russh::Error::Kex).code, ErrorCode::Io);
    }
}
