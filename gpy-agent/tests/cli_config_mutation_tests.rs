//! Integration tests for CLI configuration path resolution and malformed-config
//! handling.
//!
//! Covers two June 2026 audit defects:
//! - #179: mutating commands must load and save the *same* active config file,
//!   honoring `GPY_CONFIG_PATH` and an existing local `.gpy.toml`, and must never
//!   create a lower-priority config when a higher-priority one is active.
//! - #182: a load/parse/validation failure must abort a mutation without writing,
//!   leaving the original file byte-for-byte unchanged, and read-only commands
//!   must report the error and exit non-zero instead of printing defaults.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// A malformed TOML document that fails to parse.
const MALFORMED_TOML: &str = "[ui\nthis is invalid toml\n";

/// A document that parses as TOML but fails configuration validation
/// (`skip_paths` entries must be absolute or `~`-relative).
const INVALID_VALIDATION_TOML: &str = "[git]\nskip_paths = [\"relative/path\"]\n";

/// Run the `gpy` binary with a fully isolated environment.
///
/// `HOME`, `XDG_CONFIG_HOME`, and the agent socket are pinned under the test's
/// temp tree so the real user config can never be read or written, and any
/// inherited `GPY_CONFIG_PATH` is cleared unless the test sets one explicitly.
fn run_gpy(
    args: &[&str],
    home: &Path,
    xdg: &Path,
    config_path: Option<&Path>,
    cwd: &Path,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gpy"));
    command
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg)
        .env("XDG_RUNTIME_DIR", home)
        .env("XDG_CACHE_HOME", home)
        .env("GPY_AGENT_SOCKET_PATH", home.join("gpy.sock"))
        .env_remove("GPY_CONFIG_PATH");

    if let Some(path) = config_path {
        command.env("GPY_CONFIG_PATH", path);
    }

    command.output().expect("gpy binary should run")
}

