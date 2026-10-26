//! Pure, testable translation of a `starship.toml` into a GPY palette + prompt theme.
//!
//! No I/O: the command layer reads files and calls [`build`]; unrepresentable
//! constructs become [`Warning`]s.

mod emit;
mod error;
mod format;
mod layout;
pub mod model;
mod modules;
mod palette;
mod warn;

pub use emit::{ImportArtifacts, build};
pub use error::ImportError;
pub use model::parse;
pub use warn::{Warning, WarningKind, Warnings};
