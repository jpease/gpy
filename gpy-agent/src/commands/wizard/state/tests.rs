#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use super::*;
use crate::config::types::{PaletteName, ThemeName};

/// Fixed discovery lists (clean-environment builtins) so tests don't depend on disk.
const FIXED_THEMES: [&str; 3] = ["default", "starship", "text"];
const FIXED_PALETTES: [&str; 8] = [
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "default",
    "gruvbox-dark-medium",
    "nord",
    "starship",
];
const FIXED_SEGMENTS: [&str; 6] = [
    "clock",
    "duration",
    "language",
    "directory",
    "git",
    "status",
];

fn test_state(config: Config) -> WizardState {
    WizardState::from_parts(
        config,
        FIXED_THEMES.iter().map(|&s| s.to_owned()).collect(),
        FIXED_PALETTES.iter().map(|&s| s.to_owned()).collect(),
        FIXED_SEGMENTS.iter().map(|&s| s.to_owned()).collect(),
    )
}

/// Avoids two bool params (`max-fn-params-bools`).
#[derive(Clone, Copy)]
struct GitLanguageFlags {
    git: bool,
    language: bool,
}

/// Test config; `git`/`language` are listed only if the caller adds them.
fn make_config(
    theme: &str,
    palette: &str,
    flags: GitLanguageFlags,
    enabled_segments: &[&str],
) -> Config {
    let mut config = Config::default();
    config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
    config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
    config.git.enabled = flags.git;
    config.language.enabled = flags.language;
    config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
    config
}

#[test]
fn new_reflects_config_theme_and_palette() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let state = test_state(config);

    assert_eq!(state.selected_theme(), "default");
    assert_eq!(state.selected_palette(), "default");
}

#[test]
fn initial_theme_and_palette_reflect_startup_config() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let state = test_state(config);

    assert_eq!(state.initial_theme(), "default");
    assert_eq!(state.initial_palette(), "default");
}

#[test]
fn initial_theme_stays_fixed_after_selection_changes() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
fn new_reflects_flag_and_list_membership_for_git_and_language() {
    // Rendering needs both list membership and the flag; each has only one.
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: false,
            language: true,
        },
        &["directory", "git"],
    );
    let state = test_state(config);

    assert!(!state.is_segment_enabled("git"));
    assert!(!state.is_segment_enabled("language"));
    assert!(state.is_segment_enabled("directory"));

    let both = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["language", "directory", "git"],
    );
    let both_state = test_state(both);
    assert!(both_state.is_segment_enabled("git"));
    assert!(both_state.is_segment_enabled("language"));
}

#[test]
fn select_theme_ignores_unknown_name() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
    let mut state = test_state(config);

    state.select_theme("does-not-exist");

    assert_eq!(state.selected_theme(), "default");
}

#[test]
fn select_theme_updates_known_name() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let mut state = test_state(config);

    let original = state.is_segment_enabled("directory");
    state.toggle_segment("directory");
    assert_ne!(state.is_segment_enabled("directory"), original);
    state.toggle_segment("directory");
    assert_eq!(state.is_segment_enabled("directory"), original);

    state.toggle_segment("not-a-real-segment");
    assert!(!state.is_segment_enabled("not-a-real-segment"));
}

#[test]
fn dirty_false_immediately_after_new() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let state = test_state(config);

    assert!(!state.dirty());
}

#[test]
fn dirty_true_after_theme_change() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let mut state = test_state(config);

    state.toggle_segment("directory");

    assert!(state.dirty());
}

#[test]
fn dirty_false_after_toggle_and_untoggle() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    let mut state = test_state(config);

    state.toggle_segment("directory");
    state.toggle_segment("directory");

    assert!(!state.dirty());
}

