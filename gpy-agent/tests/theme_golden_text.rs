//! Golden ANSI baseline for the text.toml migration (SP2 #199).
//!
//! `capture_text_golden` (ignored) regenerates fixtures from the CURRENT
//! theme using the live formatter. Run it after changing text.toml:
//!   cargo test --test `theme_golden_text` `capture_text_golden` -- --ignored
//! The non-ignored tests assert the live (template) render is byte-equal to the
//! committed fixtures, pinning the canonical template output as a regression
//! guard. The fixtures are NOT a legacy-equivalence proof: byte parity with the
//! pre-migration legacy renderer is intentionally not required.
//!
//! Scope note: `git`, `language`, `directory`, `duration`, and `character`
//! (the final prompt symbol) are rendered to ANSI by the agent and are pinned
//! here. `clock` has no agent render path (no `Response` variant), so it cannot
//! be golden-pinned through this harness.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(missing_docs)]

use gpy_agent::config::Config;
use gpy_agent::formatter::{Formatter, RenderContext, SegmentPosition};
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use gpy_agent::ipc::{LanguageInfo, Response};
use gpy_agent::theme::ThemeConfig;
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn load_text_theme() -> ThemeConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/themes/text.toml");
    gpy_agent::config::loader::load_theme_from_path(&path.display().to_string())
        .expect("load text.toml")
}

fn render_response(theme: &ThemeConfig, response: &Response, is_last: bool) -> String {
    let config = Config::default();
    let palette = gpy_agent::palette::active_palette(&config);
    let position = if is_last {
        SegmentPosition::LAST
    } else {
        SegmentPosition::MIDDLE
    };
    let ctx = RenderContext::new(&config, theme, position).with_palette(palette);
    let formatter = gpy_agent::formatter::FishAnsiFormatter;
    formatter.render(response, &ctx).expect("render segment")
}

// --- Per-segment render inputs ----------------------------------------------

fn git_clean() -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
}

fn git_dirty() -> RepositoryStatus {
    RepositoryStatus {
        staged: 1,
        unstaged: 2,
        untracked: 1,
        state: RepositoryState::Dirty,
        ..git_clean()
    }
}

fn git_ahead_behind() -> RepositoryStatus {
    RepositoryStatus {
        ahead: 2,
        behind: 1,
        ..git_clean()
    }
}

fn render_git(theme: &ThemeConfig, status: &RepositoryStatus, is_last: bool) -> String {
    render_response(theme, &Response::RepositoryStatus(status.clone()), is_last)
}

fn lang(name: &str, version: &str, color: &str) -> LanguageInfo {
    LanguageInfo {
        name: name.to_owned(),
        version: Some(version.to_owned()),
        color: gpy_agent::config::types::ColorSpec::new(color).expect("color spec"),
    }
}

fn render_language(theme: &ThemeConfig, languages: Vec<LanguageInfo>, is_last: bool) -> String {
    render_response(theme, &Response::Language { languages }, is_last)
}

fn render_directory(theme: &ThemeConfig) -> String {
    render_response(
        theme,
        &Response::Directory {
            cwd: "/home/user/project".to_owned(),
            read_only: false,
        },
        false,
    )
}

fn render_duration(theme: &ThemeConfig) -> String {
    render_response(theme, &Response::Duration { duration_ms: 1500 }, false)
}

fn render_status(theme: &ThemeConfig, success: bool) -> String {
    render_response(theme, &Response::Character { success }, true)
}

/// `(fixture_name, rendered_bytes)` for every pinned case.
///
/// Table-driven: add a row here to pin a new scenario.
fn all_cases(theme: &ThemeConfig) -> Vec<(&'static str, String)> {
    vec![
        (
            "text_git_clean.ansi",
            render_git(theme, &git_clean(), false),
        ),
        (
            "text_git_dirty.ansi",
            render_git(theme, &git_dirty(), false),
        ),
        (
            "text_git_ahead_behind.ansi",
            render_git(theme, &git_ahead_behind(), false),
        ),
        (
            "text_git_clean_last.ansi",
            render_git(theme, &git_clean(), true),
        ),
        (
            "text_language_rust.ansi",
            render_language(theme, vec![lang("Rust", "1.75.0", "red")], false),
        ),
        (
            "text_language_python.ansi",
            render_language(theme, vec![lang("Python", "3.12.0", "blue")], false),
        ),
        ("text_directory.ansi", render_directory(theme)),
        ("text_duration.ansi", render_duration(theme)),
        ("text_status_ok.ansi", render_status(theme, true)),
        ("text_status_fail.ansi", render_status(theme, false)),
    ]
}

// --- Capture ----------------------------------------------------------------

#[test]
#[ignore = "regenerates golden fixtures from the current theme; run manually after changing text.toml"]
fn capture_text_golden() {
    std::fs::create_dir_all(golden_dir()).expect("mkdir golden");
    let theme = load_text_theme();
    for (file, rendered) in all_cases(&theme) {
        std::fs::write(golden_dir().join(file), rendered).expect("write fixture");
    }
}

// --- Comparison -------------------------------------------------------------

fn assert_case(file: &str, got: &str) {
    let expected = std::fs::read_to_string(golden_dir().join(file))
        .unwrap_or_else(|_| panic!("missing golden {file}; run capture_text_golden --ignored"));
    assert!(
        !expected.is_empty(),
        "golden {file} is empty; the segment produced no template output"
    );
    assert_eq!(
        got, &expected,
        "text.toml render drifted for {file}\n expected: {expected:?}\n      got: {got:?}"
    );
}

#[test]
fn all_cases_match_golden() {
    let theme = load_text_theme();
    for (file, rendered) in all_cases(&theme) {
        assert_case(file, &rendered);
    }
}
