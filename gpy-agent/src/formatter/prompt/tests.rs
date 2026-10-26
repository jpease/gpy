#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use super::*;
use crate::formatter::SegmentPosition;
use crate::theme::ThemeConfig;

fn ctx() -> (Config, ThemeConfig) {
    (Config::default(), ThemeConfig::default())
}

#[test]
fn git_without_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.git.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let status = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    let response = Response::RepositoryStatus(status);
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "no format → empty");
}

#[test]
fn git_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.git.format = Some("on [$branch(green)".to_owned()); // unbalanced
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let status = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    let response = Response::RepositoryStatus(status);
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "broken format → silent empty");
}

#[test]
fn format_some_renders_via_engine() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.git.format = Some("on [$branch](bold green)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let status = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    let response = Response::RepositoryStatus(status);
    let got = formatter.render(&response, &rc).expect("render");
    // "on " is literal; "main" is bold green; one trailing reset.
    assert_eq!(got, "on \x1b[1;32mmain\x1b[0m");
}

#[test]
fn style_indirection_renders_via_engine() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.git.format = Some("[$branch]($style)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let status = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    let response = Response::RepositoryStatus(status);
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains("main"), "expected branch name, got {got:?}");
    assert!(
        got.contains('\u{1b}'),
        "expected an ANSI escape, got {got:?}"
    );
}

fn rust_response() -> Response {
    Response::Language {
        languages: vec![LanguageInfo {
            name: "Rust".to_owned(),
            version: Some("1.75.0".to_owned()),
            color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
        }],
    }
}

#[test]
fn language_without_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.language.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let got = formatter.render(&rust_response(), &rc).expect("render");
    assert_eq!(got, "", "no format → empty");
}

#[test]
fn language_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.language.format = Some("[$symbol(green)".to_owned()); // unbalanced
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let got = formatter.render(&rust_response(), &rc).expect("render");
    assert_eq!(got, "", "broken format → silent empty");
}

#[test]
fn language_format_some_renders_via_engine() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.language.format = Some("[$symbol $version](bold green)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let got = formatter.render(&rust_response(), &rc).expect("render");
    // Bold green wraps the styled group; one trailing reset.
    assert!(got.contains("1.75.0"), "expected version, got {got:?}");
    assert!(
        got.contains("\x1b[1;32m"),
        "expected bold green, got {got:?}"
    );
    assert!(
        got.ends_with("\x1b[0m"),
        "expected trailing reset, got {got:?}"
    );
}

#[test]
fn language_format_caps_at_three() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.language.format = Some("[$version ](green)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
    let mk = |name: &str, ver: &str| LanguageInfo {
        name: name.to_owned(),
        version: Some(ver.to_owned()),
        color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
    };
    let response = Response::Language {
        languages: vec![
            mk("Rust", "1.0.0"),
            mk("Go", "2.0.0"),
            mk("Python", "3.0.0"),
            mk("Node", "4.0.0"),
        ],
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains("1.0.0") && got.contains("3.0.0"));
    assert!(
        !got.contains("4.0.0"),
        "fourth language must be dropped: {got:?}"
    );
}

/// Only the first pill inherits `is_first`, only the last `is_last`.
#[test]
fn language_pills_chain_positions_within_segment() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.language.format = Some(
        "([$sep_open](fg:$bg bg:default))[ $symbol]($style)([ $version]($style))([$sep_gap]($style))([$sep_close](fg:$bg bg:default))"
            .to_owned(),
    );
    let mk = |name: &str, ver: &str| LanguageInfo {
        name: name.to_owned(),
        version: Some(ver.to_owned()),
        color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
    };
    let response = Response::Language {
        languages: vec![
            mk("Rust", "1.0.0"),
            mk("Go", "2.0.0"),
            mk("Python", "3.0.0"),
        ],
    };
    let counts = |pos: SegmentPosition| {
        let rc = RenderContext::new(&config, &theme, pos);
        let got = formatter.render(&response, &rc).expect("render");
        (
            got.matches('\u{e0ba}').count(),
            got.matches('\u{e0bc}').count(),
            got.matches('\u{e0b4}').count(),
        )
    };
    assert_eq!(counts(SegmentPosition::FIRST), (2, 3, 0), "first");
    assert_eq!(counts(SegmentPosition::MIDDLE), (3, 3, 0), "middle");
    assert_eq!(counts(SegmentPosition::LAST), (3, 2, 1), "last");
    assert_eq!(counts(SegmentPosition::ONLY), (2, 2, 1), "only");
}

