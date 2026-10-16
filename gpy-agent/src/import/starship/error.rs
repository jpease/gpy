//! Error type for the Starship importer's pure translation layer.

use thiserror::Error;

/// Errors produced while parsing a `starship.toml` into the importer model.
///
/// I/O errors live in `crate::commands::theme_import`.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The input could not be parsed as a Starship TOML document.
    #[error("failed to parse starship.toml: {message}")]
    Parse {
        /// Human-readable parser message (from the `toml` crate).
        message: String,
    },
    /// The embedded builtin `starship` preset failed to parse.
    #[error("failed to load the builtin starship preset: {message}")]
    Preset {
        /// Human-readable parser message.
        message: String,
    },
}

/// Convenience alias for importer translation results.
pub type Result<T> = std::result::Result<T, ImportError>;
