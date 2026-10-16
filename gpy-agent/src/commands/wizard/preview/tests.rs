#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

use super::*;
use crate::config::types::{Icon, PaletteName, ThemeName};
use crate::git::{RepositoryState, RepositoryStatus};
use crate::template::{Palette, Style as TStyle};

fn make_facts() -> PreviewFacts {
    PreviewFacts {
        repository_status: None,
        languages: Vec::new(),
        hostname: "host.example.com".to_owned(),
        username: "tester".to_owned(),
        sample_duration_ms: 128,
        sample_character_success: true,
        sample_is_ssh: true,
    }
}

/// Avoids two bool params (`max-fn-params-bools`).
#[derive(Clone, Copy)]
struct GitLanguageFlags {
    git: bool,
    language: bool,
}

fn make_config(enabled_segments: &[&str], flags: GitLanguageFlags) -> Config {
    let mut config = Config::default();
    config.ui.theme = ThemeName::new("default".to_owned()).expect("valid theme name");
    config.ui.palette = PaletteName::new("default".to_owned()).expect("valid palette name");
    config.git.enabled = flags.git;
    config.language.enabled = flags.language;
    config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
    config
}

/// Mirrors `state` tests' fixed lists; segment order matters for chain-position tests.
fn test_state(config: Config) -> WizardState {
    WizardState::from_parts(
        config,
        vec![
            "default".to_owned(),
            "starship".to_owned(),
            "text".to_owned(),
        ],
        vec![
            "catppuccin-frappe".to_owned(),
            "catppuccin-latte".to_owned(),
            "catppuccin-macchiato".to_owned(),
            "catppuccin-mocha".to_owned(),
            "default".to_owned(),
            "gruvbox-dark-medium".to_owned(),
            "nord".to_owned(),
            "starship".to_owned(),
        ],
        vec![
            "clock".to_owned(),
            "duration".to_owned(),
            "language".to_owned(),
            "directory".to_owned(),
            "git".to_owned(),
            "status".to_owned(),
        ],
    )
}

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
fn to_ratatui_color_maps_named_variant() {
    assert_eq!(
        to_ratatui_color(&TemplateColor::Named("green".to_owned())),
        Some(RatatuiColor::Green)
    );
}

#[test]
fn to_ratatui_color_maps_rgb_variant() {
    assert_eq!(
        to_ratatui_color(&TemplateColor::Rgb {
            r: 10,
            g: 20,
            b: 30
        }),
        Some(RatatuiColor::Rgb(10, 20, 30))
    );
}

#[test]
fn to_ratatui_color_maps_ansi256_variant() {
    assert_eq!(
        to_ratatui_color(&TemplateColor::Ansi256(42)),
        Some(RatatuiColor::Indexed(42))
    );
}

#[test]
fn to_ratatui_color_treats_palette_variant_as_unreachable_none() {
    assert_eq!(
        to_ratatui_color(&TemplateColor::Palette("accent".to_owned())),
        None
    );
}

#[test]
fn to_ratatui_color_treats_prev_fg_variant_as_unreachable_none() {
    assert_eq!(to_ratatui_color(&TemplateColor::PrevFg), None);
}

#[test]
fn to_ratatui_color_treats_prev_bg_variant_as_unreachable_none() {
    assert_eq!(to_ratatui_color(&TemplateColor::PrevBg), None);
}

#[test]
fn named_to_ratatui_white_is_gray_not_bright() {
    // gpy "white" is ANSI 37, which ratatui calls `Gray`.
    assert_eq!(named_to_ratatui("white"), Some(RatatuiColor::Gray));
}

#[test]
fn named_to_ratatui_bright_white_is_white() {
    assert_eq!(named_to_ratatui("bright-white"), Some(RatatuiColor::White));
}

#[test]
fn named_to_ratatui_default_resets() {
    assert_eq!(named_to_ratatui("default"), Some(RatatuiColor::Reset));
}

#[test]
fn named_to_ratatui_unknown_name_is_none() {
    assert_eq!(named_to_ratatui("not-a-real-color"), None);
}

#[test]
fn to_ratatui_modifier_maps_bold() {
    assert_eq!(to_ratatui_modifier(Attr::Bold), Modifier::BOLD);
}

#[test]
fn to_ratatui_modifier_maps_italic() {
    assert_eq!(to_ratatui_modifier(Attr::Italic), Modifier::ITALIC);
}

#[test]
fn to_ratatui_modifier_maps_underline() {
    assert_eq!(to_ratatui_modifier(Attr::Underline), Modifier::UNDERLINED);
}

#[test]
fn to_ratatui_modifier_maps_dimmed() {
    assert_eq!(to_ratatui_modifier(Attr::Dimmed), Modifier::DIM);
}

#[test]
fn to_ratatui_modifier_maps_inverted() {
    assert_eq!(to_ratatui_modifier(Attr::Inverted), Modifier::REVERSED);
}

#[test]
fn to_ratatui_modifier_maps_blink() {
    assert_eq!(to_ratatui_modifier(Attr::Blink), Modifier::SLOW_BLINK);
}

