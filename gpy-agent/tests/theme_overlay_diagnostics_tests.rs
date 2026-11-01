//! Overlay-aware listing and diagnostics (ADR-0012): a load error
//! of a self-named overlay is the chain error, `gpy theme list` names an
//! overlay's base, `gpy theme validate` warns about `unset`, and
//! `gpy plugin validate` checks the theme files of a plugin.
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

use cli_harness::{CliCommandResult, CliTestEnv};
use gpy_agent::config::defaults::DEFAULT_THEME_CONTENT;
use std::fs;
use std::path::PathBuf;

fn env() -> CliTestEnv {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"default\"\n").expect("create CLI env");
    fs::create_dir_all(env.themes_dir()).expect("themes dir");
    env
}

fn combined(result: &CliCommandResult) -> String {
    format!("{}{}", result.stdout, result.stderr)
}

fn write_theme(env: &CliTestEnv, name: &str, text: &str) -> PathBuf {
    let path = env.themes_dir().join(format!("{name}.toml"));
    fs::write(&path, text).expect("write theme");
    path
}

#[test]
fn self_named_overlay_load_error_is_the_chain_error_not_the_fork_hint() {
    let env = env();
    let path = write_theme(
        &env,
        "starship",
        "extends = \"starship\"\n\n[segments.username]\nformat = \"$user\"\n",
    );

    let result = env
        .run_gpy(&["theme", "validate", "starship"])
        .expect("run gpy theme validate");
    let output = combined(&result);

    assert_ne!(result.exit_code, 0_i32, "{output}");
    assert!(
        output.contains("segments.username.format")
            && output.contains(&format!("{}:4:1", path.display()))
            && output.contains("chain: starship (user) → starship (builtin)"),
        "the chain error names key, line and chain: {output}"
    );
    assert!(
        !output.contains("older GPY") && !output.contains("gpy theme rebase"),
        "an overlay is not an old fork: {output}"
    );
}

#[test]
fn full_copy_load_error_keeps_the_fork_hint() {
    let env = env();
    let stale = DEFAULT_THEME_CONTENT.replace("[ui]", "[ui]\nprompt_color = \"nonsuch\"");
    write_theme(&env, "default", &stale);

    let result = env
        .run_gpy(&["theme", "validate", "default"])
        .expect("run gpy theme validate");
    let output = combined(&result);

    assert_ne!(result.exit_code, 0_i32, "{output}");
    assert!(output.contains("older GPY"), "{output}");
}

#[test]
fn theme_list_shows_an_overlays_base() {
    let env = env();
    write_theme(
        &env,
        "starship",
        "extends = \"starship\"\n\n[segments.clock]\ntime_format = \"24\"\n",
    );
    write_theme(&env, "mine", "extends = \"starship\"\n");
    write_theme(&env, "gone", "extends = \"nonesuch\"\n");
    write_theme(&env, "plain", DEFAULT_THEME_CONTENT);

    let result = env.run_gpy(&["theme", "list"]).expect("run gpy theme list");
    result.assert_success("gpy theme list");
    let lines: Vec<&str> = result.stdout.lines().map(str::trim).collect();

    for expected in [
        "starship [user, extends builtin starship]",
        "mine [user, extends user starship]",
        "gone [user, overlay, does not load]",
        "plain [user]",
    ] {
        assert!(
            lines.contains(&expected),
            "{expected:?} missing: {}",
            result.stdout
        );
    }
}

#[test]
fn validate_reports_unset_in_a_file_without_extends() {
    let env = env();
    let text = format!("unset = [\"segments.clock\"]\n{DEFAULT_THEME_CONTENT}");
    write_theme(&env, "plain", &text);

    let result = env
        .run_gpy(&["theme", "validate", "plain"])
        .expect("run gpy theme validate");

    result.assert_success("gpy theme validate plain");
    assert!(
        combined(&result).contains("`unset` does nothing in a file without `extends`"),
        "{}",
        combined(&result)
    );
}

