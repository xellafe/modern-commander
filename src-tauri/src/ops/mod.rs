//! Operazioni sui file: modello, espansione in elementi per file, politiche di conflitto,
//! esecutore e coda dei job.

pub mod conflict;
pub mod exec;
pub mod model;
pub mod plan;
pub mod queue;
mod runner;

pub use conflict::{next_free_name, resolve};
pub use model::*;
pub use plan::expand;
