//! Operazioni sui file: modello, espansione in elementi per file e politiche di conflitto.

pub mod conflict;
pub mod model;
pub mod plan;

pub use conflict::{next_free_name, resolve};
pub use model::*;
pub use plan::expand;
