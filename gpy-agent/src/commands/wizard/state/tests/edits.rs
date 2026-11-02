use super::*;

fn clean_state() -> WizardState {
    let config = make_config("default", "default", &["directory"]);
    test_state(config)
}

#[test]
fn directory_display_defaults_to_the_config_value() {
    let mut config = make_config("default", "default", &["directory"]);
    config.ui.directory.display = crate::config::types::DirectoryDisplay::Truncated;
    let state = test_state(config);

    assert_eq!(
        state.directory_display(),
        crate::config::types::DirectoryDisplay::Truncated
    );
}

#[test]
fn directory_display_edits_cycle_through_every_variant_both_ways() {
    use crate::config::types::DirectoryDisplay;

    let mut state = clean_state();
    let base = ThemeConfig::default();
    assert_eq!(state.directory_display(), DirectoryDisplay::Basename);

    for expected in [
        DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Truncated,
        DirectoryDisplay::Full,
        DirectoryDisplay::Basename,
    ] {
        state.apply_edit(SegmentEdit::NextDirectoryDisplay, &base);
        assert_eq!(state.directory_display(), expected);
    }
    for expected in [
        DirectoryDisplay::Full,
        DirectoryDisplay::Truncated,
        DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Basename,
    ] {
        state.apply_edit(SegmentEdit::PrevDirectoryDisplay, &base);
        assert_eq!(state.directory_display(), expected);
    }
}

#[test]
fn directory_display_edit_marks_dirty_and_reaches_apply_to() {
    let mut state = clean_state();

    state.apply_edit(SegmentEdit::NextDirectoryDisplay, &ThemeConfig::default());

    assert!(state.dirty());
    let mut applied = Config::default();
    state.apply_to(&mut applied);
    assert_eq!(
        applied.ui.directory.display,
        crate::config::types::DirectoryDisplay::Abbreviated
    );
}

#[test]
fn language_display_defaults_to_the_config_value() {
    let mut config = make_config("default", "default", &["directory"]);
    config.language.display = crate::config::types::LanguageDisplay::Text;
    let state = test_state(config);

    assert_eq!(
        state.language_display(),
        crate::config::types::LanguageDisplay::Text
    );
}

#[test]
fn language_display_edit_toggles_between_icon_and_text() {
    use crate::config::types::LanguageDisplay;

    let mut state = clean_state();
    let base = ThemeConfig::default();
    assert_eq!(state.language_display(), LanguageDisplay::Icon);

    state.apply_edit(SegmentEdit::ToggleLanguageDisplay, &base);
    assert_eq!(state.language_display(), LanguageDisplay::Text);
    assert!(state.dirty());
    let mut applied = Config::default();
    state.apply_to(&mut applied);
    assert_eq!(applied.language.display, LanguageDisplay::Text);

    state.apply_edit(SegmentEdit::ToggleLanguageDisplay, &base);
    assert_eq!(state.language_display(), LanguageDisplay::Icon);
}

#[test]
fn language_show_versions_edit_flips_the_flag() {
    let mut state = clean_state();
    let base = ThemeConfig::default();
    let before = state.language_show_versions();

    state.apply_edit(SegmentEdit::ToggleLanguageShowVersions, &base);
    assert_eq!(state.language_show_versions(), !before);
    assert!(state.dirty());
    let mut applied = Config::default();
    state.apply_to(&mut applied);
    assert_eq!(applied.language.show_versions, !before);

    state.apply_edit(SegmentEdit::ToggleLanguageShowVersions, &base);
    assert_eq!(state.language_show_versions(), before);
}

/// The theme options `SegmentEdit`s toggle, as on/off flags.
fn theme_options(theme: &ThemeConfig) -> Vec<bool> {
    let segments = &theme.segments;
    vec![
        segments.clock.time_format() == "24",
        segments.clock.show_seconds(),
        segments.duration.show_milliseconds,
        segments.git.show_branch(),
        segments.git.show_ahead_behind(),
        segments.git.show_stash(),
    ]
}

#[test]
fn active_theme_reads_the_base_until_edited() {
    let state = clean_state();
    let base = ThemeConfig::default();

    assert_eq!(
        theme_options(state.active_theme(&base)),
        [false, false, true, true, true, true]
    );
    assert!(state.pending_theme().is_none());
}

