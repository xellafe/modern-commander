//! Operazioni sui file: modello, espansione in elementi per file, politiche di conflitto,
//! esecutore e coda dei job.

pub mod conflict;
pub mod exec;
pub mod model;
pub mod plan;
pub mod queue;
pub mod rooted;
mod runner;

pub use conflict::{next_free_name, resolve};
pub use model::*;
pub use plan::expand;

/// Per i test: nega l'elenco di una cartella a Everyone; il divieto si toglie al drop, anche
/// dopo un panic (senza, la tempdir non si cancella).
#[cfg(test)]
pub(crate) struct DenyList(std::path::PathBuf);

#[cfg(test)]
impl DenyList {
    pub(crate) fn new(dir: &std::path::Path) -> Self {
        let guard = DenyList(dir.to_path_buf());
        let ok = std::process::Command::new("icacls").arg(dir).args(["/deny", "*S-1-1-0:(RD)"]).output().unwrap();
        assert!(ok.status.success(), "icacls /deny non riuscito");
        guard
    }
}

#[cfg(test)]
impl Drop for DenyList {
    fn drop(&mut self) {
        let _ = std::process::Command::new("icacls").arg(&self.0).args(["/remove:d", "*S-1-1-0"]).output();
    }
}
