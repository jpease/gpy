//! base16/base24 color-scheme importer (tinted-theming).
//!
//! Parses a scheme into typed values (`Hex`, `Base16Scheme`) at the boundary,
//! then maps standard slots 1:1 onto GPY palette role names. Parse, don't validate.

pub mod hex;
pub mod map;
pub mod scheme;

pub use hex::Hex;
pub use map::to_palette_config;
pub use scheme::{Base16Scheme, SchemeSystem, parse_scheme};

/// Errors from importing a base16/base24 scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// A color value was not a 6-digit hex string.
    BadHex(String),
    /// A required scheme field or slot was missing.
    Missing(String),
    /// The input was not a recognizable base16/base24 scheme.
    Malformed(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadHex(v) => write!(f, "invalid hex color '{v}' (expected 6 hex digits)"),
            Self::Missing(v) => write!(f, "missing scheme field/slot: {v}"),
            Self::Malformed(v) => write!(f, "malformed base16/base24 scheme: {v}"),
        }
    }
}

impl std::error::Error for ImportError {}

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
