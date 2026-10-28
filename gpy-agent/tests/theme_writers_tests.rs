//! `gpy theme new` and `gpy theme save` write overlays (ADR-0012 section 5,
//! "Writers"); `--full` writes the effective theme as a full file.
//!
//! The loads that compare two written themes run in-process with
//! `XDG_CONFIG_HOME` pointed at the CLI env through [`EnvGuard`], so every
//! test here is `#[serial]`.
#![allow(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure"
)]
#![allow(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure"
)]

#[path = "common/cli_harness.rs"]
mod cli_harness;
#[path = "common/env_guard.rs"]
mod env_guard;

use cli_harness::{CliCommandResult, CliTestEnv};
use env_guard::EnvGuard;
use gpy_agent::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT};
use gpy_agent::config::loader::load_theme_file;
use gpy_agent::theme::normalised_table;
use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};

/// A user overlay of `starship` with one changed key.
const MINE: &str =
    "# my starship\nextends = \"starship\"\n\n[segments.clock]\ntime_format = \"24\"\n";

fn env_with_theme(theme: &str) -> CliTestEnv {
    CliTestEnv::with_config(&format!("[ui]\ntheme = \"{theme}\"\n")).expect("create CLI env")
}

fn write_theme(env: &CliTestEnv, name: &str, text: &str) -> PathBuf {
    let path = env.themes_dir().join(format!("{name}.toml"));
    fs::write(&path, text).expect("write theme");
    path
}

fn gpy(env: &CliTestEnv, args: &[&str]) -> CliCommandResult {
    let result = env.run_gpy(args).expect("run gpy");
    result.assert_success(&format!("gpy {}", args.join(" ")));
    result
}

/// The normalised merged result of the theme file at `path`, loaded in
/// process against `env`'s config root.
fn normalised(env: &CliTestEnv, path: &Path) -> toml::Table {
    let empty_plugins = env.root().join("no-bundled-plugins");
    fs::create_dir_all(&empty_plugins).expect("empty plugin dir");
    let mut guard = EnvGuard::default();
    guard.set("XDG_CONFIG_HOME", env.xdg_config_home());
    guard.set("GPY_BUNDLED_PLUGIN_DIR", &empty_plugins);
    let loaded = load_theme_file(path).expect("theme loads");
    normalised_table(&loaded.theme).expect("theme normalises")
}

#[test]
#[serial]
fn save_on_an_active_user_file_writes_nothing() {
    let env = env_with_theme("default");
    let path = env.themes_dir().join("default.toml");
    let before = fs::read(&path).expect("read active theme");

    for args in [
        &["theme", "save"][..],
        &["theme", "save", "default"][..],
        &["theme", "save", "--full"][..],
    ] {
        let result = gpy(&env, args);
        assert!(
            result.stdout.contains("nothing was written"),
            "{args:?}: {result:?}"
        );
        assert!(!result.stdout.contains("[y/N]"), "{args:?}: {result:?}");
    }

    assert_eq!(fs::read(&path).expect("read active theme"), before);
    assert!(!env.themes_dir().join("default.toml.bak").exists());
}

#[test]
#[serial]
fn save_on_an_active_user_overlay_writes_nothing() {
    let env = env_with_theme("starship");
    let path = write_theme(&env, "starship", MINE);

    let result = gpy(&env, &["theme", "save"]);

    assert!(result.stdout.contains("nothing was written"), "{result:?}");
    assert_eq!(fs::read_to_string(&path).expect("read overlay"), MINE);
}

#[test]
#[serial]
fn full_copy_of_a_full_base_is_its_raw_source_and_loads_like_the_overlay() {
    let env = env_with_theme("default");

    gpy(&env, &["theme", "new", "follows", "--from", "starship"]);
    let full = gpy(
        &env,
        &["theme", "new", "frozen", "--from", "starship", "--full"],
    );

    assert!(
        full.stdout.contains("A full copy of starship (builtin)"),
        "{full:?}"
    );
    let overlay = env.themes_dir().join("follows.toml");
    let frozen = env.themes_dir().join("frozen.toml");
    assert_eq!(
        fs::read_to_string(&overlay).expect("read overlay"),
        "extends = \"starship\"\n"
    );
    assert_eq!(
        fs::read_to_string(&frozen).expect("read full copy"),
        STARSHIP_THEME_CONTENT,
        "a full base is copied verbatim, comments kept"
    );
    assert_eq!(normalised(&env, &frozen), normalised(&env, &overlay));
}

