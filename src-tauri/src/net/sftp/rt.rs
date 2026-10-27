//! Runtime tokio dedicato all'SFTP (`russh`): i metodi del provider restano sincroni e vi si affacciano con `block`.
//!
//! I metodi del provider si chiamano da thread bloccanti, mai da un task tokio (`block_on` lì va in panico).

use crate::error::{AppError, ErrorCode};
use futures_util::future::BoxFuture;
use std::future::Future;
use std::sync::OnceLock;
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Il runtime, creato al primo uso. Se la creazione fallisce (`IO`, il motivo in `detail`) si ritenta alla chiamata
/// successiva.
pub(super) fn runtime() -> Result<&'static Runtime, AppError> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("sftp-rt")
        .enable_all()
        .build()
        .map_err(|e| AppError { detail: Some(e.to_string()), ..AppError::new(ErrorCode::Io, None) })?;
    Ok(RUNTIME.get_or_init(|| rt))
}

/// Aspetta `fut` per al massimo `limit`: scaduto, `TIMEOUT`.
pub(super) fn block<T>(limit: Duration, fut: impl Future<Output = Result<T, AppError>>) -> Result<T, AppError> {
    // `timeout` si crea dentro il runtime: fuori, il suo timer non troverebbe il reactor.
    let done = runtime()?.block_on(async { tokio::time::timeout(limit, fut).await });
    done.map_err(|_| AppError::new(ErrorCode::Timeout, None))?
}

/// Il primo poll di `fut` nel contesto del runtime, senza bloccare. Una richiesta di russh-sftp parte lì (`send_bytes`
/// accoda il pacchetto prima di sospendersi), quindi le richieste partono nell'ordine delle chiamate; la risposta si
/// aspetta poi con `block`, o si scarta lasciando cadere il future.
pub(super) fn start<T: Send + 'static>(
    fut: impl Future<Output = T> + Send + 'static,
) -> Result<BoxFuture<'static, T>, AppError> {
    let mut fut: BoxFuture<'static, T> = Box::pin(fut);
    let _rt = runtime()?.enter();
    Ok(match fut.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => Box::pin(std::future::ready(v)),
        Poll::Pending => fut,
    })
}

/// Avvia `fut` sul runtime senza aspettarlo; senza runtime non fa nulla.
pub(super) fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    if let Ok(rt) = runtime() {
        rt.spawn(fut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn block_returns_the_value() {
        assert_eq!(block(Duration::from_secs(1), async { Ok(7) }).unwrap(), 7);
    }

    #[test]
    fn block_times_out() {
        let t = Instant::now();
        let r: Result<(), _> = block(Duration::from_millis(20), async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        });
        assert_eq!(r.unwrap_err().code, ErrorCode::Timeout);
        assert!(t.elapsed() < Duration::from_millis(400));
    }

    #[test]
    fn spawn_runs_off_the_calling_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(async move {
            tx.send(std::thread::current().name().map(str::to_owned)).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().as_deref(), Some("sftp-rt"));
    }
}
