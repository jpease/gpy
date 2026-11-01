use super::*;

fn precedence_status() -> RepositoryStatus {
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

#[test]
fn render_segment_git_uses_state_precedence() {
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let mut theme = ThemeConfig::default();
    theme.segments.git.format = Some("[$branch]($style)".to_owned());

    let mut status = precedence_status();
    // Other non-zero fields prove precedence, not just "conflicts != 0".
    status.conflicts = 1;
    status.state = RepositoryState::Rebasing;
    status.staged = 1;
    status.untracked = 1;

    let facts = PreviewFacts {
        repository_status: Some(status.clone()),
        ..make_facts()
    };

    assert_eq!(GitState::from_status(&status), GitState::Conflicts);

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let spans = render_segment("git", &config, &theme, &facts, &ctx)
        .expect("git segment should render when format + status are present");
    assert!(
        spans.iter().any(|s| s.text.contains("main")),
        "expected branch text in rendered spans, got {spans:?}"
    );
}

fn make_lang(name: &str, version: Option<&str>, color: &str) -> crate::ipc::LanguageInfo {
    crate::ipc::LanguageInfo {
        name: name.to_owned(),
        version: version.map(str::to_owned),
        color: crate::config::types::ColorSpec::new(color).expect("valid color spec"),
    }
}

#[test]
fn render_segment_language_uses_selected_display_symbol() {
    let mut config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: true,
        },
    );
    config.language.display = crate::config::types::LanguageDisplay::Text;
    let mut theme = ThemeConfig::default();
    theme.segments.language.format = Some("[$symbol]($style)".to_owned());

    let facts = PreviewFacts {
        languages: vec![make_lang("Rust", Some("1.75.0"), "red")],
        ..make_facts()
    };

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let spans = render_segment("language", &config, &theme, &facts, &ctx)
        .expect("language segment should render when format + a language are present");

    assert!(
        spans.iter().any(|s| s.text.contains("Rust")),
        "text display mode should render the language name, got {spans:?}"
    );
}

#[test]
fn render_segment_language_draws_every_selected_language() {
    let mut config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: true,
        },
    );
    config.language.display = crate::config::types::LanguageDisplay::Text;
    let mut theme = ThemeConfig::default();
    theme.segments.language.format = Some("[$symbol]($style)".to_owned());

    let facts = PreviewFacts {
        languages: vec![
            make_lang("Rust", Some("1.75.0"), "red"),
            make_lang("Node", Some("20.0.0"), "green"),
        ],
        ..make_facts()
    };

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let spans = render_segment("language", &config, &theme, &facts, &ctx)
        .expect("language segment should render");
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.contains("Rust") && text.contains("Node"),
        "preview must draw both selected languages like the prompt, got {text:?}"
    );
}

#[test]
fn render_segment_language_drops_versionless_language_when_show_versions_on() {
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: true,
        },
    );
    assert!(
        config.language.show_versions,
        "test relies on show_versions defaulting to true"
    );
    let mut theme = ThemeConfig::default();
    theme.segments.language.format = Some("[$symbol]($style)".to_owned());

    let facts = PreviewFacts {
        languages: vec![make_lang("Nix", None, "blue")],
        ..make_facts()
    };

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let result = render_segment("language", &config, &theme, &facts, &ctx);

    assert!(
        result.is_none(),
        "show_versions=true must drop a versionless language from the preview, matching the real prompt's select_languages"
    );
}

#[test]
fn render_segment_language_keeps_versionless_language_when_show_versions_off() {
    let mut config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: true,
        },
    );
    config.language.show_versions = false;
    let mut theme = ThemeConfig::default();
    theme.segments.language.format = Some("[$symbol]($style)".to_owned());

    let facts = PreviewFacts {
        languages: vec![make_lang("Nix", None, "blue")],
        ..make_facts()
    };

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let spans = render_segment("language", &config, &theme, &facts, &ctx);

    assert!(
        spans.is_some(),
        "show_versions=false must keep a versionless language in the preview"
    );
}

#[test]
fn render_segment_language_hides_version_text_when_show_versions_off() {
    // Must hide even a detected version; suppression is in the resolver.
    let mut config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: true,
        },
    );
    config.language.show_versions = false;
    let mut theme = ThemeConfig::default();
    theme.segments.language.format = Some("[$symbol $version]($style)".to_owned());

    let facts = PreviewFacts {
        languages: vec![make_lang("Rust", Some("1.75.0"), "red")],
        ..make_facts()
    };

    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
    let spans = render_segment("language", &config, &theme, &facts, &ctx)
        .expect("a versioned language with show_versions=false should still render");

    assert!(
        spans.iter().all(|s| !s.text.contains("1.75.0")),
        "show_versions=false must hide version text even when a version was detected, got {spans:?}"
    );
}

#[test]
fn render_segment_returns_none_when_no_format_configured() {
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let theme = ThemeConfig::default();
    assert!(theme.segments.directory.format.is_none());

    let facts = make_facts();
    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);

    let result = render_segment("directory", &config, &theme, &facts, &ctx);

    assert!(
        result.is_none(),
        "no configured format must yield None, not empty spans"
    );
}