#[test]
#[serial]
fn full_copy_of_an_overlay_is_the_merged_result_and_loads_like_it() {
    let env = env_with_theme("mine");
    let mine = write_theme(&env, "mine", MINE);

    gpy(
        &env,
        &["theme", "new", "frozen", "--from", "mine", "--full"],
    );
    gpy(&env, &["theme", "save", "saved", "--full"]);

    for name in ["frozen", "saved"] {
        let path = env.themes_dir().join(format!("{name}.toml"));
        let text = fs::read_to_string(&path).expect("read full copy");
        let table: toml::Table = text.parse().expect("full copy parses");
        assert!(table.get("extends").is_none(), "{name}: {text}");
        assert!(
            table.contains_key("segments") && table.contains_key("ui"),
            "{name}: {text}"
        );
        assert_eq!(normalised(&env, &path), normalised(&env, &mine), "{name}");
        gpy(&env, &["theme", "validate", name]);
    }
}

#[test]
#[serial]
fn save_names_the_active_theme_as_the_base_of_a_new_overlay() {
    let env = env_with_theme("mine");
    let mine = write_theme(&env, "mine", MINE);

    let result = gpy(&env, &["theme", "save", "other"]);

    assert!(result.stdout.contains("Extends: mine (user)"), "{result:?}");
    assert!(
        result.stdout.contains("Activate with: gpy theme use other"),
        "{result:?}"
    );
    let other = env.themes_dir().join("other.toml");
    assert_eq!(
        fs::read_to_string(&other).expect("read overlay"),
        "extends = \"mine\"\n"
    );
    assert_eq!(normalised(&env, &other), normalised(&env, &mine));
}

#[test]
#[serial]
fn a_write_that_would_close_a_cycle_writes_nothing() {
    let env = env_with_theme("mine");
    write_theme(&env, "mine", MINE);

    let result = env
        .run_gpy(&["theme", "save", "starship"])
        .expect("run gpy theme save");

    assert_ne!(result.exit_code, 0_i32, "{result:?}");
    assert!(
        result.stderr.contains("would not load") && result.stderr.contains("cycle"),
        "{result:?}"
    );
    assert!(!env.themes_dir().join("starship.toml").exists());
}

#[test]
#[serial]
fn new_names_the_plugin_an_overlay_depends_on() {
    let env = env_with_theme("default");
    let root = env.plugins_dir().join("demo-tools");
    fs::create_dir_all(root.join("segments")).expect("segments dir");
    fs::create_dir_all(root.join("themes")).expect("themes dir");
    fs::write(
        root.join("plugin.toml"),
        "id = \"demo-tools\"\nname = \"Demo\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"demo\"]\n",
    )
    .expect("plugin manifest");
    fs::write(root.join("segments/demo.fish"), "# demo\n").expect("segment file");
    fs::write(
        root.join("themes/demo-look.toml"),
        "extends = \"starship\"\n[segments.clock]\ntime_format = \"24\"\n",
    )
    .expect("plugin theme");

    let overlay = gpy(&env, &["theme", "new", "mine", "--from", "demo-look"]);
    assert!(
        overlay
            .stdout
            .contains("Extends: demo-look (plugin:demo-tools)")
            && overlay.stdout.contains("Depends on plugin 'demo-tools'"),
        "{overlay:?}"
    );

    let full = gpy(
        &env,
        &["theme", "new", "frozen", "--from", "demo-look", "--full"],
    );
    assert!(
        !full.stdout.contains("Depends on plugin"),
        "a full copy is independent: {full:?}"
    );
}

#[test]
#[serial]
fn new_without_from_extends_default_and_follows_a_user_default() {
    let env = env_with_theme("default");
    let edited = DEFAULT_THEME_CONTENT.replacen("time_format = \"12\"", "time_format = \"24\"", 1);
    let default = write_theme(&env, "default", &edited);

    gpy(&env, &["theme", "new", "mine"]);

    let mine = env.themes_dir().join("mine.toml");
    assert_eq!(
        fs::read_to_string(&mine).expect("read overlay"),
        "extends = \"default\"\n"
    );
    assert_eq!(normalised(&env, &mine), normalised(&env, &default));
}

/// A theme file managed as a symlink (dotfiles) stays a symlink when a writer
/// replaces it, and the link's target receives the new content.
#[test]
#[serial]
#[cfg(unix)]
fn replacing_a_symlinked_theme_file_writes_through_the_link() {
    let env = env_with_theme("default");
    let dotfiles = env.root().join("dotfiles");
    fs::create_dir_all(&dotfiles).expect("dotfiles dir");
    let target = dotfiles.join("mine.toml");
    fs::write(&target, MINE).expect("write link target");
    let link = env.themes_dir().join("mine.toml");
    std::os::unix::fs::symlink(&target, &link).expect("symlink theme");

    let result = env
        .run_gpy_with_stdin(&["theme", "save", "mine"], "y\n")
        .expect("run gpy theme save");

    result.assert_success("gpy theme save mine");
    assert!(
        link.symlink_metadata()
            .expect("stat link")
            .file_type()
            .is_symlink(),
        "the theme symlink must survive the save"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("read link target"),
        "extends = \"default\"\n"
    );
}