#[test]
fn apply_to_updates_theme_palette_and_git_language_flags() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: false,
            language: false,
        },
        &["directory"],
    );
    let mut state = test_state(config);

    state.select_theme("default");
    state.select_palette("default");
    state.toggle_segment("git");
    state.toggle_segment("language");

    let mut applied = Config::default();
    state.apply_to(&mut applied);

    assert_eq!(applied.ui.theme.as_str(), "default");
    assert_eq!(applied.ui.palette.to_string(), "default");
    assert!(applied.git.enabled);
    assert!(applied.language.enabled);
    assert!(
        applied
            .ui
            .enabled_segments
            .contains(&"directory".to_owned())
    );
    // Export needs both the list entry and the flag, in BUILTIN_ORDER.
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
    assert!(applied.git.enabled);
    assert!(applied.language.enabled);
}

#[test]
fn apply_to_deselected_git_clears_flag_and_keeps_list_entry() {
    // Like `gpy disable git`: entry kept so re-enabling restores its position.
    let mut state = test_state(Config::default());
    state.toggle_segment("git");
    let mut applied = Config::default();
    state.apply_to(&mut applied);

    assert!(!applied.git.enabled);
    assert_eq!(
        applied.ui.enabled_segments,
        Config::default().ui.enabled_segments
    );
}

fn clean_state() -> WizardState {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
    test_state(config)
}

#[test]
fn directory_display_defaults_to_the_config_value() {
    let mut config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
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
    let mut config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory"],
    );
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
fn reapply_to_keeps_a_segment_toggled_off_and_on_in_its_startup_place() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &["directory", "status", "clock"],
    );
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

#[test]
fn apply_to_preserves_builtin_order_in_enabled_segments() {
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: false,
            language: false,
        },
        &[],
    );
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
        GitLanguageFlags {
            git: false,
            language: false,
        },
        &["directory", "clock", "removed-plugin-seg"],
    );
    let state = test_state(config.clone());

    let mut applied = config.clone();
    state.apply_to(&mut applied);

    assert_eq!(applied.ui.enabled_segments, config.ui.enabled_segments);
}

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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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
    let config = make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    );
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

#[test]
fn filtered_indices_is_identity_when_query_empty() {
    let state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));

    let indices = state.filtered_indices(Section::Segments);

    assert_eq!(
        indices,
        (0..state.available_segments().len()).collect::<Vec<_>>()
    );
}

#[test]
fn filtered_indices_narrows_to_matching_items() {
    let state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));

    let indices = state.filtered_indices(Section::Segments);
    let has_match = indices.into_iter().any(|i| {
        state
            .available_segments()
            .get(i)
            .is_some_and(|segment| segment.contains("dir"))
    });

    assert!(
        has_match,
        "test relies on a segment containing 'dir' (e.g. directory) existing"
    );
}

#[test]
fn move_cursor_down_steps_through_filtered_list_only() {
    let mut state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);

    let indices = state.filtered_indices(Section::Segments);
    assert!(indices.len() >= 2, "test relies on 2+ builtin segments");

    state.move_cursor_down();

    assert_eq!(
        state.segment_cursor(),
        *indices.get(1).expect("indices has at least 2 entries")
    );
}

fn segments_focused_state() -> WizardState {
    let mut state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));
    state.next_section();
    state.next_section();
    assert_eq!(state.focus(), Section::Segments);
    state
}

#[test]
fn push_filter_char_narrows_focused_section_list() {
    let mut state = segments_focused_state();

    for c in "dir".chars() {
        state.push_filter_char(c);
    }

    assert_eq!(state.filter_query(), "dir");
    let indices = state.filtered_indices(Section::Segments);
    assert_ne!(indices, Vec::<usize>::new());
    assert!(
        indices.iter().all(|&i| state
            .available_segments()
            .get(i)
            .is_some_and(|segment| segment.contains("dir"))),
        "every filtered index should match the typed query"
    );
}

#[test]
fn push_filter_char_snaps_cursor_to_top_match_when_current_item_no_longer_matches() {
    let mut state = segments_focused_state();
    let git_index = state
        .available_segments()
        .iter()
        .position(|s| s == "git")
        .expect("git should be a builtin segment");
    while state.segment_cursor() != git_index {
        state.move_cursor_down();
    }

    for c in "dir".chars() {
        state.push_filter_char(c);
    }

    let cursor_item = state
        .available_segments()
        .get(state.segment_cursor())
        .expect("segment cursor should point at a valid segment");
    assert!(
        cursor_item.contains("dir"),
        "cursor should have snapped onto a matching item, was {cursor_item:?}"
    );
}