#[test]
fn render_preview_line_threads_prev_colors_between_segments() {
    let config = make_config(
        &["duration"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format = Some("[$duration](bg:#010101)".to_owned());
    // Character fg uses `prev_bg`, proving prev-color threading reaches the preview.
    theme.segments.character.format = Some("[$symbol](fg:prev_bg)".to_owned());

    let palette = Palette::default();
    let facts = make_facts();

    let lines = render_preview_line(&state, &config, &theme, &palette, &facts);

    let fg_colors: Vec<Option<RatatuiColor>> = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|s| s.style.fg)
        .collect();
    assert!(
        fg_colors.contains(&Some(RatatuiColor::Rgb(1, 1, 1))),
        "expected the character line's fg to inherit the duration segment's \
             bg (rgb(1,1,1)) via prev_bg threading, got {fg_colors:?}"
    );
}

#[test]
fn render_preview_line_skips_disabled_segments() {
    let config = make_config(
        &["duration"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let mut state = test_state(config.clone());
    state.toggle_segment("duration"); // starts enabled, toggle off

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format = Some("[$duration]($style)".to_owned());

    let palette = Palette::default();
    let facts = make_facts();

    let lines = render_preview_line(&state, &config, &theme, &palette, &facts);

    let text: String = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        !text.contains("128ms") && text.is_empty(),
        "disabled duration segment must not appear in the composed line, got {text:?}"
    );
}

#[test]
fn render_preview_line_follows_enabled_segments_order() {
    let config = make_config(
        &["directory", "clock"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.directory.format = Some("[DIRMARK]".to_owned());

    let lines = render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());
    let text: String = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    let dir = text.find("DIRMARK");
    let clock = text.find("9:41");
    assert!(
        dir.is_some() && clock.is_some() && dir < clock,
        "directory must precede clock, got {text:?}"
    );
}

#[test]
fn render_preview_line_only_last_chain_segment_gets_closing_cap() {
    // Same trailing `$sep_close` shape as the real default theme.
    let config = make_config(
        &["duration", "directory"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format =
        Some("[$duration]($style)([$sep_close](fg:black bg:default))".to_owned());
    theme.segments.directory.format =
        Some("[$path]($style)([$sep_close](fg:black bg:default))".to_owned());

    let palette = Palette::default();
    let facts = make_facts();

    let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
    assert_eq!(lines.len(), 1, "no status segment enabled, expect one line");

    let text: String = lines
        .first()
        .expect("asserted above: exactly one line")
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    let duration_idx = text.find('\u{e0bc}').expect(
        "duration (not last in BUILTIN_ORDER) should render the continuation \
             triangle, not the closing cap",
    );
    let directory_idx = text.find('\u{e0b4}').expect(
        "directory (last enabled chain segment) should render the closing \
             half-circle cap",
    );
    assert!(
        duration_idx < directory_idx,
        "continuation glyph must precede the closing-cap glyph, got {text:?}"
    );
    assert!(
        !text.contains("duration\u{e0b4}") && text.matches('\u{e0b4}').count() == 1,
        "only the last chain segment may emit the closing cap, got {text:?}"
    );
}

#[test]
fn render_preview_line_only_first_chain_segment_suppresses_opening_cap() {
    let config = make_config(
        &["duration", "directory"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format =
        Some("([$sep_open](fg:black bg:default))[$duration]($style)".to_owned());
    theme.segments.directory.format =
        Some("([$sep_open](fg:black bg:default))[$path]($style)".to_owned());

    let palette = Palette::default();
    let facts = make_facts();

    let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
    assert_eq!(lines.len(), 1, "no status segment enabled, expect one line");

    let text: String = lines
        .first()
        .expect("asserted above: exactly one line")
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(
        text.matches('\u{e0ba}').count(),
        1,
        "only the non-first chain segment may emit the opening cap, got {text:?}"
    );
    // Default Basename display renders SAMPLE_CWD as "gpy".
    let path_idx = text.find("gpy").expect("directory path text present");
    let cap_idx = text.find('\u{e0ba}').expect("opening cap glyph present");
    assert!(
        cap_idx < path_idx,
        "directory (not first) should render its opening cap before its own \
             content, got {text:?}"
    );
}

#[test]
fn render_preview_line_first_segment_position_not_identity_determines_cap() {
    // Clock renders first here, so duration must keep its opening cap: the
    // suppression is by chain position, not segment identity.
    let config = make_config(
        &["clock", "duration", "directory"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format =
        Some("([$sep_open](fg:black bg:default))[$duration]($style)".to_owned());
    theme.segments.directory.format =
        Some("([$sep_open](fg:black bg:default))[$path]($style)".to_owned());

    let palette = Palette::default();
    let facts = make_facts();

    let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
    let text: String = lines
        .first()
        .expect("render_preview_line always produces at least the chain line")
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(
        text.matches('\u{e0ba}').count(),
        2,
        "with clock occupying the first chain slot, both duration and \
             directory are non-first and must render their opening caps, got {text:?}"
    );
}
