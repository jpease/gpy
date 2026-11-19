//! A theme `gpy theme validate` accepts must render as written through the
//! real `gpy-agent oneshot` path, and one it should reject must fail validation.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::fs;
use std::path::PathBuf;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::{CliCommandResult, CliTestEnv};

/// The prompt formats Fish, Bash and Zsh draw from.
const LIVE_PROMPT_FORMATS: [&str; 3] = ["ansi", "bash-prompt", "zsh-prompt"];

/// An isolated env whose active theme is `audit` with `theme_toml` as its body.
fn env_with_theme(theme_toml: &str) -> CliTestEnv {
    let env =
        CliTestEnv::with_config("[ui]\ntheme = \"audit\"\nenabled_segments = [\"directory\"]\n")
            .expect("create CLI env");
    fs::write(env.themes_dir().join("audit.toml"), theme_toml).expect("write theme");
    env
}

/// A directory named `fixture` to render; its name is the directory segment text.
fn fixture_dir(env: &CliTestEnv) -> PathBuf {
    let dir = env.root().join("fixture");
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn validate_audit_theme(env: &CliTestEnv) -> CliCommandResult {
    env.run_gpy(&["theme", "validate", "audit"])
        .expect("run gpy theme validate")
}

fn render_directory(env: &CliTestEnv, format: &str) -> String {
    let cwd = fixture_dir(env);
    let result = env
        .run_gpy_agent(&[
            "oneshot",
            "directory",
            "--cwd",
            cwd.to_str().expect("utf-8 path"),
            "--format",
            format,
        ])
        .expect("run gpy-agent oneshot");
    result.assert_success(&format!("oneshot directory --format {format}"));
    result.stdout
}

/// Every ESC in `rendered` must open an SGR sequence (`ESC [ params m`).
fn assert_only_sgr_escapes(rendered: &str, context: &str) {
    assert!(
        !rendered.contains('\u{7}'),
        "{context}: BEL survived: {rendered:?}"
    );
    for (index, _) in rendered.match_indices('\u{1b}') {
        let sequence = index
            .checked_add(1)
            .and_then(|start| rendered.get(start..))
            .unwrap_or_default();
        let params_end = sequence
            .strip_prefix('[')
            .and_then(|rest| rest.find(|c: char| !c.is_ascii_digit() && c != ';'))
            .and_then(|offset| offset.checked_add(1));
        assert!(
            params_end.is_some_and(|end| sequence
                .get(end..)
                .is_some_and(|tail| tail.starts_with('m'))),
            "{context}: non-SGR escape survived: {rendered:?}"
        );
    }
}

#[test]
fn theme_literal_terminal_controls_fail_validation_and_never_render() {
    let env = env_with_theme(
        "[ui]\n[segments.directory]\nformat = \"\\u001b]0;GPY-AUDIT\\u0007$path\"\n",
    );

    let validation = validate_audit_theme(&env);
    assert_ne!(
        validation.exit_code, 0_i32,
        "theme with ESC/BEL in its format must fail validation\nstdout:\n{}\nstderr:\n{}",
        validation.stdout, validation.stderr
    );
    let report = format!("{}{}", validation.stdout, validation.stderr);
    assert!(
        report.contains("directory"),
        "validation must name the offending segment: {report}"
    );
    assert!(
        !report.contains('\u{1b}') && !report.contains('\u{7}'),
        "validation output must not replay the controls: {report:?}"
    );

    for format in LIVE_PROMPT_FORMATS {
        let rendered = render_directory(&env, format);
        assert!(
            !rendered.contains('\u{1b}') && !rendered.contains('\u{7}'),
            "--format {format}: control bytes survived: {rendered:?}"
        );
        assert!(
            rendered.contains("fixture"),
            "--format {format}: directory text missing: {rendered:?}"
        );
    }
}

#[test]
fn theme_styling_and_newline_layout_still_render() {
    let env = env_with_theme(
        "[ui]\n[segments.directory]\nformat = \"[$path](bold fg:#112233)\\n[>](fg:red)\"\n",
    );

    validate_audit_theme(&env).assert_success("gpy theme validate audit");

    for format in LIVE_PROMPT_FORMATS {
        let rendered = render_directory(&env, format);
        assert!(
            rendered.contains("38;2;17;34;51"),
            "--format {format}: RGB style missing: {rendered:?}"
        );
        let path_at = rendered.find("fixture").expect("directory text rendered");
        let arrow_at = rendered.find('>').expect("second line rendered");
        let between = rendered.get(path_at..arrow_at).unwrap_or_default();
        assert!(
            between.contains('\n'),
            "--format {format}: newline layout lost: {rendered:?}"
        );
        assert_only_sgr_escapes(&rendered, &format!("--format {format}"));
    }
}

/// A theme whose directory format renders `text_color`, which the clock
/// pill `gpy prompt` draws itself also uses.
fn color_theme(text_color: &str) -> String {
    format!(
        "[ui]\n[segments.directory]\nformat = \"[$path]($style)\"\ntext_color = \"{text_color}\"\n\
         [segments.clock]\ntext_color = \"{text_color}\"\n"
    )
}

/// The Bash prompt `gpy prompt bash` draws with only the clock enabled.
fn bash_clock_prompt(env: &CliTestEnv) -> String {
    env.write_config("[ui]\ntheme = \"audit\"\nenabled_segments = [\"clock\"]\n")
        .expect("write config");
    let result = env
        .run_gpy_with_env(
            &["prompt", "bash", "--pid", "1"],
            &[("GPY_AGENT_ENABLED", "0"), ("TERM", "xterm-256color")],
        )
        .expect("run gpy prompt bash");
    result.assert_success("gpy prompt bash");
    result.stdout
}

#[test]
fn repeated_hash_hex_color_fails_validation() {
    let env = env_with_theme(&color_theme("##112233"));
    let validation = validate_audit_theme(&env);
    assert_ne!(
        validation.exit_code, 0_i32,
        "'##112233' must fail validation\nstdout:\n{}\nstderr:\n{}",
        validation.stdout, validation.stderr
    );
}

#[test]
fn parser_color_aliases_validate_and_render_bright_magenta_everywhere() {
    for spelling in ["bright-magenta", "brpurple", "bright_purple", "BRMAGENTA"] {
        let env = env_with_theme(&color_theme(spelling));
        validate_audit_theme(&env).assert_success(&format!("gpy theme validate ({spelling})"));

        let rendered = render_directory(&env, "ansi");
        assert!(
            rendered.contains("\u{1b}[95;") && rendered.contains("fixture"),
            "'{spelling}': directory not drawn in bright magenta: {rendered:?}"
        );

        let prompt = bash_clock_prompt(&env);
        assert!(
            prompt.contains("\u{1b}[95"),
            "'{spelling}': the clock is not drawn in bright magenta: {prompt:?}"
        );
    }
}

#[test]
fn palette_with_aliased_keys_loads_warns_and_renders_documented_winner() {
    let env = CliTestEnv::with_config(
        "[ui]\ntheme = \"audit\"\npalette = \"dup\"\nenabled_segments = [\"directory\"]\n",
    )
    .expect("create CLI env");
    fs::write(
        env.themes_dir().join("audit.toml"),
        "[ui]\n[segments.directory]\nformat = \"[$path](fg:purple)[!](fg:bright-purple)\"\n",
    )
    .expect("write theme");
    let palettes = env.config_dir().join("palettes");
    fs::create_dir_all(&palettes).expect("create palettes dir");
    fs::write(
        palettes.join("dup.toml"),
        "name = \"dup\"\n[colors]\nmagenta = \"#111111\"\npurple = \"#222222\"\n\
         bright_magenta = \"#333333\"\nbrmagenta = \"#444444\"\n",
    )
    .expect("write palette");

    let validation = env
        .run_gpy(&["palette", "validate", "dup"])
        .expect("run gpy palette validate");
    validation.assert_success("gpy palette validate dup");
    assert!(
        validation.stderr.contains("'magenta'") && validation.stderr.contains("'purple'"),
        "aliased keys must be reported as a warning:\n{}",
        validation.stderr
    );

    let rendered = render_directory(&env, "ansi");
    // `purple` is a key: its exact entry. `bright-purple` is not: the alphabetically first key.
    assert!(
        rendered.contains("38;2;34;34;34") && rendered.contains("38;2;51;51;51"),
        "palette aliases rendered wrong colors: {rendered:?}"
    );
}
