//! Livello virtuale sul file system: posizioni locali e remote.

pub mod loc;
pub mod local;
#[cfg(test)]
pub mod mem;
pub mod provider;
pub mod router;
