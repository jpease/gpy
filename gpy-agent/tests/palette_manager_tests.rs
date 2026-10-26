//! Palette resolution against the builtins. A user file in
//! `$XDG_CONFIG_HOME/gpy/palettes` shadows a builtin, so each case points
//! `XDG_CONFIG_HOME` at an empty temp dir (serialized: env mutation).

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::config::Config;
use gpy_agent::config::types::PaletteName;
use gpy_agent::palette::resolve::palette_by_name;
use gpy_agent::palette::{
    PaletteCache, PaletteManager, PaletteSource, active_palette, load_active_palette,
};
use serial_test::serial;
use std::path::PathBuf;
use tempfile::TempDir;

/// Run `check` with an empty config root so no user palette shadows a builtin.
fn with_empty_config_home<T>(check: impl FnOnce() -> T) -> T {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    check()
}

/// Builtin palette names: one per file in `config/palettes`.
fn shipped_palette_names() -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/palettes");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
#[serial]
fn loads_builtin_default() {
    let mgr =
        with_empty_config_home(|| PaletteManager::new("default")).expect("default palette loads");
    assert!(mgr.get().colors.contains_key("red"));
}

#[test]
#[serial]
fn loads_builtin_starship() {
    let mgr =
        with_empty_config_home(|| PaletteManager::new("starship")).expect("starship palette loads");
    let colors = &mgr.get().colors;
    assert_eq!(colors.get("orange").expect("orange").as_str(), "202");
    assert_eq!(
        colors
            .get("bright_magenta")
            .expect("bright_magenta")
            .as_str(),
        "147"
    );
    assert_eq!(
        colors.get("bright_green").expect("bright_green").as_str(),
        "149"
    );
}

#[test]
#[serial]
fn discovery_includes_builtin_default_and_starship() {
    let found = with_empty_config_home(PaletteManager::discover_available_palettes);
    for name in ["default", "starship"] {
        assert!(
            found
                .iter()
                .any(|p| p.name == name && matches!(p.source, PaletteSource::Builtin)),
            "{name} is discovered as a builtin: {found:?}"
        );
    }
}

#[test]
#[serial]
fn all_builtins_load_and_are_discovered() {
    let shipped = shipped_palette_names();
    let (discovered, loaded) = with_empty_config_home(|| {
        let discovered: Vec<String> = PaletteManager::discover_available_palettes()
            .into_iter()
            .map(|p| p.name)
            .collect();
        let loaded: Vec<(String, bool)> = shipped
            .iter()
            .map(|name| (name.clone(), PaletteManager::new(name).is_ok()))
            .collect();
        (discovered, loaded)
    });

    assert_eq!(discovered, shipped, "every shipped palette is discovered");
    for (name, ok) in loaded {
        assert!(ok, "{name} loads");
    }
}

#[test]
#[serial]
fn default_config_resolves_builtin_default_palette() {
    // `ui.palette` defaults to the builtin "default", mapping ANSI names to themselves.
    let palette = with_empty_config_home(|| active_palette(&Config::default()));
    assert!(
        palette.get("green").is_some(),
        "default palette must define green"
    );
}

#[test]
#[serial]
fn default_palette_defines_required_named_colors() {
    let palette = with_empty_config_home(|| active_palette(&Config::default()));
    // Identity ANSI names (default.toml/text.toml use these directly).
    for name in [
        "black", "red", "green", "yellow", "blue", "cyan", "white", "magenta",
    ] {
        assert!(
            palette.get(name).is_some(),
            "default palette must define standard name '{name}'"
        );
    }
}

#[test]
#[serial]
fn default_palette_defines_orange_and_brown() {
    let palette = with_empty_config_home(|| active_palette(&Config::default()));
    assert!(
        palette.get("orange").is_some(),
        "default palette must define orange"
    );
    assert!(
        palette.get("brown").is_some(),
        "default palette must define brown"
    );
}

#[test]
#[serial]
fn palette_by_name_resolves_builtin_default_palette() {
    let palette = with_empty_config_home(|| palette_by_name("default"));
    assert!(
        palette.get("green").is_some(),
        "palette_by_name(\"default\") must define green"
    );
}

#[test]
#[serial]
fn palette_cache_starts_on_the_configured_palette() {
    let cache = with_empty_config_home(|| PaletteCache::from_config(&Config::default()));
    assert!(
        cache.get().get("green").is_some(),
        "initial palette must define green"
    );
}

/// Every way a user palette can be unusable is reported, not degraded.
#[test]
#[serial]
fn load_active_palette_reports_unusable_user_palettes() {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let palettes_dir = temp_dir.path().join("gpy").join("palettes");
    std::fs::create_dir_all(&palettes_dir).unwrap();
    let mut config = Config::default();
    config.ui.palette = PaletteName::new("mine".to_owned()).expect("valid name");

    for (case, content) in [
        ("malformed TOML", "[colors\nsignal = "),
        (
            "reference cycle",
            "[colors]\nsignal = \"loop\"\nloop = \"signal\"\n",
        ),
    ] {
        std::fs::write(palettes_dir.join("mine.toml"), content).unwrap();
        assert!(load_active_palette(&config).is_err(), "{case} must error");
    }

    std::fs::remove_file(palettes_dir.join("mine.toml")).unwrap();
    assert!(
        load_active_palette(&config).is_err(),
        "a missing palette must error"
    );
}

#[test]
#[serial]
fn validate_target_accepts_builtin_default() {
    let report =
        with_empty_config_home(|| gpy_agent::commands::palette::validate_target(Some("default")))
            .expect("default valid");
    assert_eq!(report.target, "default");
}

#[test]
#[serial]
fn validate_by_name_accepts_builtin_default() {
    let report =
        with_empty_config_home(|| gpy_agent::commands::palette::validate_by_name("default"))
            .expect("default valid");
    assert_eq!(report.target, "default");
    assert_eq!(report.source, "named palette");
}
