//! `fg:prev_bg` resolves the previous segment's background as the span's
//! foreground in the rendered ANSI.

use gpy_agent::config::Config;
use gpy_agent::formatter::{Formatter, PromptFormatter, RenderContext, SegmentPosition};
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

/// `prev_bg = blue` with `fg:prev_bg` renders `\x1b[34m`.
#[test]
fn prev_bg_blue_produces_ansi_blue_fg_in_render() {
    let formatter = PromptFormatter::default();
    let config = Config::default();
    let mut theme = ThemeConfig::default();
    // The powerline-chevron use case.
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
