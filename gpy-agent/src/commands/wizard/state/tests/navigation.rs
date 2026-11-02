use super::*;

#[test]
fn section_next_wraps_theme_palette_segments_theme() {
    assert_eq!(Section::Theme.next(), Section::Palette);
    assert_eq!(Section::Palette.next(), Section::Segments);
    assert_eq!(Section::Segments.next(), Section::Theme);
}

#[test]
fn section_prev_wraps_the_other_direction() {
    assert_eq!(Section::Theme.prev(), Section::Segments);
    assert_eq!(Section::Segments.prev(), Section::Palette);
    assert_eq!(Section::Palette.prev(), Section::Theme);
}

#[test]
fn theme_cursor_starts_at_selected_theme_index() {
    let config = make_config("default", "default", &[]);
    let state = test_state(config);

    let expected = state
        .available_themes()
        .iter()
        .position(|t| t == "default")
        .expect("default theme should be discoverable");

    assert_eq!(state.theme_cursor(), expected);
}

#[test]
fn palette_cursor_starts_at_selected_palette_index() {
    let config = make_config("default", "default", &[]);
    let state = test_state(config);

    let expected = state
        .available_palettes()
        .iter()
        .position(|p| p == "default")
        .expect("default palette should be discoverable");

    assert_eq!(state.palette_cursor(), expected);
}

#[test]
fn move_cursor_down_advances_within_focused_section() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section(); // Theme -> Palette
    state.next_section(); // Palette -> Segments
    assert_eq!(state.focus(), Section::Segments);
    assert!(
        state.available_segments().len() >= 2,
        "test relies on at least two builtin segments existing"
    );

    let start = state.segment_cursor();
    state.move_cursor_down();

    assert_eq!(state.segment_cursor(), start + 1);
}

#[test]
fn move_cursor_up_retreats_within_focused_section() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);

    state.move_cursor_down();
    state.move_cursor_down();
    let after_two_down = state.segment_cursor();
    state.move_cursor_up();

    assert_eq!(state.segment_cursor(), after_two_down - 1);
}

#[test]
fn move_cursor_down_clamps_at_list_end() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);

    let len = state.available_segments().len();
    for _ in 0..len + 5 {
        state.move_cursor_down();
    }

    assert_eq!(state.segment_cursor(), len - 1);
}

#[test]
fn move_cursor_up_clamps_at_zero() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);
    assert_eq!(state.segment_cursor(), 0);

    state.move_cursor_up();

    assert_eq!(state.segment_cursor(), 0);
}

#[test]
fn move_cursor_only_affects_focused_section() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    assert_eq!(state.focus(), Section::Theme);

    let palette_before = state.palette_cursor();
    let segment_before = state.segment_cursor();

    state.move_cursor_down();

    assert_eq!(state.palette_cursor(), palette_before);
    assert_eq!(state.segment_cursor(), segment_before);
}

#[test]
fn activate_cursor_selects_theme_at_cursor_when_theme_focused() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    assert_eq!(state.focus(), Section::Theme);

    let expected = state
        .available_themes()
        .get(state.theme_cursor())
        .cloned()
        .expect("theme cursor should point at a valid theme");

    state.activate_cursor();

    assert_eq!(state.selected_theme(), expected);
}

#[test]
fn activate_cursor_selects_palette_at_cursor_when_palette_focused() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section();
    assert_eq!(state.focus(), Section::Palette);

    let expected = state
        .available_palettes()
        .get(state.palette_cursor())
        .cloned()
        .expect("palette cursor should point at a valid palette");

    state.activate_cursor();

    assert_eq!(state.selected_palette(), expected);
}

#[test]
fn activate_cursor_toggles_segment_at_cursor_when_segments_focused() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);

    let target = state
        .available_segments()
        .first()
        .cloned()
        .expect("at least one builtin segment should exist");
    let before = state.is_segment_enabled(&target);

    state.activate_cursor();

    assert_ne!(state.is_segment_enabled(&target), before);
}
