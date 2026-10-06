//! Golden ANSI baseline for the default.toml migration (SP2 #199).
//!
//! `capture_default_golden` (ignored) regenerates fixtures from the CURRENT
//! theme using the live formatter. Run it after changing default.toml:
//!   cargo test --test `theme_golden_default` `capture_default_golden` -- --ignored
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
use gpy_agent::palette::PaletteConfig;
use gpy_agent::theme::ThemeConfig;
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn load_default_theme() -> ThemeConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/themes/default.toml");
    gpy_agent::config::loader::load_theme_from_path(&path.display().to_string())
        .expect("load default.toml")
}

/// The shipped default palette, read from the repo like the theme above.
/// `active_palette` would prefer `~/.config/gpy/palettes/default.toml` and
/// render the developer's colors instead of the fixtures' (#664).
fn load_default_palette() -> gpy_agent::template::Palette {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/palettes/default.toml");
    let content = std::fs::read_to_string(&path).expect("read palettes/default.toml");
    toml::from_str::<PaletteConfig>(&content)
        .expect("parse palettes/default.toml")
        .to_template_palette()
}

fn render_response(theme: &ThemeConfig, response: &Response, is_last: bool) -> String {
    let config = Config::default();
    let palette = load_default_palette();
    let position = if is_last {
        SegmentPosition::LAST
    } else {
        SegmentPosition::MIDDLE
    };
    let ctx = RenderContext::new(&config, theme, position).with_palette(palette);
    let formatter = gpy_agent::formatter::FishAnsiFormatter::default();
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
fn all_cases(theme: &ThemeConfig) -> Vec<(&'static str, String)> {
    vec![
        (
            "default_git_clean.ansi",
            render_git(theme, &git_clean(), false),
        ),
        (
            "default_git_dirty.ansi",
            render_git(theme, &git_dirty(), false),
        ),
        (
            "default_git_ahead_behind.ansi",
            render_git(theme, &git_ahead_behind(), false),
        ),
        (
            "default_git_clean_last.ansi",
            render_git(theme, &git_clean(), true),
        ),
        (
            "default_language_rust.ansi",
            render_language(theme, vec![lang("Rust", "1.75.0", "red")], false),
        ),
        (
            "default_language_python.ansi",
            render_language(theme, vec![lang("Python", "3.12.0", "blue")], false),
        ),
        ("default_directory.ansi", render_directory(theme)),
        ("default_duration.ansi", render_duration(theme)),
        ("default_status_ok.ansi", render_status(theme, true)),
        ("default_status_fail.ansi", render_status(theme, false)),
    ]
}

// --- Capture ----------------------------------------------------------------

#[test]
#[ignore = "regenerates golden fixtures from the current theme; run manually after changing default.toml"]
fn capture_default_golden() {
    std::fs::create_dir_all(golden_dir()).expect("mkdir golden");
    let theme = load_default_theme();
    for (file, rendered) in all_cases(&theme) {
        std::fs::write(golden_dir().join(file), rendered).expect("write fixture");
    }
}

// --- Comparison -------------------------------------------------------------

fn assert_case(file: &str, got: &str) {
    let expected = std::fs::read_to_string(golden_dir().join(file))
        .unwrap_or_else(|_| panic!("missing golden {file}; run capture_default_golden --ignored"));
    assert!(
        !expected.is_empty(),
        "golden {file} is empty; the segment produced no template output"
    );
    assert_eq!(
        got, &expected,
        "default.toml render drifted for {file}\n expected: {expected:?}\n      got: {got:?}"
    );
}

#[test]
fn all_cases_match_golden() {
    let theme = load_default_theme();
    for (file, rendered) in all_cases(&theme) {
        assert_case(file, &rendered);
    }
}

// The sep_close glyph (U+E0B4) must be rendered with fg=segment-bg AND
// bg=default (SGR 49) so it is visible against the terminal background.
// Without bg:default the glyph inherits the segment background, making
// fg == bg and rendering the half-circle invisible.
#[test]
fn sep_close_glyph_has_bg_reset_when_last() {
    let theme = load_default_theme();
    // Clean git: bg=green (SGR 32). sep_close must carry fg:32 bg:default(49).
    let output = render_git(&theme, &git_clean(), true);
    assert!(
        output.contains("\x1b[32;49m"),
        "sep_close must include bg:default (SGR 49) so the half-circle is visible; got: {output:?}"
    );
}

// #732: `in_progress_bg_color = "magenta"` must render as SGR 45 (text pill) and
// 35;49 (caps). It used to resolve to a palette self-reference and render uncolored.
#[test]
fn in_progress_git_uses_magenta_background() {
    let theme = load_default_theme();
    let status = RepositoryStatus {
        state: RepositoryState::Rebasing,
        ..git_clean()
    };
    let output = render_git(&theme, &status, true);
    assert!(
        output.contains("37;45m"),
        "in-progress git text must use fg white on bg magenta; got: {output:?}"
    );
    assert!(
        output.contains("35;49m"),
        "in-progress git caps must use fg magenta on default bg; got: {output:?}"
    );
}
