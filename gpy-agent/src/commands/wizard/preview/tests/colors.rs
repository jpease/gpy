use super::*;

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
