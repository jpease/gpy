//! Snapshot tests for formatter outputs using insta
//!
//! This test suite uses snapshot testing to ensure formatter outputs remain stable
//! across code changes. When formatter output changes intentionally, reviewers can
//! easily see the diff and approve/reject the changes.
//!
//! To review snapshots: cargo insta review
//! To update snapshots: cargo insta accept

#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::Config;
use gpy_agent::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
use gpy_agent::ipc::{LanguageInfo, Response};
use gpy_agent::theme::ThemeConfig;
use insta::{assert_snapshot, with_settings};

/// Helper to create a sample git status response
fn sample_git_status() -> Response {
    Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: gpy_agent::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    })
}

/// Helper to create git status with conflicts
fn sample_git_with_conflicts() -> Response {
    Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "feature/authentication".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 2,
        unstaged: 1,
        untracked: 0,
        conflicts: 3,
        state: gpy_agent::git::RepositoryState::Merging,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    })
}

/// Helper to create git status during rebase
fn sample_git_rebase() -> Response {
    Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "develop".to_owned(),
        ahead: 5,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 2,
        untracked: 1,
        conflicts: 0,
        state: gpy_agent::git::RepositoryState::Rebasing,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    })
}

/// Helper to create language response with multiple languages
fn sample_language_multi() -> Response {
    Response::Language {
        languages: vec![
            LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.75.0".to_owned()),
                color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
            },
            LanguageInfo {
                name: "TypeScript".to_owned(),
                version: Some("5.3.3".to_owned()),
                color: gpy_agent::config::types::ColorSpec::new("#3178c6").unwrap(),
            },
        ],
    }
}

/// Helper to create language response with single language (no version)
fn sample_language_single_no_version() -> Response {
    Response::Language {
        languages: vec![LanguageInfo {
            name: "Python".to_owned(),
            version: None,
            color: gpy_agent::config::types::ColorSpec::new("#3776ab").unwrap(),
        }],
    }
}

/// Helper to create agent status response
fn sample_agent_status() -> Response {
    Response::AgentStatus {
        version: "0.1.0".to_owned(),
        protocol_version: 1,
        watched_repos: 3,
        registered_clients: 5,
        cache_entries: 10,
    }
}

// ============================================================================
// Git Status Snapshots
// ============================================================================

#[test]
fn snapshot_git_json_clean() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_git_status(), &ctx)
        .unwrap();

    with_settings!({description => "Git status (clean) in JSON format"}, {
        assert_snapshot!("git_json_clean", output);
    });
}

#[test]
fn snapshot_git_json_conflicts() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_git_with_conflicts(), &ctx)
        .unwrap();

    with_settings!({description => "Git status (merge conflicts) in JSON format"}, {
        assert_snapshot!("git_json_conflicts", output);
    });
}

#[test]
fn snapshot_git_json_rebase() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_git_rebase(), &ctx)
        .unwrap();

    with_settings!({description => "Git status (rebase) in JSON format"}, {
        assert_snapshot!("git_json_rebase", output);
    });
}

#[test]
fn snapshot_git_fish_args_clean() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Fish)
        .unwrap()
        .render(&sample_git_status(), &ctx)
        .unwrap();

    with_settings!({description => "Git status (clean) in Fish args format"}, {
        assert_snapshot!("git_fish_args_clean", output);
    });
}

#[test]
fn snapshot_git_fish_source_clean() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::FishSource)
        .unwrap()
        .render(&sample_git_status(), &ctx)
        .unwrap();

    with_settings!({description => "Git status (clean) in Fish source format"}, {
        assert_snapshot!("git_fish_source_clean", output);
    });
}

// ============================================================================
// Language Detection Snapshots
// ============================================================================

#[test]
fn snapshot_language_json_multi() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_language_multi(), &ctx)
        .unwrap();

    with_settings!({description => "Multiple languages with versions in JSON format"}, {
        assert_snapshot!("language_json_multi", output);
    });
}

#[test]
fn snapshot_language_json_single_no_version() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_language_single_no_version(), &ctx)
        .unwrap();

    with_settings!({description => "Single language without version in JSON format"}, {
        assert_snapshot!("language_json_single_no_version", output);
    });
}

#[test]
fn snapshot_language_fish_source_multi() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::FishSource)
        .unwrap()
        .render(&sample_language_multi(), &ctx)
        .unwrap();

    with_settings!({description => "Multiple languages in Fish source format"}, {
        assert_snapshot!("language_fish_source_multi", output);
    });
}

// ============================================================================
// Agent Status Snapshots
// ============================================================================

#[test]
fn snapshot_agent_status_json() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&sample_agent_status(), &ctx)
        .unwrap();

    with_settings!({description => "Agent status in JSON format"}, {
        assert_snapshot!("agent_status_json", output);
    });
}

#[test]
fn snapshot_agent_status_fish_source() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let output = create_formatter(Format::FishSource)
        .unwrap()
        .render(&sample_agent_status(), &ctx)
        .unwrap();

    with_settings!({description => "Agent status in Fish source format"}, {
        assert_snapshot!("agent_status_fish_source", output);
    });
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn snapshot_git_long_branch_name() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "feature/very-long-branch-name-that-should-be-handled-gracefully".to_owned(),
        ahead: 100,
        behind: 50,
        ahead_capped: false,
        behind_capped: false,
        staged: 999,
        unstaged: 888,
        untracked: 777,
        conflicts: 0,
        state: gpy_agent::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&response, &ctx)
        .unwrap();

    with_settings!({description => "Git status with long branch name and large counts"}, {
        assert_snapshot!("git_json_long_branch", output);
    });
}

#[test]
fn snapshot_language_special_characters() {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let response = Response::Language {
        languages: vec![LanguageInfo {
            name: "C++".to_owned(),
            version: Some("20".to_owned()),
            color: gpy_agent::config::types::ColorSpec::new("#f34b7d").unwrap(),
        }],
    };

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(&response, &ctx)
        .unwrap();

    with_settings!({description => "Language with special characters (C++)"}, {
        assert_snapshot!("language_json_special_chars", output);
    });
}