#[test]
fn directory_format_none_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.directory.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Directory {
        cwd: "/home/user/project".to_owned(),
        read_only: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "no format → agent emits nothing");
}

#[test]
fn error_response_renders_empty() {
    let (config, theme) = ctx();
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Error {
        message: "x".into(),
    };
    for dialect in PromptDialect::ALL {
        let got = PromptFormatter::new(dialect)
            .render(&response, &rc)
            .expect("render");
        assert_eq!(got, "", "{dialect:?}: error must render empty");
    }
}

#[test]
fn non_segment_responses_render_empty() {
    let (config, theme) = ctx();
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let status = Response::AgentStatus {
        version: "0".into(),
        protocol_version: 1,
        watched_repos: 0,
        registered_clients: 0,
        cache_entries: 0,
    };
    for dialect in PromptDialect::ALL {
        for response in [&Response::Ack, &status] {
            let got = PromptFormatter::new(dialect)
                .render(response, &rc)
                .expect("render");
            assert_eq!(got, "", "{dialect:?}: {response:?} must render empty");
        }
    }
}

/// The clock's live-time token reaches each shell unescaped; the literal around it is escaped.
#[test]
fn clock_prompt_token_is_left_for_the_shell_to_expand() {
    use crate::shell::Shell;
    let (config, mut theme) = ctx();
    theme.segments.clock.format = Some(r"[\$ $time](fg:white)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let bash = PromptFormatter::new(PromptDialect::BashPrompt)
        .render(&Response::Clock { shell: Shell::Bash }, &rc)
        .expect("render bash");
    // Each span re-emits its SGR; the token is visible text outside the markers.
    assert!(bash.contains("\\\\$ \\["), "{bash:?}");
    assert!(bash.contains("\\]\\D{%-I:%M %p}\\["), "{bash:?}");

    let zsh = PromptFormatter::new(PromptDialect::ZshPrompt)
        .render(&Response::Clock { shell: Shell::Zsh }, &rc)
        .expect("render zsh");
    assert!(zsh.contains("\\$ %{"), "{zsh:?}");
    assert!(zsh.contains("%}%D{%-I:%M %p}%{"), "{zsh:?}");
}

#[test]
fn directory_text_is_escaped_per_dialect() {
    let (config, mut theme) = ctx();
    theme.segments.directory.format = Some("[$path 100%](fg:green)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Directory {
        cwd: "/w/$(echo X)".to_owned(),
        read_only: false,
    };
    let render = |dialect| {
        PromptFormatter::new(dialect)
            .render(&response, &rc)
            .expect("render")
    };
    let ansi = render(PromptDialect::Ansi);
    assert!(
        ansi.contains("m$(echo X)") && ansi.contains(" 100%"),
        "{ansi:?}"
    );
    let bash = render(PromptDialect::BashPrompt);
    assert!(
        bash.contains("\\]\\\\$(echo X)") && bash.contains(" 100%"),
        "{bash:?}"
    );
    let zsh = render(PromptDialect::ZshPrompt);
    assert!(
        zsh.contains("%}\\$(echo X)") && zsh.contains(" 100%%"),
        "{zsh:?}"
    );
}

#[test]
fn directory_format_some_renders_via_engine() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.directory.format = Some("[$path](bold green)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Directory {
        cwd: "/home/user/project".to_owned(),
        read_only: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(
        got.contains("project") || got.contains("/home/user/project"),
        "got {got:?}"
    );
    assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
    assert!(
        got.ends_with("\x1b[0m"),
        "expected trailing reset, got {got:?}"
    );
}

#[test]
fn directory_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.directory.format = Some("[$path(green)".to_owned()); // unbalanced '['
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Directory {
        cwd: "/tmp".to_owned(),
        read_only: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "broken format → silent fallback → empty");
}