/// Build an isolated `home`/`xdg` pair under a fresh temp dir.
fn isolated_dirs() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path().join("home");
    let xdg = temp.path().join("xdg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&xdg).unwrap();
    (temp, home, xdg)
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// --- #179: active config path resolution ------------------------------------

#[test]
fn gpy_config_path_is_used_for_both_load_and_save() {
    let (temp, home, xdg) = isolated_dirs();
    let custom = temp.path().join("custom.toml");
    fs::write(&custom, "[ui]\ndirectory.max_length = 77\n").unwrap();

    let output = run_gpy(
        &["config", "set", "ui.directory.max_length", "123"],
        &home,
        &xdg,
        Some(&custom),
        temp.path(),
    );

    assert!(
        output.status.success(),
        "config set should succeed; stderr: {}",
        stderr_of(&output)
    );

    let contents = fs::read_to_string(&custom).unwrap();
    assert!(
        contents.contains("123"),
        "GPY_CONFIG_PATH file should hold the new value: {contents}"
    );
    assert!(
        !contents.contains("77"),
        "old value must be overwritten in the active file"
    );
    assert!(
        !xdg.join("gpy").join("config.toml").exists(),
        "no global XDG config may be created when GPY_CONFIG_PATH is active"
    );
}

#[test]
fn local_gpy_toml_is_updated_without_creating_a_global_config() {
    let (temp, home, xdg) = isolated_dirs();
    let workdir = temp.path().join("work");
    fs::create_dir_all(&workdir).unwrap();
    fs::write(
        workdir.join(".gpy.toml"),
        "[ui]\ndirectory.max_length = 50\n",
    )
    .unwrap();

    let output = run_gpy(
        &["config", "set", "ui.directory.max_length", "99"],
        &home,
        &xdg,
        None,
        &workdir,
    );

    assert!(
        output.status.success(),
        "config set should succeed; stderr: {}",
        stderr_of(&output)
    );

    let contents = fs::read_to_string(workdir.join(".gpy.toml")).unwrap();
    assert!(
        contents.contains("99"),
        "local .gpy.toml should be updated: {contents}"
    );
    assert!(
        !xdg.join("gpy").join("config.toml").exists(),
        "must not create an XDG config when a local .gpy.toml is active"
    );
    assert!(
        !home
            .join(".config")
            .join("gpy")
            .join("config.toml")
            .exists(),
        "must not create a HOME config when a local .gpy.toml is active"
    );
}

// --- #182: malformed / invalid config handling ------------------------------

/// Write `body` to the active XDG config path and return its location.
fn write_xdg_config(xdg: &Path, body: &str) -> std::path::PathBuf {
    let config_dir = xdg.join("gpy");
    fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("config.toml");
    fs::write(&config_path, body).unwrap();
    config_path
}

#[test]
fn malformed_config_aborts_mutation_and_preserves_file() {
    let (temp, home, xdg) = isolated_dirs();
    let config_path = write_xdg_config(&xdg, MALFORMED_TOML);

    let output = run_gpy(
        &["config", "set", "ui.directory.max_length", "123"],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "mutation must fail when the active config is malformed"
    );
    assert_eq!(
        fs::read_to_string(&config_path).unwrap(),
        MALFORMED_TOML,
        "malformed config must remain byte-for-byte unchanged"
    );
}

#[test]
fn validation_failure_aborts_mutation_and_preserves_file() {
    let (temp, home, xdg) = isolated_dirs();
    let config_path = write_xdg_config(&xdg, INVALID_VALIDATION_TOML);

    let output = run_gpy(
        &["config", "set", "ui.directory.max_length", "123"],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "mutation must fail when the active config fails validation"
    );
    assert_eq!(
        fs::read_to_string(&config_path).unwrap(),
        INVALID_VALIDATION_TOML,
        "a validation failure must leave the config unchanged"
    );
}

#[test]
fn read_only_get_reports_error_on_malformed_config() {
    let (temp, home, xdg) = isolated_dirs();
    write_xdg_config(&xdg, MALFORMED_TOML);

    let output = run_gpy(
        &["config", "get", "ui.directory.max_length"],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "read-only get must exit non-zero on a malformed config"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("80"),
        "get must not print the default value for an invalid config"
    );
}

// --- #450: `config set ui.theme` must validate existence/parseability -------
//
// `ui.theme` is the only resource (file/directory)-backed settable key:
// `config::metadata`'s `set:` closure for it only runs `ThemeName::new`,
// which checks that the string is a syntactically safe config name (no path
// separators, no control characters) but never touches the filesystem. That
// lets `gpy config set ui.theme <bogus>` write an unusable theme name to
// disk. These tests cover both failure shapes named in #450: a theme name
// that resolves to nothing on disk ("missing"), and a theme name that
// resolves to a file that exists but fails to parse ("malformed") — plus a
// sanity check that a real, valid theme name still works.

/// A theme name that matches no builtin, user, or plugin theme.
const MISSING_THEME_NAME: &str = "this-theme-does-not-exist-450";

/// Malformed TOML for a user theme file (fails TOML syntax parsing).
const MALFORMED_THEME_TOML: &str = "[ui\nthis is not valid toml\n";

/// Judgment call 2: a failed mutation against a fresh, file-less environment must not
/// materialize a config file.
///
/// Run against a completely fresh environment with no config file anywhere on the active-path
/// search order. This is distinct from the "existing file preserved" tests below —
/// `load_active_config` returns `Config::default()` for a missing file, so a bug here would
/// show up as a *newly created* file rather than a mutated one.
#[test]
fn config_set_ui_theme_missing_name_creates_no_config_file() {
    let (temp, home, xdg) = isolated_dirs();
    let config_path = xdg.join("gpy").join("config.toml");
    assert!(
        !config_path.exists(),
        "precondition: no config file exists yet"
    );

    let output = run_gpy(
        &["config", "set", "ui.theme", MISSING_THEME_NAME],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "setting a nonexistent theme must fail; stderr: {}",
        stderr_of(&output)
    );
    assert_eq!(
        output.status.code(),
        Some(1_i32),
        "process must exit with a non-zero status code; stderr: {}",
        stderr_of(&output)
    );
    assert!(
        !config_path.exists(),
        "a failed mutation must not create a config file where none existed"
    );
}

#[test]
fn config_set_ui_theme_missing_name_preserves_existing_config_byte_for_byte() {
    let (temp, home, xdg) = isolated_dirs();
    let starting_contents = "[ui]\ntheme = \"default\"\n";
    let config_path = write_xdg_config(&xdg, starting_contents);
    let original_bytes = fs::read(&config_path).unwrap();

    let output = run_gpy(
        &["config", "set", "ui.theme", MISSING_THEME_NAME],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "setting a nonexistent theme must fail; stderr: {}",
        stderr_of(&output)
    );
    assert_eq!(
        fs::read(&config_path).unwrap(),
        original_bytes,
        "config file must remain byte-for-byte unchanged on a missing theme"
    );
}

#[test]
fn config_set_ui_theme_malformed_theme_file_preserves_config_byte_for_byte() {
    let (temp, home, xdg) = isolated_dirs();
    let starting_contents = "[ui]\ntheme = \"default\"\n";
    let config_path = write_xdg_config(&xdg, starting_contents);
    let original_bytes = fs::read(&config_path).unwrap();

    let themes_dir = xdg.join("gpy").join("themes");
    fs::create_dir_all(&themes_dir).unwrap();
    fs::write(themes_dir.join("badtheme.toml"), MALFORMED_THEME_TOML).unwrap();

    let output = run_gpy(
        &["config", "set", "ui.theme", "badtheme"],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        !output.status.success(),
        "setting a malformed theme must fail; stderr: {}",
        stderr_of(&output)
    );
    assert_eq!(
        output.status.code(),
        Some(1_i32),
        "process must exit with a non-zero status code; stderr: {}",
        stderr_of(&output)
    );
    assert_eq!(
        fs::read(&config_path).unwrap(),
        original_bytes,
        "config file must remain byte-for-byte unchanged on a malformed theme"
    );
}

#[test]
fn config_set_ui_theme_valid_discovered_name_still_succeeds() {
    let (temp, home, xdg) = isolated_dirs();
    let config_path = write_xdg_config(&xdg, "[ui]\ntheme = \"text\"\n");

    let output = run_gpy(
        &["config", "set", "ui.theme", "default"],
        &home,
        &xdg,
        None,
        temp.path(),
    );

    assert!(
        output.status.success(),
        "setting a valid, discoverable theme must still succeed; stderr: {}",
        stderr_of(&output)
    );
    let contents = fs::read_to_string(&config_path).unwrap();
    assert!(
        contents.contains("\"default\""),
        "config should hold the newly set theme: {contents}"
    );
}

#[test]
fn every_mutation_command_family_aborts_on_malformed_config() {
    // One representative invocation per mutation command family.
    let families: &[&[&str]] = &[
        &["config", "set", "ui.directory.max_length", "123"],
        &["theme", "use", "default"],
        &["enable", "git"],
        &["disable", "git"],
        &["lang", "versions", "on"],
    ];

    for args in families {
        let (temp, home, xdg) = isolated_dirs();
        let config_path = write_xdg_config(&xdg, MALFORMED_TOML);

        let output = run_gpy(args, &home, &xdg, None, temp.path());

        assert!(
            !output.status.success(),
            "`gpy {}` must fail on a malformed active config",
            args.join(" ")
        );
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            MALFORMED_TOML,
            "`gpy {}` must not rewrite the malformed config",
            args.join(" ")
        );
    }
}

