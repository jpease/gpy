use super::translate_git;
use crate::import::starship::Warnings;

fn table(toml_src: &str) -> toml::value::Table {
    toml::from_str(toml_src).unwrap()
}

#[test]
fn translate_git_composes_branch_and_status() {
    let branch = table("style = \"bold purple\"\n");
    let status = table("style = \"bold red\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert_eq!(
        format,
        r"on [($symbol )$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )"
    );
}

#[test]
fn git_branch_and_status_concatenate_without_separator() {
    let branch = table("format = \"[$branch](bg:blue)\"\n");
    let status = table("format = \"[$all_status](bg:blue)\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("[$branch](bg:blue)[$status](bg:blue)")
    );
}

#[test]
fn translate_git_maps_all_status_and_drops_remote_branch() {
    let branch = table(
        "format = \"on [$symbol$branch(:$remote_branch)]($style) \"\nstyle = \"bold purple\"\nsymbol = \" \"\n",
    );
    let status =
        table("format = '([\\[$all_status$ahead_behind\\]]($style) )'\nstyle = \"bold red\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert!(
        format.contains("$status"),
        "all_status mapped to status: {format}"
    );
    assert!(!format.contains("all_status"));
    assert!(
        !format.contains("remote_branch"),
        "remote_branch dropped: {format}"
    );
    assert!(warnings.iter().any(|w| w.message.contains("remote_branch")));
    assert!(!format.contains("(:)"), "empty grouping removed: {format}");
}

#[test]
fn translate_git_maps_braced_all_status() {
    let branch = table("style = \"bold purple\"\nsymbol = \" \"\n");
    let status =
        table("format = '([${all_status}${ahead_behind}]($style) )'\nstyle = \"bold red\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    // Assemble "${status}" without a brace-placeholder literal, which clippy's
    // `literal_string_with_formatting_args` flags outside a format! macro.
    let braced_status = ["${", "status", "}"].concat();
    assert!(
        format.contains("$status") || format.contains(&braced_status),
        "braced all_status mapped to status: {format}"
    );
    assert!(
        !format.contains("all_status"),
        "all_status not present in output: {format}"
    );
    assert!(
        !warnings.iter().any(|w| w.message.contains("all_status")),
        "no warning for all_status: {warnings:?}"
    );
}

#[test]
fn translate_git_collapses_granular_status_flags() {
    // Pure Preset builds the dirty indicator from granular flags; all collapse to `$status`.
    let branch = table("style = \"bold purple\"\nsymbol = \" \"\n");
    let status = table(
        "format = '[[(*$conflicted$untracked$modified$staged$renamed$deleted)](218) ($ahead_behind$stashed)]($style)'\nstyle = \"bold red\"\n",
    );
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert_eq!(
        format.matches("$status").count(),
        1_usize,
        "granular flags collapse to exactly one $status: {format}"
    );
    for flag in [
        "conflicted",
        "untracked",
        "modified",
        "staged",
        "renamed",
        "deleted",
        "stashed",
    ] {
        assert!(
            !format.contains(flag),
            "granular flag '{flag}' should not remain: {format}"
        );
    }
    assert!(
        format.contains("$ahead_behind"),
        "ahead_behind is preserved: {format}"
    );
}

#[test]
fn translate_git_granular_flags_warn_once() {
    let branch = table("style = \"bold purple\"\nsymbol = \" \"\n");
    let status = table(
        "format = '[[(*$conflicted$untracked$modified$staged$renamed$deleted)](218) ($ahead_behind$stashed)]($style)'\nstyle = \"bold red\"\n",
    );
    let mut warnings = Warnings::new();
    let _theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.message.contains("granular"))
            .count(),
        1_usize,
        "exactly one collapsed granular warning, not one per flag: {warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.message.contains("has no GPY equivalent; dropped")),
        "granular flags must not fall through to the per-var dropped warning: {warnings:?}"
    );
}

#[test]
fn translate_git_collapse_spans_bracket_groups() {
    // The collapse is global across bracket groups.
    let branch = table("style = \"bold purple\"\nsymbol = \" \"\n");
    let status = table(
        "format = '[$modified$staged]($style) ($ahead_behind$stashed)'\nstyle = \"bold red\"\n",
    );
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert_eq!(
        format.matches("$status").count(),
        1_usize,
        "collapse-to-first spans bracket groups: {format}"
    );
    assert!(!format.contains("modified"));
    assert!(!format.contains("staged"));
    assert!(!format.contains("stashed"));
    assert!(format.contains("$ahead_behind"));
}