#[test]
fn duration_format_none_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.duration.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Duration {
        duration_ms: 2500_u64,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "no format → agent emits nothing");
}

#[test]
fn duration_format_some_renders_via_engine() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.duration.format = Some("[$duration]($style)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Duration {
        duration_ms: 2500_u64,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains("2.500s"), "expected '2.500s' in {got:?}");
    assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
    assert!(
        got.ends_with("\x1b[0m"),
        "expected trailing reset, got {got:?}"
    );
}

#[test]
fn duration_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.duration.format = Some("[$duration(green)".to_owned()); // unbalanced '['
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Duration {
        duration_ms: 1000_u64,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "broken format → silent fallback → empty");
}

#[test]
fn character_format_none_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.character.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Character { success: true };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "no format → agent emits nothing");
}

#[test]
fn character_format_success_renders_symbol_with_ansi() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.character.format = Some("[$symbol]($style)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Character { success: true };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains('❯'), "expected ❯ in {got:?}");
    assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
    assert!(
        got.contains("\x1b[32m") || got.contains("32"),
        "expected green color, got {got:?}"
    );
    assert!(
        got.ends_with("\x1b[0m"),
        "expected trailing reset, got {got:?}"
    );
}

#[test]
fn character_format_error_renders_red() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.character.format = Some("[$symbol]($style)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Character { success: false };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains('❯'), "expected ❯ in {got:?}");
    assert!(
        got.contains("\x1b[31m") || got.contains("31"),
        "expected red color, got {got:?}"
    );
}

#[test]
fn character_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.character.format = Some("[$symbol(green)".to_owned()); // unbalanced '['
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Character { success: true };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "broken format → silent fallback → empty");
}

#[test]
fn hostname_format_none_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, theme) = ctx();
    assert!(theme.segments.hostname.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Hostname {
        hostname: "host.example.com".to_owned(),
        is_ssh: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "no format → agent emits nothing");
}

#[test]
fn hostname_format_some_renders_trimmed_name_with_ansi() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.hostname.format = Some("on [$symbol$hostname](bold green) ".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Hostname {
        hostname: "host.example.com".to_owned(),
        is_ssh: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    // Default trim_at="." trims the FQDN down to the short name.
    assert!(
        got.contains("host"),
        "expected trimmed hostname, got {got:?}"
    );
    assert!(
        !got.contains("host.example.com"),
        "expected trimming to have applied, got {got:?}"
    );
    assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
    // Reset lands at the group boundary, before the trailing literal " ".
    assert_eq!(got, "on \u{1b}[1;32mhost\u{1b}[0m ");
}

#[test]
fn hostname_icon_renders_only_over_ssh() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.hostname.format = Some("[$symbol$hostname](bold green) ".to_owned());
    theme.segments.hostname.icon = Some(crate::config::types::Icon::new("🌐 ").expect("icon"));
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let render = |is_ssh| {
        let response = Response::Hostname {
            hostname: "host".to_owned(),
            is_ssh,
        };
        formatter.render(&response, &rc).expect("render")
    };
    assert_eq!(render(true), "\u{1b}[1;32m🌐 \u{1b}[1;32mhost\u{1b}[0m ");
    assert_eq!(render(false), "\u{1b}[1;32mhost\u{1b}[0m ");
}

#[test]
fn hostname_malformed_format_emits_empty() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.hostname.format = Some("[$hostname(green)".to_owned()); // unbalanced '['
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let response = Response::Hostname {
        hostname: "host".to_owned(),
        is_ssh: false,
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "", "broken format → silent fallback → empty");
}

