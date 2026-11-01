#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use super::*;
use crate::config::types::{PaletteName, ThemeName};

mod dirty_and_apply;
mod edits;
mod filter;
mod navigation;
mod selection;

fn test_state(config: Config) -> WizardState {
    builtin_state(config)
}

/// Avoids two bool params (`max-fn-params-bools`).
#[derive(Clone, Copy)]
struct GitLanguageFlags {
    git: bool,
    language: bool,
}

/// Test config; `git`/`language` are listed only if the caller adds them.
fn make_config(
    theme: &str,
    palette: &str,
    flags: GitLanguageFlags,
    enabled_segments: &[&str],
) -> Config {
    let mut config = Config::default();
    config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
    config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
    config.git.enabled = flags.git;
    config.language.enabled = flags.language;
    config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
    config
}
