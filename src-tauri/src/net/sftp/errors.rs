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

/// `UnexpectedBehavior` di `russh-sftp` che dicono che il canale non c'è più (il testo è l'unico modo di
/// distinguerli): `SendError: ..`/`RecvError: ..` dalle `From` di `mpsc::SendError` e `oneshot::RecvError`,
/// `sender dropped` (risposta mai arrivata, `rawsession.rs`), `session closed` (invio a canale chiuso), `I/O: ..` e
/// `Unexpected EOF on stream` (il `crate::error::Error` interno convertito).
fn channel_gone(m: &str) -> bool {
    ["SendError: ", "RecvError: ", "I/O: "].iter().any(|p| m.starts_with(p))
        || ["sender dropped", "session closed", "Unexpected EOF on stream"].contains(&m)
}

/// Errore del client SFTP; un canale chiuso è `ConnectionFailed` (`Sessions` riapre la sessione).
pub(super) fn sftp_error(e: SftpError, path: Option<&str>) -> AppError {
    let code = match &e {
        SftpError::Status(s) => return app_error(status_code(s.status_code), path, s.error_message.clone()),
        SftpError::Timeout => ErrorCode::Timeout,
        SftpError::IO(_) => ErrorCode::ConnectionFailed,
        SftpError::UnexpectedBehavior(m) if channel_gone(m) => ErrorCode::ConnectionFailed,
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

    /// Il `crate::error::Error` di russh-sftp è privato: lo si raggiunge come errore di `de::from_bytes`.
    fn internal(e: std::io::Error) -> SftpError {
        fn conv<B, E: From<std::io::Error>>(_: impl FnOnce(&mut B) -> Result<u8, E>, e: std::io::Error) -> E {
            E::from(e)
        }
        conv(|b| russh_sftp::de::from_bytes::<u8>(b), e).into()
    }

    #[test]
    fn closed_channel_is_connection_failed() {
        use std::io::ErrorKind;
        let eof = internal(ErrorKind::UnexpectedEof.into());
        assert_eq!(eof.to_string(), "Unexpected EOF on stream");
        let io = internal(ErrorKind::BrokenPipe.into());
        assert!(io.to_string().starts_with("I/O: "));
        // `rawsession.rs`: invio con il canale già chiuso.
        let closed = SftpError::UnexpectedBehavior("session closed".into());
        for e in [eof, io, closed] {
            assert!(matches!(e, SftpError::UnexpectedBehavior(_)));
            assert_eq!(sftp_error(e, None).code, ErrorCode::ConnectionFailed);
        }
    }

    /// Il server chiude il canale dopo aver ricevuto la richiesta: la risposta non arriva più.
    #[test]
    fn sender_dropped_is_connection_failed() {
        use russh_sftp::client::RawSftpSession;
        use tokio::io::AsyncReadExt;
        let e = super::super::rt::block(std::time::Duration::from_secs(5), async {
            let (client, mut server) = tokio::io::duplex(1024);
            let sftp = RawSftpSession::new(client);
            let hang_up = async move {
                let _ = server.read(&mut [0; 64]).await;
            };
            Ok(tokio::join!(sftp.init(), hang_up).0.unwrap_err())
        })
        .unwrap();
        assert_eq!(e.to_string(), "sender dropped");
        assert_eq!(sftp_error(e, None).code, ErrorCode::ConnectionFailed);
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