/// Root renders `format`; everyone else `user_format`, or `format` when that is unset.
#[test]
fn username_format_follows_root_with_user_format_fallback() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.username.format = Some("[$username](bold red)".to_owned());
    let render = |themed: &ThemeConfig, is_root| {
        let rc = RenderContext::new(&config, themed, SegmentPosition::MIDDLE);
        let response = Response::Username {
            username: "ada".to_owned(),
            is_root,
        };
        formatter.render(&response, &rc).expect("render")
    };
    assert_eq!(render(&theme, false), "\u{1b}[1;31mada\u{1b}[0m");
    theme.segments.username.user_format = Some("[$username](bold yellow)".to_owned());
    assert_eq!(render(&theme, true), "\u{1b}[1;31mada\u{1b}[0m");
    assert_eq!(render(&theme, false), "\u{1b}[1;33mada\u{1b}[0m");
}

#[test]
fn git_format_resolves_through_attached_palette() {
    use crate::template::{Color, Palette};
    use std::collections::HashMap;

    let formatter = PromptFormatter::default();
    let (config, mut theme) = ctx();
    theme.segments.git.format = Some("[$branch](fg:accent)".to_owned());
    let mut map = HashMap::new();
    map.insert(
        "accent".to_owned(),
        Color::Rgb {
            r: 10,
            g: 20,
            b: 30,
        },
    );
    let rc =
        RenderContext::new(&config, &theme, SegmentPosition::LAST).with_palette(Palette::new(map));
    let status = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    let response = Response::RepositoryStatus(status);
    let got = formatter.render(&response, &rc).expect("render");
    assert_eq!(got, "\x1b[38;2;10;20;30mmain\x1b[0m");
}

fn make_lang(name: &str, version: Option<&str>, color: &str) -> crate::ipc::LanguageInfo {
    crate::ipc::LanguageInfo {
        name: name.to_owned(),
        version: version.map(str::to_owned),
        color: crate::config::types::ColorSpec::new(color).expect("valid color spec"),
    }
}

#[test]
fn select_languages_show_versions_true_filters_versionless() {
    let mut config = Config::default();
    config.language.show_versions = true;
    let theme = ThemeConfig::default();

    let langs = vec![
        make_lang("Rust", Some("1.75.0"), "red"),
        make_lang("Nix", None, "blue"), // no version — filtered out when show_versions=true
        make_lang("Python", Some("3.12.0"), "yellow"),
    ];

    let selected = select_languages(&config, &theme, &langs);
    assert_eq!(selected.len(), 2_usize, "only versioned languages selected");
    assert_eq!(selected.first().map(|l| l.name.as_str()), Some("Rust"));
    assert_eq!(selected.get(1).map(|l| l.name.as_str()), Some("Python"));
}

#[test]
fn select_languages_show_versions_false_includes_all() {
    let mut config = Config::default();
    config.language.show_versions = false;
    let theme = ThemeConfig::default();

    let langs = vec![
        make_lang("Rust", Some("1.75.0"), "red"),
        make_lang("Nix", None, "blue"),
    ];

    let selected = select_languages(&config, &theme, &langs);
    assert_eq!(
        selected.len(),
        2_usize,
        "all languages selected regardless of version"
    );
}

#[test]
fn select_languages_theme_show_symbol_without_version_keeps_versionless() {
    // show_symbol_without_version keeps versionless languages despite show_versions.
    let mut config = Config::default();
    config.language.show_versions = true;
    let mut theme = ThemeConfig::default();
    theme.segments.language.show_symbol_without_version = Some(true);

    let langs = vec![
        make_lang("Rust", Some("1.75.0"), "red"),
        make_lang("Fish", None, "green"),
    ];

    let selected = select_languages(&config, &theme, &langs);
    assert_eq!(
        selected.len(),
        2_usize,
        "versionless language kept when theme shows symbol without version"
    );
}

/// `render_or_warn` must neither propagate the error nor panic.
#[test]
fn render_or_warn_takes_warn_and_empty_branch_on_error() {
    let error = crate::template::TemplateError::Unbalanced {
        delimiter: '[',
        position: 3,
    };
    let got = render_or_warn("directory segment", Err(error));
    assert_eq!(
        got, "",
        "an error must still yield an empty segment, matching pre-refactor behavior"
    );
}

#[test]
fn render_or_warn_passes_through_ok_value_unchanged() {
    let got = render_or_warn("directory segment", Ok("rendered".to_owned()));
    assert_eq!(got, "rendered", "Ok(value) must pass through unchanged");
}

