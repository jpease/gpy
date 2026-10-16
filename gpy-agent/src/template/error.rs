//! Error type for template parsing and evaluation.

use thiserror::Error;

/// Errors produced while parsing or evaluating a Starship-style template.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// A bracket or paren was opened but never closed (or vice versa).
    #[error("unbalanced '{delimiter}' at character {position}")]
    Unbalanced {
        /// The delimiter that was unbalanced.
        delimiter: char,
        /// Character index into the template where the imbalance was detected.
        position: usize,
    },

    /// A backslash escape was not followed by an escapable character.
    #[error("invalid escape at character {position}")]
    BadEscape {
        /// Character index of the offending backslash.
        position: usize,
    },

    /// A style token could not be parsed.
    #[error("invalid style token: {token}")]
    InvalidStyle {
        /// The token that failed to parse.
        token: String,
    },

    /// A color name/value was not recognized.
    #[error("unknown color: {name}")]
    UnknownColor {
        /// The unrecognized color text.
        name: String,
    },

    /// Template nesting exceeded the maximum supported depth.
    #[error("template nesting exceeds the maximum depth of {limit}")]
    TooDeep {
        /// The depth limit that was exceeded.
        limit: usize,
    },
}

/// Convenience alias for template results.
pub type Result<T> = std::result::Result<T, TemplateError>;
