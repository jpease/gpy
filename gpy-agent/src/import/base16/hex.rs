//! Validated 6-digit hex color, parsed once at the import boundary.

use super::{ImportError, Result};

/// A 6-digit RGB hex color, stored lowercase without `#`. Always valid by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hex(String);

impl Hex {
    /// Parse a hex color (`#rrggbb` or `rrggbb`, any case).
    ///
    /// # Errors
    /// Returns [`ImportError::BadHex`] when not exactly 6 hex digits.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim().trim_start_matches('#').to_ascii_lowercase();
        if trimmed.len() == 6_usize && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Self(trimmed))
        } else {
            Err(ImportError::BadHex(raw.to_owned()))
        }
    }

    /// Render as a CSS `#rrggbb` string (the form GPY palettes store).
    #[must_use]
    pub fn as_css(&self) -> String {
        format!("#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    use super::Hex;
    use crate::import::base16::ImportError;

    #[test]
    fn parses_with_and_without_hash() {
        assert_eq!(Hex::parse("#1E1E2E").unwrap().as_css(), "#1e1e2e");
        assert_eq!(Hex::parse("a6e3a1").unwrap().as_css(), "#a6e3a1");
    }

    #[test]
    fn rejects_bad_hex() {
        assert_eq!(
            Hex::parse("xyz").unwrap_err(),
            ImportError::BadHex("xyz".to_owned())
        );
        assert!(Hex::parse("#12345").is_err());
    }
}
