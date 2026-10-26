//! base16/base24 color-scheme importer (tinted-theming).
//!
//! Parses into typed values, then maps slots 1:1 onto GPY palette roles.

pub mod hex;
pub mod map;
pub mod scheme;

pub use hex::Hex;
pub use map::to_palette_config;
pub use scheme::{Base16Scheme, SchemeSystem, parse_scheme};

/// Errors from importing a base16/base24 scheme.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// A color value was not a 6-digit hex string.
    #[error("invalid hex color '{0}' (expected 6 hex digits)")]
    BadHex(String),
    /// A required scheme field or slot was missing.
    #[error("missing scheme field/slot: {0}")]
    Missing(String),
    /// The input was not a recognizable base16/base24 scheme.
    #[error("malformed base16/base24 scheme: {0}")]
    Malformed(String),
}

/// Result alias for importer operations.
pub type Result<T> = std::result::Result<T, ImportError>;

/// Parse and map a scheme string into a GPY palette in one step.
///
/// # Errors
/// Returns [`ImportError`] when parsing or mapping fails.
pub fn import_scheme(input: &str) -> Result<crate::palette::PaletteConfig> {
    let scheme = parse_scheme(input)?;
    map::to_palette_config(&scheme)
}
