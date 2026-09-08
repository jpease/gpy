//! SP4: swapping ui.palette recolors languages while the starship palette is vanilla.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::palette::PaletteManager;
use gpy_agent::template::Color;

#[test]
fn starship_palette_resolves_off_ansi_languages_to_vanilla_indices() {
    let pal = PaletteManager::new("starship")
        .unwrap()
        .get()
        .to_template_palette();
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
        let pal = PaletteManager::new(name)
            .unwrap_or_else(|e| panic!("{name} loads: {e}"))
            .get()
            .to_template_palette();
        for role in REQUIRED_ROLES {
            assert!(pal.get(role).is_some(), "{name} must define role {role}");
        }
    }
}

#[test]
fn swapping_to_catppuccin_recolors_swift_away_from_vanilla() {
    let pal = PaletteManager::new("catppuccin-mocha")
        .unwrap()
        .get()
        .to_template_palette();
    // swift → orange; under Catppuccin it must NOT be the vanilla 256-index 202.
    assert_ne!(pal.get("orange"), Some(Color::Ansi256(202)));
}