#[test]
fn to_ratatui_modifier_maps_hidden() {
    assert_eq!(to_ratatui_modifier(Attr::Hidden), Modifier::HIDDEN);
}

#[test]
fn to_ratatui_modifier_maps_strikethrough() {
    assert_eq!(
        to_ratatui_modifier(Attr::Strikethrough),
        Modifier::CROSSED_OUT
    );
}

#[test]
fn to_ratatui_span_preserves_text_and_style() {
    let span = TemplateSpan {
        text: "main".to_owned(),
        style: TStyle {
            fg: Some(TemplateColor::Named("green".to_owned())),
            bg: Some(TemplateColor::Rgb { r: 1, g: 2, b: 3 }),
            attrs: vec![Attr::Bold],
        },
        kind: SpanKind::Text,
    };

    let got = to_ratatui_span(&span);

    assert_eq!(got.content, "main");
    assert_eq!(got.style.fg, Some(RatatuiColor::Green));
    assert_eq!(got.style.bg, Some(RatatuiColor::Rgb(1, 2, 3)));
    assert!(got.style.add_modifier.contains(Modifier::BOLD));
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

#[test]
fn render_segment_clock_renders_fixed_demo_time() {
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let theme = ThemeConfig::default();
    let facts = make_facts();
    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let spans = render_segment("clock", &config, &theme, &facts, &ctx)
        .expect("clock should always render a demo, unlike hostname/username with no format");
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert!(
        text.contains("9:41"),
        "expected the fixed demo time in clock output, got {text:?}"
    );
}

/// The SSH-only hostname icon shows with the SSH sample, not without it.
#[test]
fn render_segment_hostname_icon_follows_sample_ssh_state() {
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let mut theme = ThemeConfig::default();
    theme.segments.hostname.format = Some("[$symbol$hostname](green)".to_owned());
    theme.segments.hostname.icon = Some(Icon::new("🌐 ").expect("valid icon"));
    let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
    let text_for = |facts: &PreviewFacts| -> String {
        render_segment("hostname", &config, &theme, facts, &ctx)
            .expect("hostname with a format renders")
            .iter()
            .map(|s| s.text.as_str())
            .collect()
    };

    assert_eq!(text_for(&make_facts()), "🌐 host");
    let local = PreviewFacts {
        sample_is_ssh: false,
        ..make_facts()
    };
    assert_eq!(text_for(&local), "host");
}

fn delimiter(icon: &str, icon_color: &str, bg_color: &str) -> crate::config::DelimiterConfig {
    crate::config::DelimiterConfig {
        icon: crate::config::types::Icon::new(icon).expect("valid icon"),
        icon_color: crate::config::types::ColorSpec::new(icon_color).expect("valid icon_color"),
        bg_color: crate::config::types::ColorSpec::new(bg_color).expect("valid bg_color"),
    }
}

#[test]
fn clock_demo_spans_omits_cap_when_delimiter_icon_empty() {
    // Like the `starship` preset: empty delimiter icons, no caps.
    let mut theme = ThemeConfig::default();
    theme.ui.prompt_open = Some(delimiter("", "white", "transparent"));
    theme.ui.prompt_close = Some(delimiter("", "white", "transparent"));
    theme.ui.segment_open = Some(delimiter("", "white", "transparent"));
    theme.ui.segment_close = Some(delimiter("", "white", "transparent"));

    let first_and_last = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
    let mid_chain = clock_demo_spans(&theme, IsFirst::No, IsLast::No);

    assert_eq!(
        first_and_last.len(),
        1,
        "no caps expected: {first_and_last:?}"
    );
    assert_eq!(
        mid_chain.len(),
        2,
        "no caps expected, only the time span and the mid-chain gap: {mid_chain:?}"
    );
}

#[test]
fn clock_demo_spans_renders_closing_cap_when_last() {
    let mut theme = ThemeConfig::default();
    theme.ui.prompt_close = Some(delimiter(")", "white", "transparent"));

    let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::Yes);
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.ends_with(')'),
        "expected the prompt_close icon as the final glyph, got {text:?}"
    );
}

#[test]
fn clock_demo_spans_renders_segment_close_cap_when_not_last() {
    let mut theme = ThemeConfig::default();
    theme.ui.segment_close = Some(delimiter("]", "white", "transparent"));
    theme.ui.prompt_close = Some(delimiter(")", "white", "transparent"));

    let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::No);
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.ends_with(']'),
        "not-last clock should use segment_close, not prompt_close, got {text:?}"
    );
}

#[test]
fn clock_demo_spans_renders_opening_cap_when_not_first() {
    let mut theme = ThemeConfig::default();
    theme.ui.segment_open = Some(delimiter("[", "white", "transparent"));
    theme.ui.prompt_open = Some(delimiter("|", "white", "transparent"));

    let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::Yes);
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.starts_with('['),
        "not-first clock should use segment_open, not prompt_open, got {text:?}"
    );
}

