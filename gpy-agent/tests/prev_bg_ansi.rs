//! Integration test: `prev_bg` threads through the render pipeline and produces
//! the correct ANSI foreground escape code on an `fg:prev_bg`-styled span.
//!
//! Task 3 (threading `prev_bg` through the render pipeline) wired `prev_bg`
//! from the IPC message into the `RenderContext`. This test exercises the
//! *render* end of that pipeline: a custom format string containing
//! `fg:prev_bg` must resolve the previous segment's background colour as the
//! current span's foreground and emit the corresponding ANSI SGR sequence.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(missing_docs)]

use gpy_agent::config::Config;
use gpy_agent::formatter::{FishAnsiFormatter, Formatter, RenderContext, SegmentPosition};
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use gpy_agent::ipc::Response;
use gpy_agent::template::Color;
use gpy_agent::theme::ThemeConfig;

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

/// A `RepositoryStatus` response with `prev_bg = Some(Color::Named("blue"))`
/// and a format that references `fg:prev_bg` must produce `\x1b[34m` (ANSI
/// blue foreground) in the rendered ANSI output.
///
/// This verifies that the `prev_bg` field threaded into `RenderContext` in
/// Task 3 actually reaches the template engine and encodes correctly.
#[test]
fn prev_bg_blue_produces_ansi_blue_fg_in_render() {
    let formatter = FishAnsiFormatter;
    let config = Config::default();
    let mut theme = ThemeConfig::default();
    // Use a format that applies fg:prev_bg so the previous segment's background
    // colour becomes this span's foreground — the powerline-chevron use case.
    theme.segments.git.format = Some("[ ](fg:prev_bg)[$branch]($style)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE)
        .with_prev_colors(None, Some(Color::Named("blue".to_owned())));
    let response = Response::RepositoryStatus(git_clean());
    let got = formatter
        .render(&response, &rc)
        .expect("render should succeed");
    assert!(
        got.contains("\x1b[34m"),
        "expected ANSI blue fg (\\x1b[34m) from prev_bg=blue, got: {got:?}"
    );
}