#[test]
fn pop_filter_char_never_invalidates_cursor() {
    let mut state = segments_focused_state();
    for c in "dir".chars() {
        state.push_filter_char(c);
    }
    let cursor_after_push = state.segment_cursor();

    state.pop_filter_char();

    assert_eq!(state.filter_query(), "di");
    assert_eq!(
        state.segment_cursor(),
        cursor_after_push,
        "widening the query should never move a still-valid cursor"
    );
}

#[test]
fn clear_filter_restores_full_list_and_keeps_cursor_item_highlighted() {
    let mut state = segments_focused_state();
    for c in "dir".chars() {
        state.push_filter_char(c);
    }
    let highlighted = state
        .available_segments()
        .get(state.segment_cursor())
        .expect("segment cursor should point at a valid segment")
        .clone();

    state.clear_filter();

    assert_eq!(state.filter_query(), "");
    assert_eq!(
        state.filtered_indices(Section::Segments).len(),
        state.available_segments().len()
    );
    assert_eq!(
        state
            .available_segments()
            .get(state.segment_cursor())
            .expect("segment cursor should point at a valid segment"),
        &highlighted,
        "clearing the filter should not move the cursor off the item it was on"
    );
}

#[test]
fn filter_query_is_shared_across_sections_and_persists_across_tab() {
    let mut state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));
    assert_eq!(state.focus(), Section::Theme);

    state.push_filter_char('d');
    assert_eq!(state.filter_query(), "d");

    state.next_section(); // Theme -> Palette
    assert_eq!(
        state.filter_query(),
        "d",
        "the filter query is shared, not per-section — switching focus must not change it"
    );

    state.next_section(); // Palette -> Segments
    state.next_section(); // Segments -> Theme
    assert_eq!(state.focus(), Section::Theme);
    assert_eq!(
        state.filter_query(),
        "d",
        "the filter query must survive being tabbed away from and back"
    );
}

#[test]
fn push_filter_char_narrows_every_section_not_just_focused() {
    let mut state = test_state(make_config(
        "default",
        "default",
        GitLanguageFlags {
            git: true,
            language: true,
        },
        &[],
    ));
    assert_eq!(state.focus(), Section::Theme);

    for c in "dir".chars() {
        state.push_filter_char(c);
    }

    let segment_indices = state.filtered_indices(Section::Segments);
    assert!(
        !segment_indices.is_empty(),
        "Segments should still narrow to matches even though Theme has focus"
    );
    assert!(
        segment_indices.iter().all(|&i| state
            .available_segments()
            .get(i)
            .is_some_and(|segment| segment.contains("dir"))),
        "every filtered Segments index should match the shared query"
    );
}

#[test]
fn empty_filtered_result_makes_cursor_movement_and_activation_no_ops() {
    let mut state = segments_focused_state();
    let cursor_before = state.segment_cursor();
    let enabled_before: Vec<bool> = state
        .available_segments()
        .iter()
        .map(|s| state.is_segment_enabled(s))
        .collect();

    for c in "zzzznotarealsegment".chars() {
        state.push_filter_char(c);
    }
    assert_eq!(
        state.filtered_indices(Section::Segments),
        Vec::<usize>::new()
    );

    state.move_cursor_up();
    state.move_cursor_down();
    state.activate_cursor();

    assert_eq!(
        state.segment_cursor(),
        cursor_before,
        "cursor should not move against an empty filtered list"
    );
    let enabled_after: Vec<bool> = state
        .available_segments()
        .iter()
        .map(|s| state.is_segment_enabled(s))
        .collect();
    assert_eq!(
        enabled_after, enabled_before,
        "activate_cursor should be a no-op against an empty filtered list"
    );
}
