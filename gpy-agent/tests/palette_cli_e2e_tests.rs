//! `gpy palette` through the real binary (#648).
//!
//! Replaces the in-process `palette_cli_tests.rs`, which mutated
//! `XDG_CONFIG_HOME` for the whole test process and only exercised the
//! library functions behind the subcommands. Every case here runs `gpy`
//! against an isolated `CliTestEnv` and asserts on what a user sees.
//!
//! Hermetic (no socket, no agent), so it runs on every platform the CLI
//! builds for (#653).
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

mod common;

use common::{CliCommandResult, CliTestEnv};
use std::fmt::Write as _;
use std::path::PathBuf;

/// The base16 scheme fixture shared with the import golden tests.
fn tokyo_night_fixture() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/base16/tokyo-night-dark.yaml");
    assert!(path.exists(), "fixture missing: {}", path.display());
    path
}

fn palette_section(env: &CliTestEnv) -> String {
    let config = std::fs::read_to_string(env.config_path()).expect("read config");
    config
        .lines()
        .find(|line| line.trim_start().starts_with("palette"))
        .map(str::to_owned)
        .unwrap_or_default()
}

fn assert_error_names(result: &CliCommandResult, needle: &str, context: &str) {
    assert_eq!(result.exit_code, 1_i32, "{context}: {result:?}");
    assert!(
        result.stderr.starts_with("Error: "),
        "{context}: one Error: line: {result:?}"
    );
    assert!(
        result.stderr.contains(needle),
        "{context}: must name {needle}: {result:?}"
    );
}

#[test]
fn list_shows_builtins_and_marks_the_active_one() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let listed = env.run_gpy(&["palette", "list"]).expect("spawn gpy");
    listed.assert_success("gpy palette list");

    let entries: Vec<&str> = listed
        .stdout
        .lines()
        .filter(|line| line.starts_with("  "))
        .map(str::trim)
        .collect();
    assert!(
        entries.contains(&"default * [builtin]"),
        "the builtin default is listed and marked active: {listed:?}"
    );
    for builtin in [
        "nord",
        "starship",
        "catppuccin-mocha",
        "gruvbox-dark-medium",
    ] {
        assert!(
            entries.contains(&format!("{builtin} [builtin]").as_str()),
            "builtin {builtin} must be listed: {listed:?}"
        );
    }
}

#[test]
fn show_prints_the_active_palette_name() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let shown = env.run_gpy(&["palette", "show"]).expect("spawn gpy");
    shown.assert_success("gpy palette show");
    assert_eq!(shown.stdout.trim(), "default", "{shown:?}");
}

#[test]
fn use_writes_ui_palette_and_show_follows() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let switched = env.run_gpy(&["palette", "use", "nord"]).expect("spawn gpy");
    switched.assert_success("gpy palette use nord");
    assert!(
        switched.stdout.contains("Switched to 'nord' palette"),
        "{switched:?}"
    );
    assert_eq!(
        palette_section(&env).replace(' ', ""),
        "palette=\"nord\"",
        "ui.palette is persisted"
    );

    let shown = env.run_gpy(&["palette", "show"]).expect("spawn gpy");
    assert_eq!(shown.stdout.trim(), "nord", "{shown:?}");

    let listed = env.run_gpy(&["palette", "list"]).expect("spawn gpy");
    assert!(
        listed.stdout.contains("  nord * [builtin]"),
        "list marks the new active palette: {listed:?}"
    );
}

#[test]
fn use_unknown_palette_exits_1_naming_it() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let result = env.run_gpy(&["palette", "use", "nope"]).expect("spawn gpy");
    assert_error_names(&result, "'nope'", "gpy palette use nope");
    assert!(
        palette_section(&env).is_empty(),
        "a failed switch leaves the config untouched (no ui.palette line is written)"
    );
    let shown = env.run_gpy(&["palette", "show"]).expect("spawn gpy");
    assert_eq!(shown.stdout.trim(), "default", "{shown:?}");
}