#[test]
fn clock_demo_spans_uses_prompt_open_when_first() {
    let mut theme = ThemeConfig::default();
    theme.ui.segment_open = Some(delimiter("[", "white", "transparent"));
    theme.ui.prompt_open = Some(delimiter("|", "white", "transparent"));

    let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.starts_with('|'),
        "first clock should use prompt_open, not segment_open, got {text:?}"
    );
}

#[test]
fn clock_demo_spans_inserts_gap_space_before_non_last_close_cap() {
    let mut theme = ThemeConfig::default();
    theme.ui.segment_close = Some(delimiter("]", "white", "transparent"));

    let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::No);

    assert!(
        spans.iter().any(|s| s.text == " " && s.style.fg.is_none()),
        "expected a same-background gap span before the closing cap: {spans:?}"
    );
    let last = spans.last().expect("at least the time span");
    assert_eq!(last.text, "]");
}

#[test]
fn clock_demo_spans_renders_closing_cap_on_the_real_default_theme() {
    // Real embedded theme: `ui.prompt_close` is U+E0B4.
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();

    let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();

    assert!(
        text.ends_with('\u{e0b4}'),
        "expected the default theme's powerline closing cap, got {text:?}"
    );
}

#[test]
fn resolve_clock_cap_color_maps_match_bg_and_match_text_and_transparent() {
    assert_eq!(
        resolve_clock_cap_color("match_bg", "black", "white"),
        Some(TemplateColor::Named("black".to_owned()))
    );
    assert_eq!(
        resolve_clock_cap_color("match_text", "black", "white"),
        Some(TemplateColor::Named("white".to_owned()))
    );
    assert_eq!(
        resolve_clock_cap_color("transparent", "black", "white"),
        None
    );
    assert_eq!(
        resolve_clock_cap_color("red", "black", "white"),
        Some(TemplateColor::Named("red".to_owned()))
    );
}

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn preview_always_renders_character_line() {
    let config = make_config(
        &["duration"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format = Some("[$duration]($style)".to_owned());
    theme.segments.character.format = Some("[$symbol]($style)".to_owned());

    let lines = render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());

    assert_eq!(lines.len(), 2, "chain line plus character line: {lines:?}");
    let last = lines.last().expect("asserted above: two lines");
    assert!(
        line_text(last).contains('❯'),
        "last line must be the character line, got {:?}",
        line_text(last)
    );
}

#[test]
fn preview_renders_status_as_chain_pill() {
    let config = make_config(
        &["duration", "status"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());

    let mut theme = ThemeConfig::default();
    theme.segments.duration.format = Some("[$duration]($style)".to_owned());
    theme.segments.character.format = Some("[$symbol]($style)".to_owned());
    theme.segments.status.ok_icon = Some(Icon::new("✔").expect("valid icon"));

    let lines = render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());

    assert_eq!(lines.len(), 2, "chain line plus character line: {lines:?}");
    let chain_text = line_text(lines.first().expect("asserted above: two lines"));
    let char_text = line_text(lines.get(1_usize).expect("asserted above: two lines"));
    assert!(
        chain_text.contains("0.128s"),
        "chain must still contain duration, got {chain_text:?}"
    );
    assert!(
        chain_text.contains('✔') && !chain_text.contains('❯'),
        "status must be a pill in the chain, not the character: {chain_text:?}"
    );
    assert!(
        char_text.contains('❯'),
        "character line must follow the chain, got {char_text:?}"
    );
}

/// With `ui.show_icons` off the preview, like the live prompt, draws no caps.
#[test]
fn preview_draws_no_private_use_caps_when_icons_are_off() {
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();
    let render = |show_icons: bool| {
        let mut config = make_config(
            &["clock", "duration", "directory"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        config.ui.show_icons = show_icons;
        let state = test_state(config.clone());
        render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts())
            .iter()
            .map(line_text)
            .collect::<String>()
    };

    let on = render(true);
    assert!(
        on.contains('\u{e0ba}') && on.contains('\u{e0bc}') && on.contains('\u{e0b4}'),
        "show_icons=true control must draw the caps, got {on:?}"
    );
    let off = render(false);
    assert!(
        !has_private_use(&off),
        "show_icons=false preview leaked a private-use glyph: {off:?}"
    );
    assert!(
        off.contains("0.128s") && off.contains("9:41"),
        "segments themselves must still render, got {off:?}"
    );
}

#[test]
fn preview_status_pill_uses_fail_variant_and_theme_fallback_icon() {
    let config = make_config(
        &["status"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());
    let theme = ThemeConfig::default();
    let mut facts = make_facts();

    facts.sample_character_success = false;
    let fail_lines = render_preview_line(&state, &config, &theme, &Palette::default(), &facts);
    let fail_text = line_text(fail_lines.first().expect("chain line"));
    assert!(fail_text.contains('✖'), "fail pill, got {fail_text:?}");

    facts.sample_character_success = true;
    let ok_lines = render_preview_line(&state, &config, &theme, &Palette::default(), &facts);
    let ok_text = line_text(ok_lines.first().expect("chain line"));
    assert!(ok_text.contains('✔'), "ok pill, got {ok_text:?}");
}
