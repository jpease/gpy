use super::*;

#[test]
fn new_reflects_config_theme_and_palette() {
    let config = make_config("default", "default", &["directory"]);
    let state = test_state(config);

    assert_eq!(state.selected_theme(), "default");
    assert_eq!(state.selected_palette(), "default");
}

#[test]
fn initial_theme_and_palette_reflect_startup_config() {
    let config = make_config("default", "default", &["directory"]);
    let state = test_state(config);

    assert_eq!(state.initial_theme(), "default");
    assert_eq!(state.initial_palette(), "default");
}

#[test]
fn initial_theme_stays_fixed_after_selection_changes() {
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

    assert_eq!(state.initial_theme(), "default");
    assert_eq!(state.selected_theme(), other);
}

#[test]
fn new_reflects_list_membership_for_git_and_language() {
    let config = make_config("default", "default", &["directory", "git"]);
    let state = test_state(config);

    assert!(state.is_segment_enabled("git"));
    assert!(!state.is_segment_enabled("language"));
    assert!(state.is_segment_enabled("directory"));
}

#[test]
fn select_theme_ignores_unknown_name() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);

    state.select_theme("does-not-exist");

    assert_eq!(state.selected_theme(), "default");
}

#[test]
fn select_theme_updates_known_name() {
    let config = make_config("default", "default", &[]);
    let mut state = test_state(config);

    let maybe_other = state
        .available_themes()
        .iter()
        .find(|name| name.as_str() != "default")
        .cloned();

    let Some(other) = maybe_other else {
        state.select_theme("default");
        assert_eq!(state.selected_theme(), "default");
        return;
    };

    state.select_theme(&other);

    assert_eq!(state.selected_theme(), other);
}

#[test]
fn toggle_segment_is_idempotent_pair() {
    let config = make_config("default", "default", &["directory"]);
    let mut state = test_state(config);

    let original = state.is_segment_enabled("directory");
    state.toggle_segment("directory");
    assert_ne!(state.is_segment_enabled("directory"), original);
    state.toggle_segment("directory");
    assert_eq!(state.is_segment_enabled("directory"), original);

    state.toggle_segment("not-a-real-segment");
    assert!(!state.is_segment_enabled("not-a-real-segment"));
}
