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

/// Test config; `git`/`language` are listed only if the caller adds them.
fn make_config(theme: &str, palette: &str, enabled_segments: &[&str]) -> Config {
    let mut config = Config::default();
    config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
    config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
    config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
    config
}
