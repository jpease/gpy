use super::*;

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
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

    let spans = render_segment("clock", &config, &theme, &facts, &ctx)
        .expect("clock should always render a demo");
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert!(
        text.contains("9:41"),
        "expected the fixed demo time in clock output, got {text:?}"
    );
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

/// With `ui.show_icons` off, the default theme's Nerd Font status icons give
/// way to the shells' ASCII fallback.
#[test]
fn preview_status_pill_drops_private_use_icon_when_icons_are_off() {
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();
    let render = |show_icons: bool, success: bool| {
        let mut config = make_config(
            &["status"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        config.ui.show_icons = show_icons;
        let state = test_state(config.clone());
        let mut facts = make_facts();
        facts.sample_character_success = success;
        let lines = render_preview_line(&state, &config, &theme, &Palette::default(), &facts);
        line_text(lines.first().expect("chain line"))
    };

    assert!(
        has_private_use(&render(true, true)),
        "show_icons=true control must draw the theme's glyph"
    );
    for (success, fallback) in [(true, "ok"), (false, "err")] {
        let off = render(false, success);
        assert!(
            off.contains(fallback) && !has_private_use(&off),
            "show_icons=false status pill must show {fallback:?}, got {off:?}"
        );
    }
}

/// Status draws like every default-theme pill: first in the chain it starts
/// with exactly one space and no opening cap.
#[test]
fn preview_default_theme_pills_lead_with_one_space_when_first() {
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("default theme should always load")
        .get();
    let config = make_config(
        &[],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let facts = make_facts();
    {
        let (segment, content) = ("status", "\u{f058}");
        let first = RenderContext::new(&config, &theme, SegmentPosition::FIRST);
        let first_text: String = render_segment(segment, &config, &theme, &facts, &first)
            .unwrap_or_else(|| panic!("{segment} renders with the default theme"))
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert!(
            first_text.starts_with(&format!(" {content}")) && !first_text.contains('\u{e0ba}'),
            "{segment}: first pill must start with one space and no cap, got {first_text:?}"
        );

        let middle = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let middle_text: String = render_segment(segment, &config, &theme, &facts, &middle)
            .unwrap_or_else(|| panic!("{segment} renders with the default theme"))
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert!(
            middle_text.starts_with(&format!("\u{e0ba} {content}")),
            "{segment}: non-first pill opens with the cap then one space, got {middle_text:?}"
        );
    }
}

/// A `format` on the status segment drives the preview pill, in the failure color too.
#[test]
fn preview_status_format_uses_the_outcome_colors() {
    let config = make_config(
        &["status"],
        GitLanguageFlags {
            git: false,
            language: false,
        },
    );
    let state = test_state(config.clone());
    let mut theme = ThemeConfig::default();
    theme.segments.status.format = Some("[ $symbol]($style)".to_owned());
    theme.segments.status.ok_icon = Some(Icon::new("OK").expect("valid icon"));
    theme.segments.status.fail_icon = Some(Icon::new("NO").expect("valid icon"));
    let mut facts = make_facts();

    let text_for = |sample: &PreviewFacts| {
        let lines = render_preview_line(&state, &config, &theme, &Palette::default(), sample);
        line_text(lines.first().expect("chain line"))
    };
    facts.sample_character_success = true;
    assert_eq!(text_for(&facts).trim_end(), " OK");
    facts.sample_character_success = false;
    assert_eq!(text_for(&facts).trim_end(), " NO");
}