#[test]
fn validate_warns_on_unset_paths_the_base_lacks_and_only_those() {
    let env = env();
    write_theme(
        &env,
        "mine",
        "extends = \"starship\"\nunset = [\"segments.character.format\", \"segments.nonesuch.format\"]\n",
    );

    let result = env
        .run_gpy(&["theme", "validate", "mine"])
        .expect("run gpy theme validate");

    result.assert_success("gpy theme validate mine");
    let output = combined(&result);
    assert!(
        output.contains("'segments.nonesuch.format' names a key the base theme lacks"),
        "{output}"
    );
    assert!(!output.contains("segments.character.format'"), "{output}");
}

/// A file plugin `demo-tools` with `themes/<name>.toml` for each of `themes`.
fn install_plugin(env: &CliTestEnv, themes: &[(&str, &str)]) -> PathBuf {
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
    for (name, text) in themes {
        fs::write(root.join("themes").join(format!("{name}.toml")), text).expect("plugin theme");
    }
    root
}

#[test]
fn plugin_validate_accepts_an_overlay_of_a_builtin() {
    let env = env();
    let root = install_plugin(
        &env,
        &[(
            "demo-look",
            "extends = \"starship\"\n[segments.clock]\ntime_format = \"24\"\n",
        )],
    );

    let result = env
        .run_gpy(&["plugin", "validate", root.to_str().expect("utf-8 path")])
        .expect("run gpy plugin validate");

    result.assert_success("gpy plugin validate");
    assert!(
        combined(&result).contains("themes: demo-look"),
        "{}",
        combined(&result)
    );
}

#[test]
fn plugin_validate_rejects_a_plugin_overlay_that_extends_a_non_builtin() {
    let env = env();
    write_theme(&env, "mine", DEFAULT_THEME_CONTENT);
    let root = install_plugin(&env, &[("demo-look", "extends = \"mine\"\n")]);

    let by_path = env
        .run_gpy(&["plugin", "validate", root.to_str().expect("utf-8 path")])
        .expect("run gpy plugin validate");
    let by_id = env
        .run_gpy(&["plugin", "validate", "demo-tools"])
        .expect("run gpy plugin validate");

    for result in [by_path, by_id] {
        let output = combined(&result);
        assert_ne!(result.exit_code, 0_i32, "{output}");
        assert!(
            output.contains("extends 'mine', which is not a builtin theme")
                && output.contains("demo-look.toml"),
            "{output}"
        );
    }
}

#[test]
fn theme_validate_applies_the_plugin_rule_to_a_file_in_a_plugin_themes_dir() {
    let env = env();
    write_theme(&env, "mine", DEFAULT_THEME_CONTENT);
    let root = install_plugin(&env, &[("demo-look", "extends = \"mine\"\n")]);
    let path = root.join("themes/demo-look.toml");

    let result = env
        .run_gpy(&["theme", "validate", path.to_str().expect("utf-8 path")])
        .expect("run gpy theme validate");
    let output = combined(&result);

    assert_ne!(result.exit_code, 0_i32, "{output}");
    assert!(output.contains("which is not a builtin theme"), "{output}");

    // The same file outside a plugin is a user theme and may extend `mine`.
    let outside = write_theme(&env, "elsewhere", "extends = \"mine\"\n");
    env.run_gpy(&["theme", "validate", outside.to_str().expect("utf-8 path")])
        .expect("run gpy theme validate")
        .assert_success("gpy theme validate <user overlay of mine>");
}

#[test]
fn theme_validate_prints_theme_warnings_and_succeeds() {
    let env = env();
    write_theme(
        &env,
        "mine",
        "extends = \"default\"\n\n[segments.gitt]\nbg_color = \"red\"\n\n[segments.clock]\ntime_fromat = \"24\"\n",
    );

    let result = env
        .run_gpy(&["theme", "validate", "mine"])
        .expect("run gpy theme validate");
    let output = combined(&result);

    result.assert_success("gpy theme validate mine");
    assert!(output.contains("segments.gitt"), "{output}");
    assert!(
        output.contains("unknown key `segments.clock.time_fromat` (ignored)"),
        "{output}"
    );
}
