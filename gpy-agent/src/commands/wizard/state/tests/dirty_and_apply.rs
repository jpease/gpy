use super::*;

#[test]
fn dirty_false_immediately_after_new() {
    let config = make_config("default", "default", &["directory"]);
    let state = test_state(config);

    assert!(!state.dirty());
}

#[test]
fn dirty_true_after_theme_change() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);

    let Some(other) = state
        .available_themes()
        .iter()
        .find(|name| name.as_str() != "default")
        .cloned()
    else {
        return;
    };

    state.select_theme(&other);

    assert!(state.dirty());
}

#[test]
fn dirty_true_after_palette_change() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);

    let Some(other) = state
        .available_palettes()
        .iter()
        .find(|name| name.as_str() != "default")
        .cloned()
    else {
        return;
    };

    state.select_palette(&other);

    assert!(state.dirty());
}

#[test]
fn dirty_true_after_segment_toggle() {
    let config = make_config("default", "default", &["directory"]);
    let mut state = test_state(config);

    state.toggle_segment("directory");

    assert!(state.dirty());
}

#[test]
fn dirty_false_after_toggle_and_untoggle() {
    let config = make_config("default", "default", &["directory"]);
    let mut state = test_state(config);

    state.toggle_segment("directory");
    state.toggle_segment("directory");

    assert!(!state.dirty());
}

#[test]
fn apply_to_updates_theme_palette_and_segment_list() {
    let config = make_config("default", "default", &["directory"]);
    let mut state = test_state(config);

    state.select_theme("default");
    state.select_palette("default");
    state.toggle_segment("git");
    state.toggle_segment("language");

    let mut applied = Config::default();
    state.apply_to(&mut applied);

    assert_eq!(applied.ui.theme.as_str(), "default");
    assert_eq!(applied.ui.palette.to_string(), "default");
    assert!(
        applied
            .ui
            .enabled_segments
            .contains(&"directory".to_owned())
    );
    assert_eq!(
        applied.ui.enabled_segments,
        vec!["language", "directory", "git"]
    );
}

#[test]
fn apply_to_keeps_default_git_and_language_in_enabled_segments() {
    let state = test_state(Config::default());
    let mut applied = Config::default();
    state.apply_to(&mut applied);

    assert_eq!(
        applied.ui.enabled_segments,
        Config::default().ui.enabled_segments
    );
}

#[test]
fn apply_to_deselected_git_removes_list_entry() {
    // Like `gpy disable git`: the list entry goes.
    let mut state = test_state(Config::default());
    state.toggle_segment("git");
    let mut applied = Config::default();
    state.apply_to(&mut applied);

    assert!(!applied.segment_enabled("git"));
    assert_eq!(
        applied.ui.enabled_segments,
        ["clock", "duration", "language", "directory"]
    );
}

#[test]
fn apply_to_selected_builtin_inserts_canonically() {
    let config = make_config("default", "default", &["git", "clock", "directory"]);
    let mut applied = config.clone();
    let mut state = test_state(config);
    state.toggle_segment("language");
    state.apply_to(&mut applied);

    assert_eq!(
        applied.ui.enabled_segments,
        ["git", "clock", "language", "directory"]
    );
}

#[test]
fn reapply_to_keeps_a_segment_toggled_off_and_on_in_its_startup_place() {
    let config = make_config("default", "default", &["directory", "status", "clock"]);
    let mut preview = config.clone();
    let mut state = test_state(config);

    state.toggle_segment("status");
    state.reapply_to(&mut preview);
    assert_eq!(preview.ui.enabled_segments, ["directory", "clock"]);

    state.toggle_segment("status");
    state.reapply_to(&mut preview);
    assert_eq!(
        preview.ui.enabled_segments,
        ["directory", "status", "clock"]
    );
}

#[test]
fn apply_to_preserves_builtin_order_in_enabled_segments() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config.clone());

    for segment in ["status", "clock", "directory", "duration"] {
        state.toggle_segment(segment);
    }

    // Apply onto the starting config, as `save_to` does: deselected
    // git/language keep existing list entries.
    let mut applied = config;
    state.apply_to(&mut applied);

    assert_eq!(
        applied.ui.enabled_segments,
        vec![
            "clock".to_owned(),
            "duration".to_owned(),
            "directory".to_owned(),
            "status".to_owned(),
        ],
        "enabled_segments must follow BUILTIN_ORDER, not toggle order"
    );
}

#[test]
fn apply_to_preserves_existing_order_and_unoffered_entries() {
    let config = make_config(
        "default",
        "default",
        &["directory", "clock", "removed-plugin-seg"],
    );
    let state = test_state(config.clone());

    let mut applied = config.clone();
    state.apply_to(&mut applied);

    assert_eq!(applied.ui.enabled_segments, config.ui.enabled_segments);
}