#[test]
fn translate_git_warns_on_git_state() {
    let branch = table("style = \"bold purple\"\nsymbol = \" \"\n");
    let state =
        table("format = \"\\\\([$state( $progress_current/$progress_total)]($style)\\\\) \"\n");
    let mut warnings = Warnings::new();
    let _theme = translate_git(Some(&branch), None, Some(&state), &mut warnings);
    assert!(
        warnings.iter().any(|w| w.message.contains("git_state")),
        "git_state should warn as lossy"
    );
}

#[test]
fn translate_git_fills_missing_half_with_starship_defaults() {
    let branch = table("symbol = \"x\"\n");
    let mut warnings = Warnings::new();
    let branch_only = translate_git(Some(&branch), None, None, &mut warnings);
    assert_eq!(
        branch_only.format.as_deref(),
        Some(r"on [x$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )")
    );

    let status = table("style = \"red\"\n");
    let status_only = translate_git(None, Some(&status), None, &mut warnings);
    assert_eq!(
        status_only.format.as_deref(),
        Some(r"on [($symbol )$branch](bold purple) ([\[$status$ahead_behind\]](red) )")
    );

    let neither = translate_git(None, None, None, &mut warnings);
    assert_eq!(neither.format, None);
}

#[test]
fn default_branch_symbol_keeps_the_glyph_apart_from_the_name() {
    // Starship's own default symbol is the glyph plus a space; GPY's `$symbol` is the glyph alone.
    let branch = table("symbol = \"\\ue0a0 \"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), None, None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert!(format.starts_with("on [($symbol )$branch]"), "{format}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn glyph_only_branch_symbol_is_left_to_the_format_spacing() {
    let branch = table("symbol = \"\\ue0a0\"\nformat = '[$symbol $branch]($style)'\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), None, None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert!(
        format.starts_with("[$symbol $branch](bold purple)"),
        "{format}"
    );
}

#[test]
fn custom_branch_symbol_is_inlined() {
    let branch = table("symbol = \"git:\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), None, None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert!(
        format.starts_with("on [git:$branch](bold purple)"),
        "{format}"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn plain_status_symbols_become_theme_icons() {
    let status =
        table("modified = \"~\"\nstaged = \"S\"\nuntracked = \"\\\\?\"\nconflicted = \"\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(None, Some(&status), None, &mut warnings);
    assert_eq!(theme.unstaged_icon.as_deref(), Some("~"));
    assert_eq!(theme.staged_icon.as_deref(), Some("S"));
    assert_eq!(theme.untracked_icon.as_deref(), Some("?"));
    assert_eq!(theme.conflicts_icon.as_deref(), Some(""));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn status_symbols_absent_leave_icons_unset() {
    let mut warnings = Warnings::new();
    let theme = translate_git(None, Some(&table("")), None, &mut warnings);
    assert_eq!(theme.unstaged_icon, None);
    assert_eq!(theme.staged_icon, None);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn status_symbols_with_counts_or_styling_warn_and_are_skipped() {
    let status = table("modified = \"!$count\"\nstaged = \"[+](green)\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(None, Some(&status), None, &mut warnings);
    assert_eq!(theme.unstaged_icon, None);
    assert_eq!(theme.staged_icon, None);
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("modified")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("staged")),
        "{messages:?}"
    );
}

#[test]
fn status_symbols_without_a_theme_slot_warn_once() {
    let status = table("ahead = \">\"\nbehind = \"<\"\nrenamed = \"r\"\nstashed = \"s\"\n");
    let mut warnings = Warnings::new();
    let _theme = translate_git(None, Some(&status), None, &mut warnings);
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages.len(), 1_usize, "{messages:?}");
    for key in ["ahead", "behind", "renamed", "stashed"] {
        assert!(
            messages.iter().any(|m| m.contains(key)),
            "{key}: {messages:?}"
        );
    }
}

#[test]
fn git_state_maps_to_the_state_variable_in_its_style() {
    let state = table("style = \"bold red\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(None, None, Some(&state), &mut warnings);
    let format = theme.format.expect("git format produced");
    assert!(
        format.contains(r"([\($state\)](bold red) )"),
        "state sits between the branch and status parts: {format}"
    );
    assert!(
        format.find("$branch") < format.find("$state")
            && format.find("$state") < format.find("$status"),
        "{format}"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn git_state_labels_and_format_warn_as_not_carried_over() {
    let state = table("rebase = \"R\"\nformat = '[$state]($style)'\n");
    let mut warnings = Warnings::new();
    let _theme = translate_git(None, None, Some(&state), &mut warnings);
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(messages.len(), 1_usize, "{messages:?}");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("git_state") && m.contains("rebase"))
    );
}

#[test]
fn status_symbol_with_control_characters_warns_and_is_skipped() {
    let status = table("modified = \"\\u0007\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(None, Some(&status), None, &mut warnings);
    assert_eq!(theme.unstaged_icon, None);
    assert!(warnings.iter().any(|w| w.message.contains("modified")));
}