// --- #730: writers edit the document instead of regenerating it --------------

#[test]
fn config_set_preserves_comments_unknown_keys_and_absent_defaults() {
    let (temp, home, xdg) = isolated_dirs();
    let config_path = write_xdg_config(
        &xdg,
        "# my comment\n[ui]\ncustom_key = 1 # keep\n[git]\ntimeout_seconds = 3\n",
    );

    let output = run_gpy(
        &["config", "set", "git.show_upstream", "false"],
        &home,
        &xdg,
        None,
        temp.path(),
    );
    assert!(
        output.status.success(),
        "config set should succeed; stderr: {}",
        stderr_of(&output)
    );

    let contents = fs::read_to_string(&config_path).unwrap();
    assert!(contents.contains("# my comment"), "{contents}");
    assert!(contents.contains("custom_key = 1 # keep"), "{contents}");
    assert!(contents.contains("timeout_seconds = 3"), "{contents}");
    assert!(contents.contains("show_upstream = false"), "{contents}");
    assert!(!contents.contains("max_ahead_behind"), "{contents}");
    assert!(!contents.contains("Generated automatically"), "{contents}");
}

#[test]
fn config_set_is_deterministic_with_language_icons() {
    let (temp, home, xdg) = isolated_dirs();
    let names: [&str; 10] = [
        "zig", "rust", "go", "python", "node", "java", "elixir", "c", "cpp", "ruby",
    ];
    let seed: String = std::iter::once("[language.icons]\n".to_owned())
        .chain(names.iter().map(|name| format!("{name} = \"i-{name}\"\n")))
        .collect();
    let config_path = write_xdg_config(&xdg, &seed);

    let mut results = Vec::new();
    for _ in 0_u8..2_u8 {
        let output = run_gpy(
            &["config", "set", "git.timeout_seconds", "5"],
            &home,
            &xdg,
            None,
            temp.path(),
        );
        assert!(
            output.status.success(),
            "config set should succeed; stderr: {}",
            stderr_of(&output)
        );
        results.push(fs::read_to_string(&config_path).unwrap());
    }

    let [first, second] = results.as_slice() else {
        panic!("expected two results");
    };
    assert_eq!(first, second, "two identical runs must match");
    let icon_lines: Vec<&str> = first
        .lines()
        .filter(|line| line.contains("= \"i-"))
        .collect();
    let expected: Vec<String> = names
        .iter()
        .map(|name| format!("{name} = \"i-{name}\""))
        .collect();
    assert_eq!(icon_lines, expected, "icon order must be preserved");
}

