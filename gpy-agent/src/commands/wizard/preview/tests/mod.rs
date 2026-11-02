use super::*;
use crate::config::types::{Icon, PaletteName, ThemeName};
use crate::git::{RepositoryState, RepositoryStatus};
use crate::template::{Palette, Style as TStyle};

mod clock_and_status;
mod colors;
mod segments;

fn make_facts() -> PreviewFacts {
    PreviewFacts {
        repository_status: None,
        languages: Vec::new(),
        sample_duration_ms: 128,
        sample_character_success: true,
    }
}

fn make_config(enabled_segments: &[&str]) -> Config {
    let mut config = Config::default();
    config.ui.theme = ThemeName::new("default".to_owned()).expect("valid theme name");
    config.ui.palette = PaletteName::new("default".to_owned()).expect("valid palette name");
    config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
    config
}

/// Builtin lists; segment order matters for chain-position tests.
fn test_state(config: Config) -> WizardState {
    crate::commands::wizard::state::builtin_state(config)
}
