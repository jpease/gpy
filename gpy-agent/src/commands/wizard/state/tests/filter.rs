use super::*;

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
