//! Property tests for theme color validation and theme export invariants.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use gpy_agent::config::Config;
use gpy_agent::config::validation::colors::{canonical_color, is_valid_color};
use gpy_agent::shell::Shell;
use gpy_agent::template::Palette;
use gpy_agent::theme::ThemeManager;
use proptest::prelude::*;

fn named_color_strategy() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright_black",
        "bright_red",
        "bright_green",
        "bright_yellow",
        "bright_blue",
        "bright_magenta",
        "bright_cyan",
        "bright_white",
    ])
}

fn default_fish_export() -> String {
    let theme_mgr = ThemeManager::builtin("default").expect("create theme manager");
    theme_mgr.export(Shell::Fish, &Config::default(), &Palette::default())
}

proptest! {
    /// Every 24-bit color, in either case, is accepted and canonicalised to lowercase.
    #[test]
    fn hex_colors_canonicalise_to_lowercase(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
        let lower = format!("#{r:02x}{g:02x}{b:02x}");
        let upper = lower.to_ascii_uppercase();

        prop_assert_eq!(canonical_color(&lower), Some(lower.clone()));
        prop_assert_eq!(canonical_color(&upper), Some(lower));
    }

    /// Every ANSI color name is accepted.
    #[test]
    fn named_colors_are_valid(color in named_color_strategy()) {
        prop_assert!(is_valid_color(color), "{color} should be a valid theme color");
    }

    /// Canonicalisation never panics and its output is a fixed point.
    #[test]
    fn canonical_color_is_idempotent(value in ".{0,24}") {
        if let Some(canonical) = canonical_color(&value) {
            prop_assert_eq!(canonical_color(&canonical), Some(canonical));
        }
    }
}

#[test]
fn fish_export_sets_variables() {
    let export = default_fish_export();

    assert!(export.contains("set -gx") || export.contains("set -g"));
}

#[test]
fn fish_export_is_idempotent() {
    assert_eq!(default_fish_export(), default_fish_export());
}

#[test]
fn fish_export_has_no_raw_ansi_escapes() {
    assert!(!default_fish_export().contains('\x1b'));
}

#[test]
fn default_config_round_trips_through_toml() {
    let config = Config::default();

    let toml_str = toml::to_string(&config).expect("serialize config");
    let roundtrip: Config = toml::from_str(&toml_str).expect("deserialize config");

    assert_eq!(
        toml_str,
        toml::to_string(&roundtrip).expect("serialize roundtrip")
    );
}
