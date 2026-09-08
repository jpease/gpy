//! Formatter integration tests: exercises `Format::Ansi`, JSON, Fish-args,
//! and Fish-source paths through the Formatter trait.
//!
//! `Format::Ansi` git/language paths render exclusively through the template
//! engine; with no `format` template the agent emits nothing (the legacy
//! `ColorSpec` renderer was removed in SP2 #199). The JSON, Fish-args, and
//! Fish-source formatters test serialization independently of the template
//! engine.

#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::Config;
use gpy_agent::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use gpy_agent::ipc::{LanguageInfo, Response};
use gpy_agent::theme::ThemeConfig;

fn sample_repo_status() -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
}

fn sample_language_infos() -> Vec<LanguageInfo> {
    vec![
        LanguageInfo {
            name: "Rust".to_owned(),
            version: Some("1.75.0".to_owned()),
            color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
        },
        LanguageInfo {
            name: "Go".to_owned(),
            version: Some("1.22.0".to_owned()),
            color: gpy_agent::config::types::ColorSpec::new("#00acd7").unwrap(),
        },
    ]
}

#[test]
fn git_json_matches_legacy() {
    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::Json)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    // Formerly a hand-built `Map` covering 10 of `RepositoryStatus`'s 13
    // fields; it now delegates to `serde_json::to_value`, so `detached`,
    // `rebase_progress`, and `stash_count` are present too (gpy-agent#598).
    let expected = "{\"ahead\":2,\"ahead_capped\":false,\"behind\":1,\"behind_capped\":false,\"branch\":\"main\",\"conflicts\":0,\"detached\":false,\"rebase_progress\":null,\"staged\":3,\"stash_count\":0,\"state\":\"clean\",\"unstaged\":4,\"untracked\":5}";
    assert_eq!(output, expected);
}

#[test]
fn git_fish_args_matches_legacy() {
    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::Fish)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    let expected = "--branch main --ahead 2 --behind 1 --staged 3 --unstaged 4 --untracked 5 --conflicts 0 --state clean";
    assert_eq!(output, expected);
}

#[test]
fn git_fish_source_matches_legacy() {
    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::FishSource)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    let expected = "set -g __gpy_git_branch \"main\"\nset -g __gpy_git_ahead 2\nset -g __gpy_git_behind 1\nset -g __gpy_git_staged 3\nset -g __gpy_git_unstaged 4\nset -g __gpy_git_untracked 5\nset -g __gpy_git_conflicts 0\nset -g __gpy_git_state \"clean\"\n";
    assert_eq!(output, expected);
}

#[test]
fn git_fish_ansi_without_format_emits_empty() {
    // Post-migration contract: the ANSI formatter emits nothing when the git
    // segment has no `format` template (the legacy ColorSpec path was removed).
    let repo = sample_repo_status();
    let config = Config::default();
    let theme = ThemeConfig::default();
    assert!(theme.segments.git.format.is_none());

    let response = Response::RepositoryStatus(repo);
    let output = create_formatter(Format::Ansi)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::LAST),
        )
        .unwrap();

    assert_eq!(output, "", "no format → empty (legacy path removed)");
}

#[test]
fn language_fish_args_matches_legacy() {
    let response = Response::Language {
        languages: sample_language_infos(),
    };

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::Fish)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    assert_eq!(
        output,
        "--lang Rust --version 1.75.0 --color #dea584 --lang Go --version 1.22.0 --color #00acd7"
    );
}

#[test]
fn language_fish_source_matches_legacy() {
    let response = Response::Language {
        languages: sample_language_infos(),
    };

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::FishSource)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    assert_eq!(
        output,
        "set -g __gpy_lang_names \"Rust\" \"Go\"\nset -g __gpy_lang_versions \"1.75.0\" \"1.22.0\"\nset -g __gpy_lang_colors \"#dea584\" \"#00acd7\"\n"
    );
}

#[test]
fn language_fish_ansi_without_format_emits_empty() {
    // Post-migration contract: the ANSI formatter emits nothing when the
    // language segment has no `format` template (legacy path removed).
    let config = Config::default();
    let theme = ThemeConfig::default();
    assert!(theme.segments.language.format.is_none());

    let response = Response::Language {
        languages: sample_language_infos(),
    };
    let output = create_formatter(Format::Ansi)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::LAST),
        )
        .unwrap();

    assert_eq!(output, "", "no format → empty (legacy path removed)");
}

#[test]
fn language_json_retains_versions_when_config_disabled() {
    let mut config = Config::default();
    config.language.show_versions = false;
    let theme = ThemeConfig::default();

    let response = Response::Language {
        languages: sample_language_infos(),
    };

    let output = create_formatter(Format::Json)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    assert!(output.contains("\"version\":\"1.75.0\""));
    assert!(output.contains("\"version\":\"1.22.0\""));
}

#[test]
fn git_zsh_args_matches_legacy() {
    let response = Response::RepositoryStatus(sample_repo_status());

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::Zsh)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    let expected = "--branch main --ahead 2 --behind 1 --staged 3 --unstaged 4 --untracked 5 --conflicts 0 --state clean";
    assert_eq!(output, expected);
}

#[test]
fn language_zsh_args_matches_legacy() {
    let response = Response::Language {
        languages: sample_language_infos(),
    };

    let config = Config::default();
    let theme = ThemeConfig::default();
    let output = create_formatter(Format::Zsh)
        .unwrap()
        .render(
            &response,
            &RenderContext::new(&config, &theme, SegmentPosition::MIDDLE),
        )
        .unwrap();

    assert_eq!(
        output,
        "--lang Rust --version 1.75.0 --color #dea584 --lang Go --version 1.22.0 --color #00acd7"
    );
}

#[test]
fn create_formatter_zsh_is_now_reachable() {
    // Regression test for #360: `Format::Zsh` used to be lumped in with the
    // genuinely-unimplemented `BashSource`/`ZshSource` variants and always
    // errored. `ZshFormatter` is a complete, working implementation, so
    // `create_formatter` must route to it.
    assert!(create_formatter(Format::Zsh).is_ok());
}

#[test]
fn create_formatter_bash_source_and_zsh_source_remain_unimplemented() {
    assert!(create_formatter(Format::BashSource).is_err());
    assert!(create_formatter(Format::ZshSource).is_err());
}

#[test]
fn is_renderable_reflects_create_formatter_support() {
    assert!(Format::Json.is_renderable());
    assert!(Format::Ansi.is_renderable());
    assert!(Format::Fish.is_renderable());
    assert!(Format::FishSource.is_renderable());
    assert!(Format::Zsh.is_renderable());
    assert!(!Format::BashSource.is_renderable());
    assert!(!Format::ZshSource.is_renderable());
}