#[test]
fn each_theme_edit_flips_only_its_own_option() {
    let base = ThemeConfig::default();
    let edits = [
        SegmentEdit::ToggleClockTimeFormat,
        SegmentEdit::ToggleClockShowSeconds,
        SegmentEdit::ToggleDurationShowMilliseconds,
        SegmentEdit::ToggleGitShowBranch,
        SegmentEdit::ToggleGitShowAheadBehind,
        SegmentEdit::ToggleGitShowStash,
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let mut state = clean_state();
        let expected: Vec<bool> = theme_options(&base)
            .into_iter()
            .enumerate()
            .map(|(option, on)| on ^ (option == index))
            .collect();

        state.apply_edit(edit, &base);
        assert_eq!(
            theme_options(state.active_theme(&base)),
            expected,
            "{edit:?}"
        );
        assert!(state.dirty(), "{edit:?} must mark the state dirty");

        state.apply_edit(edit, &base);
        assert_eq!(
            theme_options(state.active_theme(&base)),
            theme_options(&base),
            "{edit:?} twice"
        );
    }
}

#[test]
fn clock_edits_accumulate_in_pending_theme_without_clobbering_each_other() {
    let mut state = clean_state();
    let base = ThemeConfig::default();

    state.apply_edit(SegmentEdit::ToggleClockTimeFormat, &base);
    state.apply_edit(SegmentEdit::ToggleClockShowSeconds, &base);

    let pending = state.pending_theme().expect("pending theme present");
    assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
    assert_eq!(pending.segments.clock.show_seconds, Some(true));
}

#[test]
fn patch_active_theme_seeds_pending_and_marks_dirty() {
    let mut state = clean_state();
    assert!(!state.dirty(), "state should start clean");
    assert!(state.pending_theme().is_none());

    let base = ThemeConfig::default();
    state.patch_active_theme(&base, |theme| {
        theme.segments.clock.time_format = Some("24".to_owned());
    });

    assert!(
        state.dirty(),
        "a pending theme edit must mark the state dirty"
    );
    let pending = state
        .pending_theme()
        .expect("pending theme present after patch");
    assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
}

#[test]
fn patch_active_theme_accumulates_successive_edits() {
    let mut state = clean_state();
    let base = ThemeConfig::default();

    state.patch_active_theme(&base, |theme| {
        theme.segments.clock.time_format = Some("24".to_owned());
    });
    state.patch_active_theme(&base, |theme| {
        theme.segments.duration.show_milliseconds = false;
    });

    let pending = state.pending_theme().expect("pending theme present");
    assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
    assert!(!pending.segments.duration.show_milliseconds);
}

#[test]
fn patch_active_theme_consults_base_only_for_the_first_edit() {
    let mut state = clean_state();

    let mut first_base = ThemeConfig::default();
    first_base.segments.clock.show_leading_zero = Some(true);
    state.patch_active_theme(&first_base, |theme| {
        theme.segments.clock.time_format = Some("24".to_owned());
    });

    // A different base is ignored once pending exists.
    let mut second_base = ThemeConfig::default();
    second_base.segments.clock.show_leading_zero = Some(false);
    state.patch_active_theme(&second_base, |theme| {
        theme.segments.duration.show_milliseconds = false;
    });

    let pending = state.pending_theme().expect("pending theme present");
    assert_eq!(
        pending.segments.clock.show_leading_zero,
        Some(true),
        "second call's base must not overwrite the accumulated copy"
    );
}

#[test]
fn select_theme_change_discards_pending_theme_edits() {
    let mut state = clean_state();
    let base = ThemeConfig::default();
    state.patch_active_theme(&base, |theme| {
        theme.segments.clock.time_format = Some("24".to_owned());
    });
    assert!(state.pending_theme().is_some());

    state.select_theme("text");

    assert!(
        state.pending_theme().is_none(),
        "switching themes must drop edits that described the previous theme"
    );
}

#[test]
fn select_theme_same_name_keeps_pending_theme_edits() {
    let mut state = clean_state();
    let base = ThemeConfig::default();
    state.patch_active_theme(&base, |theme| {
        theme.segments.clock.time_format = Some("24".to_owned());
    });

    // Re-selecting the same theme must keep pending edits.
    state.select_theme("default");

    assert_eq!(
        state
            .pending_theme()
            .and_then(|theme| theme.segments.clock.time_format.as_deref()),
        Some("24")
    );
}
