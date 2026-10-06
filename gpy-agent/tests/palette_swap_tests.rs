//! SP4: swapping ui.palette recolors languages while the starship palette is vanilla.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::palette::PaletteConfig;
use gpy_agent::template::{Color, Palette};
use std::path::PathBuf;

/// A shipped palette, read from the repo. `PaletteManager::new` would prefer
/// a same-named `~/.config/gpy/palettes/<name>.toml` and assert against the
/// developer's colors instead of the builtin's (#664).
fn builtin_palette(name: &str) -> Palette {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config/palettes")
        .join(format!("{name}.toml"));
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    toml::from_str::<PaletteConfig>(&content)
        .unwrap_or_else(|e| panic!("{name} parses: {e}"))
        .to_template_palette()
}

#[test]
fn starship_palette_resolves_off_ansi_languages_to_vanilla_indices() {
    let pal = builtin_palette("starship");
    assert_eq!(pal.get("orange"), Some(Color::Ansi256(202)));
    assert_eq!(pal.get("bright_magenta"), Some(Color::Ansi256(147)));
    assert_eq!(pal.get("bright_green"), Some(Color::Ansi256(149)));
}

const REQUIRED_ROLES: &[&str] = &[
    "red",
    "orange",
    "yellow",
    "green",
    "cyan",
    "blue",
    "magenta",
    "bright_green",
    "bright_magenta",
];

#[test]
fn every_scheme_builtin_defines_required_roles() {
    for name in [
        "catppuccin-latte",
        "catppuccin-frappe",
        "catppuccin-macchiato",
        "catppuccin-mocha",
        "nord",
        "gruvbox-dark-medium",
    ] {
        let pal = builtin_palette(name);
        for role in REQUIRED_ROLES {
            assert!(pal.get(role).is_some(), "{name} must define role {role}");
        }
    }
}

#[test]
fn swapping_to_catppuccin_recolors_swift_away_from_vanilla() {
    let pal = builtin_palette("catppuccin-mocha");
    // swift → orange; under Catppuccin it must NOT be the vanilla 256-index 202.
    assert_ne!(pal.get("orange"), Some(Color::Ansi256(202)));
}
