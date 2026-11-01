//! `gpy theme rebase` (ADR-0012): a modified full copy becomes an
//! overlay that keeps only what differs from its base.
#![allow(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure"
)]
#![allow(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure"
)]
#![allow(clippy::panic, reason = "test code: a panic is the failure")]

#[path = "common/cli_harness.rs"]
mod cli_harness;

use cli_harness::{CliCommandResult, CliTestEnv};
use gpy_agent::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT};
use std::fs;
use std::path::{Path, PathBuf};

fn write_theme(env: &CliTestEnv, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(env.themes_dir()).expect("create themes dir");
    let path = env.themes_dir().join(format!("{name}.toml"));
    fs::write(&path, content).expect("write theme file");
    path
}

fn run(env: &CliTestEnv, args: &[&str]) -> CliCommandResult {
    let mut full = vec!["theme", "rebase"];
    full.extend_from_slice(args);
    env.run_gpy(&full).expect("run gpy theme rebase")
}

fn read_table(path: &Path) -> toml::Table {
    fs::read_to_string(path)
        .expect("read theme file")
        .parse()
        .expect("theme file parses")
}

fn sub<'a>(table: &'a toml::Table, key: &str) -> &'a toml::Table {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("no table {key} in {table}"))
}

fn keys(table: &toml::Table) -> Vec<&str> {
    let mut names: Vec<&str> = table.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

fn string(text: &str) -> toml::Value {
    toml::Value::String(text.to_owned())
}

/// `content` without the text from `start` up to, not including, `end`.
fn without(content: &str, start: &str, end: &str) -> String {
    let (before, rest) = content.split_once(start).expect("start in fixture");
    let (_, after) = rest.split_once(end).expect("end in fixture");
    format!("{before}{end}{after}")
}

fn validate(env: &CliTestEnv, name: &str) {
    env.run_gpy(&["theme", "validate", name])
        .expect("run gpy theme validate")
        .assert_success(&format!("gpy theme validate {name}"));
}

/// The 0.7 `starship` builtin with a commented 24-hour clock and a changed
/// flat language symbol.
fn modified_starship_copy() -> String {
    let copy = STARSHIP_THEME_CONTENT
        .replacen(
            "time_format = \"12\"\n",
            "# 24-hour clock\ntime_format = \"24\"\n",
            1,
        )
        .replacen("rust_symbol = \"🦀\"\n", "rust_symbol = \"R\"\n", 1);
    assert!(
        copy.contains("time_format = \"24\"") && copy.contains("rust_symbol = \"R\""),
        "fixture edits apply"
    );
    copy
}

#[test]
fn a_modified_starship_copy_keeps_only_its_changes() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let copy = modified_starship_copy();
    let path = write_theme(&env, "starship", &copy);

    let preview = run(&env, &["starship"]);
    preview.assert_success("gpy theme rebase starship");
    for expected in [
        "onto starship (builtin)",
        "segments.clock.time_format, segments.language.rust_symbol",
        "extends = \"starship\"",
        "Nothing written",
    ] {
        assert!(preview.stdout.contains(expected), "{expected}: {preview:?}");
    }
    assert_eq!(fs::read_to_string(&path).expect("read copy"), copy);
    assert!(!env.themes_dir().join("starship.toml.bak").exists());

    run(&env, &["starship", "--write"]).assert_success("gpy theme rebase starship --write");
    let written = read_table(&path);
    assert_eq!(written.get("extends"), Some(&string("starship")));
    assert_eq!(keys(&written), ["extends", "segments"]);
    let segments = sub(&written, "segments");
    assert_eq!(keys(segments), ["clock", "language"]);
    assert_eq!(keys(sub(segments, "clock")), ["time_format"]);
    assert_eq!(
        sub(segments, "clock").get("time_format"),
        Some(&string("24"))
    );
    assert_eq!(
        sub(segments, "language").get("rust_symbol"),
        Some(&string("R")),
        "the flat spelling is kept"
    );
    let text = fs::read_to_string(&path).expect("read overlay");
    assert!(text.contains("# 24-hour clock\ntime_format"), "{text}");
    assert_eq!(
        fs::read_to_string(env.themes_dir().join("starship.toml.bak")).expect("read backup"),
        copy
    );
    validate(&env, "starship");
}

#[test]
fn a_pristine_copy_becomes_a_bare_overlay() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let path = write_theme(&env, "starship", STARSHIP_THEME_CONTENT);

    let output = run(&env, &["starship", "--write"]);

    output.assert_success("gpy theme rebase starship --write");
    assert!(
        output.stdout.contains("No key differs from the base"),
        "{output:?}"
    );
    let written = read_table(&path);
    assert_eq!(keys(&written), ["extends"]);
    assert_eq!(written.get("extends"), Some(&string("starship")));
    validate(&env, "starship");
}

/// A copy the wizard wrote: the whole serialized theme, every default
/// spelled out, language keys nested, and `show_seconds` toggled off where
/// the builtin has none.
#[test]
fn a_wizard_copy_drops_its_defaults_and_nested_language_keys() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let mut theme = gpy_agent::theme::parse(STARSHIP_THEME_CONTENT, "starship").unwrap();
    assert!(
        theme.segments.clock.show_seconds.is_none(),
        "the builtin sets no show_seconds"
    );
    theme.segments.clock.show_seconds = Some(false);
    let copy = toml::to_string_pretty(&theme).expect("serialize like the wizard");
    assert!(copy.contains("show_seconds = false"), "{copy}");
    assert!(copy.contains("[segments.language.symbols]"), "{copy}");
    let path = write_theme(&env, "starship", &copy);

    let output = run(&env, &["starship", "--write"]);

    output.assert_success("gpy theme rebase starship --write");
    assert!(
        output.stdout.contains("No key differs from the base"),
        "{output:?}"
    );
    assert_eq!(keys(&read_table(&path)), ["extends"]);
    validate(&env, "starship");
}