#[test]
fn validate_covers_active_named_file_and_unknown_targets() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let active = env.run_gpy(&["palette", "validate"]).expect("spawn gpy");
    active.assert_success("gpy palette validate");
    assert!(
        active.stdout.contains("✅ Palette validation passed"),
        "{active:?}"
    );
    assert!(active.stdout.contains("target: default"), "{active:?}");
    assert!(
        active.stdout.contains("source: active config palette"),
        "{active:?}"
    );

    let named = env
        .run_gpy(&["palette", "validate", "nord"])
        .expect("spawn gpy");
    named.assert_success("gpy palette validate nord");
    assert!(named.stdout.contains("target: nord"), "{named:?}");

    let imported = env
        .run_gpy(&["palette", "import", tokyo_night_fixture().to_str().unwrap()])
        .expect("spawn gpy");
    imported.assert_success("gpy palette import");
    let file = env.config_dir().join("palettes/tokyo-night-dark.toml");
    assert!(file.exists(), "import writes {}", file.display());
    let by_file = env
        .run_gpy(&["palette", "validate", file.to_str().unwrap()])
        .expect("spawn gpy");
    by_file.assert_success("gpy palette validate <file>");
    assert!(
        by_file.stdout.contains("✅ Palette validation passed"),
        "{by_file:?}"
    );

    let unknown = env
        .run_gpy(&["palette", "validate", "nope"])
        .expect("spawn gpy");
    assert_eq!(unknown.exit_code, 1_i32, "{unknown:?}");
    assert!(
        unknown.stdout.contains("❌ palette validate failed"),
        "{unknown:?}"
    );
    assert!(unknown.stdout.contains("'nope'"), "{unknown:?}");
    assert!(
        unknown.stdout.contains("remediation:"),
        "a failed validation carries a remediation hint: {unknown:?}"
    );
}

#[test]
fn import_names_the_palette_after_the_scheme_and_refuses_to_overwrite() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let fixture = tokyo_night_fixture();

    let first = env
        .run_gpy(&["palette", "import", fixture.to_str().unwrap()])
        .expect("spawn gpy");
    first.assert_success("first import");
    assert!(
        first.stdout.contains("Imported palette 'tokyo-night-dark'"),
        "{first:?}"
    );

    let listed = env.run_gpy(&["palette", "list"]).expect("spawn gpy");
    assert!(
        listed.stdout.contains("  tokyo-night-dark [user]"),
        "an imported palette lists as user-provided: {listed:?}"
    );

    let second = env
        .run_gpy(&["palette", "import", fixture.to_str().unwrap()])
        .expect("spawn gpy");
    assert_error_names(&second, "already exists", "second import without --force");
    assert!(second.stderr.contains("--force"), "{second:?}");

    let forced = env
        .run_gpy(&["palette", "import", fixture.to_str().unwrap(), "--force"])
        .expect("spawn gpy");
    forced.assert_success("import --force");
}

#[test]
fn import_refuses_to_shadow_builtin_palette_without_force() {
    // #691: a scheme named `Nord` imports as `nord`, a builtin palette; the
    // user file would silently shadow it.
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let slots = (0_u8..16_u8).fold(String::new(), |mut acc, slot| {
        writeln!(acc, "  base0{slot:X}: \"#2E3440\"").expect("write to String");
        acc
    });
    let scheme = env.root().join("nord.yaml");
    std::fs::write(
        &scheme,
        format!("system: \"base16\"\nname: \"Nord\"\npalette:\n{slots}"),
    )
    .expect("write scheme");
    let dest = env.config_dir().join("palettes").join("nord.toml");

    let refused = env
        .run_gpy(&["palette", "import", scheme.to_str().unwrap()])
        .expect("spawn gpy");
    assert_error_names(&refused, "builtin", "import over builtin nord");
    assert!(refused.stderr.contains("--name"), "{refused:?}");
    assert!(!dest.exists(), "no palette file written");

    let listed = env.run_gpy(&["palette", "list"]).expect("spawn gpy");
    assert!(listed.stdout.contains("  nord [builtin]"), "{listed:?}");

    env.run_gpy(&["palette", "import", scheme.to_str().unwrap(), "--force"])
        .expect("spawn gpy")
        .assert_success("import --force");
    assert!(dest.exists(), "--force writes the palette");
}

#[test]
fn import_honours_name_override_and_use_can_select_it() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let fixture = tokyo_night_fixture();

    let imported = env
        .run_gpy(&[
            "palette",
            "import",
            fixture.to_str().unwrap(),
            "--name",
            "night",
        ])
        .expect("spawn gpy");
    imported.assert_success("import --name night");
    assert!(
        env.config_dir().join("palettes/night.toml").exists(),
        "the override names the written file"
    );

    let switched = env
        .run_gpy(&["palette", "use", "night"])
        .expect("spawn gpy");
    switched.assert_success("gpy palette use night");
    let shown = env.run_gpy(&["palette", "show"]).expect("spawn gpy");
    assert_eq!(shown.stdout.trim(), "night", "{shown:?}");
}

/// Mirrors `theme_import_tests.rs::import_rejects_traversal_name`.
#[test]
fn import_rejects_a_traversal_name() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let fixture = tokyo_night_fixture();

    let result = env
        .run_gpy(&[
            "palette",
            "import",
            fixture.to_str().unwrap(),
            "--name",
            "../../etc/passwd",
        ])
        .expect("spawn gpy");
    assert_eq!(
        result.exit_code, 1_i32,
        "traversal name must fail: {result:?}"
    );
    assert!(result.stderr.contains("Invalid palette name"), "{result:?}");
    assert!(
        !env.config_dir().join("etc").exists() && !env.root().join("etc").exists(),
        "nothing is written outside the palettes directory"
    );
}