#[test]
fn theme_use_writes_only_ui_theme() {
    let (temp, home, xdg) = isolated_dirs();

    let output = run_gpy(&["theme", "use", "text"], &home, &xdg, None, temp.path());
    assert!(
        output.status.success(),
        "theme use should succeed; stderr: {}",
        stderr_of(&output)
    );

    let contents = fs::read_to_string(xdg.join("gpy").join("config.toml")).unwrap();
    let table: toml::Table = contents.parse().unwrap();
    let mut expected = toml::Table::new();
    let mut ui = toml::Table::new();
    ui.insert("theme".to_owned(), toml::Value::String("text".to_owned()));
    expected.insert("ui".to_owned(), toml::Value::Table(ui));
    assert_eq!(table, expected, "{contents}");
}

#[cfg(unix)]
#[test]
fn config_set_through_symlink_keeps_symlink() {
    let (temp, home, xdg) = isolated_dirs();
    let target_dir = temp.path().join("dotfiles");
    fs::create_dir_all(&target_dir).unwrap();
    let target = target_dir.join("config.toml");
    fs::write(&target, "# linked\n[git]\ntimeout_seconds = 3\n").unwrap();
    let config_dir = xdg.join("gpy");
    fs::create_dir_all(&config_dir).unwrap();
    let link = config_dir.join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let output = run_gpy(
        &["config", "set", "git.show_upstream", "false"],
        &home,
        &xdg,
        None,
        temp.path(),
    );
    assert!(
        output.status.success(),
        "config set should succeed; stderr: {}",
        stderr_of(&output)
    );

    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    let contents = fs::read_to_string(&target).unwrap();
    assert!(contents.contains("show_upstream = false"), "{contents}");
    assert!(contents.contains("# linked"), "{contents}");
}