#[test]
fn a_missing_delimiter_table_is_inherited_or_unset_at_its_top() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let copy = without(
        DEFAULT_THEME_CONTENT,
        "[segments.clock.open]\n",
        "[segments.clock.close]\n",
    );
    let path = write_theme(&env, "default", &copy);

    let preview = run(&env, &["default"]);
    preview.assert_success("gpy theme rebase default");
    assert!(
        preview.stdout.contains(
            "Inherited from the base (your copy does not have them): segments.clock.open\n"
        ),
        "{preview:?}"
    );
    assert!(preview.stdout.contains("--unset-missing"), "{preview:?}");
    assert!(!preview.stdout.contains("unset = "), "{preview:?}");

    run(&env, &["default", "--unset-missing", "--write"])
        .assert_success("gpy theme rebase default --unset-missing --write");
    let written = read_table(&path);
    assert_eq!(keys(&written), ["extends", "unset"]);
    assert_eq!(
        written.get("unset"),
        Some(&toml::Value::Array(vec![string("segments.clock.open")]))
    );
    validate(&env, "default");
}

#[test]
fn a_missing_plugin_section_is_unset_as_one_entry() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let copy = without(DEFAULT_THEME_CONTENT, "[segments.hostname]\n", "# Username");
    let path = write_theme(&env, "default", &copy);

    run(&env, &["default", "--unset-missing", "--write"])
        .assert_success("gpy theme rebase default --unset-missing --write");

    let written = read_table(&path);
    assert_eq!(keys(&written), ["extends", "unset"]);
    assert_eq!(
        written.get("unset"),
        Some(&toml::Value::Array(vec![string("segments.hostname")]))
    );
    validate(&env, "default");
}

#[test]
fn a_copy_that_does_not_load_is_refused_with_the_migrate_hint() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let stale = STARSHIP_THEME_CONTENT.replacen(
        "[segments.username]\n",
        "[segments.username]\nformat = \"[$username](bold red) in \"\n",
        1,
    );
    let path = write_theme(&env, "starship", &stale);

    let output = run(&env, &["starship", "--write"]);

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    let combined = format!("{}{}", output.stdout, output.stderr);
    assert!(
        combined.contains("gpy theme migrate starship"),
        "{combined}"
    );
    assert_eq!(fs::read_to_string(&path).expect("read copy"), stale);
    assert!(!env.themes_dir().join("starship.toml.bak").exists());
}

#[test]
fn an_overlay_is_refused() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    write_theme(&env, "starship", "extends = \"starship\"\n");

    let output = run(&env, &["starship"]);

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    assert!(output.stderr.contains("already an overlay"), "{output:?}");
}

#[test]
fn a_copy_with_no_theme_below_needs_onto() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let path = write_theme(&env, "mine", &modified_starship_copy());

    let refused = run(&env, &["mine"]);
    assert_ne!(refused.exit_code, 0_i32, "{refused:?}");
    assert!(
        refused.stderr.contains("--onto starship"),
        "names the closest builtin: {refused:?}"
    );

    run(&env, &["mine", "--onto", "starship", "--write"])
        .assert_success("gpy theme rebase mine --onto starship --write");
    let written = read_table(&path);
    assert_eq!(written.get("extends"), Some(&string("starship")));
    assert_eq!(keys(sub(&written, "segments")), ["clock", "language"]);
    validate(&env, "mine");
}

/// The write goes through the symlink to a file in a directory the command
/// cannot write to: the backup succeeds, the atomic replace fails, and the
/// copy stays as it was.
#[cfg(unix)]
#[test]
fn a_failed_write_leaves_the_copy_in_place() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let env = CliTestEnv::new().expect("create isolated CLI test env");
    fs::create_dir_all(env.themes_dir()).expect("create themes dir");
    let locked = env.root().join("locked");
    fs::create_dir_all(&locked).expect("create locked dir");
    let target = locked.join("starship.toml");
    let copy = modified_starship_copy();
    fs::write(&target, &copy).expect("write copy");
    let link = env.themes_dir().join("starship.toml");
    symlink(&target, &link).expect("link the copy into the themes dir");

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock dir");
    // Root writes through any mode bits, so the failure can't be provoked.
    let unlocked = fs::write(locked.join("probe"), "").is_ok();
    let result = (!unlocked).then(|| run(&env, &["starship", "--write"]));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock dir");
    let Some(output) = result else {
        return;
    };

    assert_ne!(output.exit_code, 0_i32, "{output:?}");
    assert!(output.stderr.contains("write"), "{output:?}");
    assert_eq!(fs::read_to_string(&target).expect("read copy"), copy);
    assert!(
        fs::symlink_metadata(&link)
            .expect("stat link")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn rebase_help_lists_its_options() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let output = env
        .run_gpy(&["theme", "rebase", "--help"])
        .expect("run gpy theme rebase --help");

    output.assert_success("gpy theme rebase --help");
    for option in ["--onto <ONTO>", "--write", "--unset-missing"] {
        assert!(output.stdout.contains(option), "{option}: {output:?}");
    }
}
