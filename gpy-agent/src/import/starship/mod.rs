//! Pure, testable translation of a `starship.toml` into a GPY palette + prompt theme.
//!
//! This module performs no I/O and spawns no processes. The command layer
//! (`crate::commands::theme_import`) reads files, calls [`emit::build`], and
//! presents the result. Anything Starship expresses that GPY cannot represent
//! becomes a [`warn::Warning`] rather than a hard failure.

mod emit;
mod error;
mod layout;
pub mod model;
mod modules;
mod palette;
mod warn;

pub use emit::{ImportArtifacts, build};
pub use error::{ImportError, Result};
pub use layout::{derive_segments, map_module};
pub use model::{StarshipConfig, parse};
pub use modules::{
    LanguageTranslation, canonical_language, first_color_token, inline_style_vars,
    retain_known_vars, translate_character, translate_directory, translate_duration, translate_git,
    translate_languages, translate_time,
};
pub use palette::{selected_palette, translate_palette};
pub use warn::{Warning, WarningKind, Warnings};