/// Every templated `Response` variant with a position-independent payload.
fn separator_bearing_responses() -> Vec<Response> {
    use crate::shell::Shell;
    vec![
        Response::RepositoryStatus(crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }),
        rust_response(),
        Response::Directory {
            cwd: "/home/user/project".to_owned(),
            read_only: false,
        },
        Response::Clock { shell: Shell::Fish },
        Response::Duration { duration_ms: 5000 },
        Response::Character { success: true },
        Response::Hostname {
            hostname: "box".to_owned(),
            is_ssh: false,
        },
        Response::Username {
            username: "ada".to_owned(),
            is_root: false,
        },
    ]
}

/// Default theme with a `$sep_close` character format so every variant can draw caps.
fn default_theme_with_terminal_caps() -> ThemeConfig {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/themes/default.toml");
    let mut theme = crate::config::loader::load_theme_from_path(path).expect("default theme");
    let cap = "([$sep_close](fg:white bg:default))";
    theme.segments.character.format = Some(format!("[$symbol](fg:green){cap}"));
    theme
}

fn has_private_use(text: &str) -> bool {
    text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c))
}

/// `ui.show_icons = false` must emit no Nerd Font caps.
#[test]
fn show_icons_false_renders_no_private_use_glyphs() {
    use crate::formatter::{Format, create_formatter};
    let theme = default_theme_with_terminal_caps();
    let formatter = create_formatter(Format::Ansi);
    let positions = [
        SegmentPosition::FIRST,
        SegmentPosition::MIDDLE,
        SegmentPosition::LAST,
        SegmentPosition::ONLY,
    ];
    for show_icons in [true, false] {
        let mut config = Config::default();
        config.ui.show_icons = show_icons;
        for response in separator_bearing_responses() {
            for position in positions {
                let rc = RenderContext::new(&config, &theme, position);
                let out = formatter.render(&response, &rc).expect("render");
                assert!(
                    !out.is_empty(),
                    "{response:?} at {position:?} must render something"
                );
                if !show_icons {
                    assert!(
                        !has_private_use(&out),
                        "show_icons=false: {response:?} at {position:?} leaked a \
                             private-use glyph: {out:?}"
                    );
                }
            }
            // Control: with icons on, the cap is really drawn at LAST.
            if show_icons {
                let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
                let out = formatter.render(&response, &rc).expect("render");
                assert!(
                    has_private_use(&out),
                    "show_icons=true control: {response:?} at LAST drew no cap: {out:?}"
                );
            }
        }
    }
}

/// The bundled default theme, whose pills all come from agent-side formats.
fn bundled_default_theme() -> ThemeConfig {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/themes/default.toml");
    crate::config::loader::load_theme_from_path(path).expect("default theme")
}

/// Rendered text with the SGR sequences removed.
fn plain_text(ansi: &str) -> String {
    let mut text = String::new();
    let mut chars = ansi.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for escape_char in chars.by_ref() {
                if escape_char == 'm' {
                    break;
                }
            }
        } else {
            text.push(c);
        }
    }
    text
}

const OPEN_CAP: char = '\u{e0ba}';
const CLOSE_CAP: char = '\u{e0bc}';
const LAST_CAP: char = '\u{e0b4}';

/// Plain text of `response` rendered with the default theme at `position`.
fn default_theme_text(
    response: &Response,
    theme: &ThemeConfig,
    position: SegmentPosition,
) -> String {
    let config = Config::default();
    let rc = RenderContext::new(&config, theme, position);
    let ansi = PromptFormatter::default()
        .render(response, &rc)
        .expect("render");
    plain_text(&ansi)
}

