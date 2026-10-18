//! A theme `gpy theme validate` accepts must render as written through the
//! real `gpy-agent oneshot` path, and one it should reject must fail validation.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

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
        let sequence = rendered.get(index + 1..).unwrap_or_default();
        let params_end = sequence
            .strip_prefix('[')
            .and_then(|rest| rest.find(|c: char| !c.is_ascii_digit() && c != ';'))
            .map(|offset| offset + 1);
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
        validation.exit_code, 0,
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
