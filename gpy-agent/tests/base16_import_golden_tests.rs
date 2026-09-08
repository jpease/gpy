//! Golden import test: a real, published base16 scheme → a usable GPY palette.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use gpy_agent::config::types::ColorSpec;
use gpy_agent::import::base16::import_scheme;
use std::path::PathBuf;

/// Roles GPY's template engine expects every palette to define
/// (mirrors `palette_swap_tests.rs`'s built-in-palette coverage check).
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

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/base16")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()))
}

#[test]
fn real_world_scheme_maps_to_a_complete_usable_palette() {
    // Verbatim "Tokyo Night Dark" from tinted-theming/schemes @ spec-0.11
    // (base16/tokyo-night-dark.yaml) — a real, published scheme rather than a
    // hand-written slot list, so the importer sees the actual metadata shape
    // (an `author` field the parser must ignore) and slot values authors
    // publish, not just the minimal cases unit tests construct.
    let input = fixture("tokyo-night-dark.yaml");
    let cfg = import_scheme(&input).expect("real scheme imports cleanly");

    assert_eq!(cfg.name, "tokyo-night-dark");
    assert_eq!(
        cfg.description,
        "Imported base scheme: Tokyo Night Dark (dark)"
    );
    assert_eq!(
        cfg.colors.get("black").map(ColorSpec::as_str),
        Some("#1a1b26")
    );
    // base16 (not base24): no explicit bright accents in the source, so
    // brights are derived from the matching base08-0F accent slot.
    assert_eq!(
        cfg.colors.get("bright_magenta").map(ColorSpec::as_str),
        Some("#bb9af7")
    );
    assert_eq!(
        cfg.colors.get("bright_magenta"),
        cfg.colors.get("magenta"),
        "base16 bright_magenta must derive from the same accent as magenta"
    );

    let template_palette = cfg.to_template_palette();
    for role in REQUIRED_ROLES {
        assert!(
            template_palette.get(role).is_some(),
            "imported scheme must define role {role}"
        );
    }
}