/// Hostname, username and status draw the same pill as the clock: one leading
/// space inside it, no opening cap when first, the cap otherwise.
#[test]
fn default_theme_pills_have_one_leading_space_and_position_caps() {
    let theme = bundled_default_theme();
    let ok_icon = theme.segments.status.ok_icon.as_deref().unwrap().to_owned();
    let responses = [
        (
            Response::Hostname {
                hostname: "host.example.com".to_owned(),
                is_ssh: false,
            },
            "host".to_owned(),
        ),
        (
            Response::Username {
                username: "ada".to_owned(),
                is_root: false,
            },
            "ada".to_owned(),
        ),
        (Response::ExitStatus { success: true }, ok_icon),
    ];
    for (response, content) in &responses {
        let at = |position| default_theme_text(response, &theme, position);
        assert_eq!(
            at(SegmentPosition::FIRST),
            format!(" {content} {CLOSE_CAP}"),
            "{response:?} first: no opening cap"
        );
        assert_eq!(
            at(SegmentPosition::MIDDLE),
            format!("{OPEN_CAP} {content} {CLOSE_CAP}"),
            "{response:?} middle: opening cap"
        );
        assert_eq!(
            at(SegmentPosition::LAST),
            format!("{OPEN_CAP} {content}{LAST_CAP}"),
            "{response:?} last: closing half circle"
        );
        assert_eq!(
            at(SegmentPosition::ONLY),
            format!(" {content}{LAST_CAP}"),
            "{response:?} only: no opening cap, closing half circle"
        );
    }
}

/// `$symbol` carries the SSH icon with its own spacing: a local session reads
/// `" host"`, an SSH one `" 🌐 host"`, never a doubled leading space.
#[test]
fn default_theme_hostname_symbol_adds_no_extra_space_locally() {
    let mut theme = bundled_default_theme();
    theme.segments.hostname.icon = Some(crate::config::types::Icon::new("🌐 ").unwrap());
    let at = |is_ssh| {
        default_theme_text(
            &Response::Hostname {
                hostname: "box".to_owned(),
                is_ssh,
            },
            &theme,
            SegmentPosition::ONLY,
        )
    };
    assert_eq!(at(false), format!(" box{LAST_CAP}"));
    assert_eq!(at(true), format!(" 🌐 box{LAST_CAP}"));
}

/// The pill's colors follow the outcome: ok_* on success, fail_* otherwise.
#[test]
fn status_pill_colors_follow_the_exit_outcome() {
    let mut theme = bundled_default_theme();
    theme.segments.status.ok_bg_color = crate::config::types::ColorSpec::new("cyan").unwrap();
    theme.segments.status.fail_bg_color = crate::config::types::ColorSpec::new("magenta").unwrap();
    let config = Config::default();
    let render = |success| {
        let rc = RenderContext::new(&config, &theme, SegmentPosition::ONLY);
        PromptFormatter::default()
            .render(&Response::ExitStatus { success }, &rc)
            .expect("render")
    };
    // 46 = cyan background, 45 = magenta background.
    assert!(render(true).contains("46"), "ok bg: {:?}", render(true));
    assert!(render(false).contains("45"), "fail bg: {:?}", render(false));
}

/// An unset icon falls back to the glyph or the ASCII word, like the shells' local pill.
#[test]
fn status_pill_unset_icon_falls_back_by_show_icons() {
    let mut theme = bundled_default_theme();
    theme.segments.status.ok_icon = None;
    theme.segments.status.fail_icon = None;
    let text = |success, show_icons| {
        let mut config = Config::default();
        config.ui.show_icons = show_icons;
        let rc = RenderContext::new(&config, &theme, SegmentPosition::FIRST);
        plain_text(
            &PromptFormatter::default()
                .render(&Response::ExitStatus { success }, &rc)
                .expect("render"),
        )
    };
    assert_eq!(text(true, true), format!(" ✔ {CLOSE_CAP}"));
    assert_eq!(text(false, true), format!(" ✖ {CLOSE_CAP}"));
    assert_eq!(text(true, false), " ok ");
    assert_eq!(text(false, false), " err ");
}

/// No status `format` → the agent emits nothing and the shell draws the pill.
#[test]
fn status_without_format_emits_empty() {
    let (config, theme) = ctx();
    assert!(theme.segments.status.format.is_none());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let got = PromptFormatter::default()
        .render(&Response::ExitStatus { success: true }, &rc)
        .expect("render");
    assert_eq!(got, "");
}
