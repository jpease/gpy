#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use super::{Token, inline_style_vars, retain_known_vars, tokens, translate_directory};
use crate::config::LanguageTheme;
use crate::config::types::{ColorSpec, DirectoryTruncationLength, DirectoryTruncationSymbol};
use crate::import::starship::Warnings;

/// No selected Starship palette.
const NO_PALETTE: Option<&std::collections::HashMap<String, String>> = None;

fn table(toml_src: &str) -> toml::value::Table {
    toml::from_str(toml_src).unwrap()
}

#[test]
fn tokens_bare_var() {
    assert_eq!(
        tokens("$x"),
        vec![Token::Var {
            name: "x",
            raw: "$x"
        }]
    );
}

#[test]
fn tokens_braced_var() {
    assert_eq!(
        tokens("${x}"),
        vec![Token::Var {
            name: "x",
            raw: "${x}"
        }]
    );
}

#[test]
fn tokens_escaped_dollar_stays_literal() {
    // The `$` after a backslash is literal, not a variable.
    assert_eq!(tokens(r"\$"), vec![Token::Literal(r"\$")]);
    assert_eq!(
        tokens(r"a\$b"),
        vec![Token::Literal(r"a\$b")],
        "escape doesn't split the surrounding literal run"
    );
}

#[test]
fn tokens_braced_var_followed_by_literal_braces() {
    // Literal braces after a braced var are not part of it.
    assert_eq!(
        tokens("${a}{b}"),
        vec![
            Token::Var {
                name: "a",
                raw: "${a}"
            },
            Token::Literal("{b}"),
        ]
    );
}

#[test]
fn tokens_unclosed_brace_falls_back_to_literal_dollar() {
    // Unclosed `${`: the rest, including `$`, is literal.
    assert_eq!(tokens("${unclosed"), vec![Token::Literal("${unclosed")]);
}

#[test]
fn tokens_trailing_bare_dollar_falls_back_to_literal() {
    assert_eq!(
        tokens("abc$"),
        vec![Token::Literal("abc$")],
        "trailing bare $ merges into the literal run"
    );
}

#[test]
fn tokens_dollar_followed_by_punctuation_is_malformed() {
    assert_eq!(tokens("$!"), vec![Token::Literal("$!")]);
}

#[test]
fn tokens_handles_multibyte_literal_text() {
    assert_eq!(
        tokens("[❯]($style)"),
        vec![
            Token::Literal("[❯]("),
            Token::Var {
                name: "style",
                raw: "$style"
            },
            Token::Literal(")"),
        ]
    );
}

#[test]
fn inlines_style_and_read_only_style() {
    let module = table("style = \"bold cyan\"\nread_only_style = \"bold red\"\n");
    let out = inline_style_vars("[$path]($style)[$read_only]($read_only_style) ", &module);
    assert_eq!(out, "[$path](bold cyan)[$read_only](bold red) ");
}

#[test]
fn retains_known_and_drops_unknown_vars() {
    let mut warnings = Warnings::new();
    let out = retain_known_vars(
        "[$path]($before_repo_root_style) $read_only",
        &["path", "read_only"],
        "directory",
        &mut warnings,
    );
    assert!(out.contains("$path"));
    assert!(out.contains("$read_only"));
    assert!(!out.contains("before_repo_root_style"));
    assert_eq!(warnings.len(), 1_usize);
}

#[test]
fn translate_directory_matches_starship_default_shape() {
    let module = table(
        "format = \"[$path]($style)[$read_only]($read_only_style) \"\nstyle = \"bold cyan\"\n",
    );
    let mut warnings = Warnings::new();
    let theme = translate_directory(&module, &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("[$path](bold cyan)[$read_only](red) ")
    );
}

#[test]
fn directory_without_style_uses_starship_default() {
    let mut warnings = Warnings::new();
    let theme = translate_directory(&table("truncation_length = 3\n"), &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("[$path](bold cyan)[$read_only](red) ")
    );
}

#[test]
fn directory_style_wins_over_default() {
    let mut warnings = Warnings::new();
    let theme = translate_directory(
        &table("style = \"bold blue\"\nread_only_style = \"bold red\"\n"),
        &mut warnings,
    );
    assert_eq!(
        theme.format.as_deref(),
        Some("[$path](bold blue)[$read_only](bold red) ")
    );
}

#[test]
fn duration_without_style_uses_starship_default() {
    use super::translate_duration;

    let mut warnings = Warnings::new();
    let theme = translate_duration(&table("min_time = 500\n"), &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("took [$duration](bold yellow) ")
    );
}

#[test]
fn duration_style_wins_over_default() {
    use super::translate_duration;

    let mut warnings = Warnings::new();
    let theme = translate_duration(&table("style = \"bold red\"\n"), &mut warnings);
    assert_eq!(theme.format.as_deref(), Some("took [$duration](bold red) "));
}

#[test]
fn hostname_without_style_uses_starship_default() {
    use super::translate_hostname;

    let mut warnings = Warnings::new();
    let theme = translate_hostname(&table("ssh_only = false\n"), &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("[$symbol$hostname](bold dimmed green) in ")
    );
    assert_eq!(theme.icon.as_deref(), Some("🌐 "), "Starship's default");
    assert!(warnings.is_empty());
}

#[test]
fn translate_directory_layout_carries_truncation_settings() {
    use super::translate_directory_layout;
    let module = table("truncation_length = 3\ntruncation_symbol = \"…\"\n");
    let mut warnings = Warnings::new();
    let layout = translate_directory_layout(&module, &mut warnings);
    assert_eq!(
        layout.truncation_length.map(DirectoryTruncationLength::get),
        Some(3_usize)
    );
    assert_eq!(
        layout
            .truncation_symbol
            .as_ref()
            .map(DirectoryTruncationSymbol::as_str),
        Some("…")
    );
    assert!(warnings.is_empty());
}

#[test]
fn translate_directory_layout_absent_stays_none() {
    use super::translate_directory_layout;
    let module = table("");
    let mut warnings = Warnings::new();
    let layout = translate_directory_layout(&module, &mut warnings);
    assert!(layout.truncation_length.is_none());
    assert!(layout.truncation_symbol.is_none());
    assert!(warnings.is_empty());
}

#[test]
fn translate_directory_layout_always_recommends_starships_default_behavior() {
    use super::translate_directory_layout;
    use crate::config::types::DirectoryDisplay;

    // Pure Preset sets only `style`, yet Starship's default truncation still applies.
    let module = table("style = \"bold blue\"\n");
    let mut warnings = Warnings::new();
    let layout = translate_directory_layout(&module, &mut warnings);
    assert_eq!(layout.display, Some(DirectoryDisplay::Truncated));
    assert_eq!(layout.truncate_to_repo, Some(true));
    assert!(warnings.is_empty());
}

#[test]
fn translate_directory_layout_warns_on_out_of_range_length() {
    use super::translate_directory_layout;
    let module = table("truncation_length = 300\n");
    let mut warnings = Warnings::new();
    let layout = translate_directory_layout(&module, &mut warnings);
    assert!(layout.truncation_length.is_none());
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("truncation_length"))
    );
}

#[test]
fn translate_directory_layout_warns_on_invalid_symbol() {
    use super::translate_directory_layout;
    let module = table("truncation_symbol = \"bad\\nsymbol\"\n");
    let mut warnings = Warnings::new();
    let layout = translate_directory_layout(&module, &mut warnings);
    assert!(layout.truncation_symbol.is_none());
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("truncation_symbol"))
    );
}

#[test]
fn translate_git_composes_branch_and_status() {
    use super::translate_git;
    let branch = table("symbol = \" \"\nstyle = \"bold purple\"\n");
    let status = table("style = \"bold red\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_git(Some(&branch), Some(&status), None, &mut warnings);
    let format = theme.format.expect("git format produced");
    assert_eq!(
        format,
        r"on [$symbol$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )"
    );
}

#[test]
fn git_branch_and_status_concatenate_without_separator() {
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
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
    use super::translate_git;
    let branch = table("symbol = \"x\"\n");
    let mut warnings = Warnings::new();
    let branch_only = translate_git(Some(&branch), None, None, &mut warnings);
    assert_eq!(
        branch_only.format.as_deref(),
        Some(r"on [$symbol$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )")
    );

    let status = table("style = \"red\"\n");
    let status_only = translate_git(None, Some(&status), None, &mut warnings);
    assert_eq!(
        status_only.format.as_deref(),
        Some(r"on [$symbol$branch](bold purple) ([\[$status$ahead_behind\]](red) )")
    );

    let neither = translate_git(None, None, None, &mut warnings);
    assert_eq!(neither.format, None);
}

#[test]
fn style_color_skips_attributes_and_strips_prefixes() {
    use super::{StyleColor, style_color};
    let resolved = |fg: Option<&str>, bg: Option<&str>| StyleColor::Resolved {
        fg: fg.map(str::to_owned),
        bg: bg.map(str::to_owned),
    };
    assert_eq!(
        style_color("bold green", NO_PALETTE),
        resolved(Some("green"), None)
    );
    assert_eq!(
        style_color("fg:green", NO_PALETTE),
        resolved(Some("green"), None)
    );
    assert_eq!(
        style_color("Bold FG:Green", NO_PALETTE),
        resolved(Some("green"), None)
    );
    assert_eq!(
        style_color("fg:#ff0000", NO_PALETTE),
        resolved(Some("#ff0000"), None)
    );
    assert_eq!(
        style_color("bold fg:202", NO_PALETTE),
        resolved(Some("202"), None)
    );
    assert_eq!(
        style_color("bg:blue fg:white", NO_PALETTE),
        resolved(Some("white"), Some("blue"))
    );
    assert_eq!(
        style_color("bg:#212736", NO_PALETTE),
        resolved(None, Some("#212736"))
    );
    assert_eq!(style_color("bold", NO_PALETTE), resolved(None, None));
    assert_eq!(style_color("none", NO_PALETTE), resolved(None, None));
}

#[test]
fn style_color_resolves_palette_aliases_and_rejects_unknown_names() {
    use super::{StyleColor, style_color};
    let palette: std::collections::HashMap<String, String> = [
        ("peach".to_owned(), "#fab387".to_owned()),
        ("green".to_owned(), "#00ff00".to_owned()),
    ]
    .into();
    assert_eq!(
        style_color("bold peach", Some(&palette)),
        StyleColor::Resolved {
            fg: Some("#fab387".to_owned()),
            bg: None
        }
    );
    assert_eq!(
        style_color("bg:peach", Some(&palette)),
        StyleColor::Resolved {
            fg: None,
            bg: Some("#fab387".to_owned())
        }
    );
    // Shadowing a builtin color name is the palette vocabulary's concern, not an alias.
    assert_eq!(
        style_color("green", Some(&palette)),
        StyleColor::Resolved {
            fg: Some("green".to_owned()),
            bg: None
        }
    );
    assert_eq!(
        style_color("peach", NO_PALETTE),
        StyleColor::Invalid("peach".to_owned())
    );
    assert_eq!(
        style_color("fg:#12345", NO_PALETTE),
        StyleColor::Invalid("#12345".to_owned())
    );
}

#[test]
fn canonical_language_maps_aliases() {
    use super::canonical_language;
    assert_eq!(canonical_language("nodejs"), Some("node"));
    assert_eq!(canonical_language("golang"), Some("go"));
    assert_eq!(canonical_language("rust"), Some("rust"));
    assert_eq!(canonical_language("aws"), None);
}

#[test]
fn translate_languages_collects_colors_and_warns_on_symbol() {
    use super::translate_languages;
    use crate::import::starship::model::parse;
    let model =
        parse("[rust]\nsymbol = \" \"\nstyle = \"bold red\"\n[python]\nstyle = \"bold yellow\"\n")
            .unwrap();
    let mut warnings = Warnings::new();
    let result = translate_languages(&model, LanguageTheme::default(), NO_PALETTE, &mut warnings);
    assert_eq!(
        result
            .theme
            .overrides
            .get("rust_bg_color")
            .map(ColorSpec::as_str),
        Some("red")
    );
    assert_eq!(
        result
            .theme
            .overrides
            .get("python_bg_color")
            .map(ColorSpec::as_str),
        Some("yellow")
    );
    assert_eq!(
        result.palette_colors.get("rust").map(String::as_str),
        Some("red")
    );
    assert!(warnings.iter().any(|w| w.message.contains("symbol")));
}

#[test]
fn attr_tokens_returns_attributes_only() {
    use super::attr_tokens;
    assert_eq!(attr_tokens("red dimmed"), vec!["dimmed"]);
    assert_eq!(attr_tokens("bold italic red"), vec!["bold", "italic"]);
    assert_eq!(attr_tokens("red"), Vec::<&str>::new());
    assert_eq!(attr_tokens("bold"), vec!["bold"]);
}

#[test]
fn attr_tokens_are_case_insensitive() {
    use super::attr_tokens;
    assert_eq!(attr_tokens("Bold green"), vec!["bold"]);
    assert_eq!(
        attr_tokens("ITALIC Underline red"),
        vec!["italic", "underline"]
    );
}

#[test]
fn styled_symbol_capitalized_bold_is_bold() {
    let (_, _, bold) = super::parse_styled_symbol("[❯](Bold green)").expect("parses");
    assert!(bold);
}

#[test]
fn translate_languages_captures_non_bold_attributes() {
    use super::translate_languages;
    use crate::import::starship::model::parse;
    // java: red dimmed → java_style = "dimmed"; rust: bold red → no _style (default);
    // go: plain red → go_style = "" (suppresses bold).
    let model = parse(
        "[java]\nstyle = \"red dimmed\"\n[rust]\nstyle = \"bold red\"\n[golang]\nstyle = \"red\"\n",
    )
    .expect("parse");
    let mut warnings = Warnings::new();
    let result = translate_languages(&model, LanguageTheme::default(), NO_PALETTE, &mut warnings);
    assert_eq!(
        result.theme.styles.get("java_style").map(String::as_str),
        Some("dimmed")
    );
    assert!(!result.theme.styles.contains_key("rust_style"));
    assert_eq!(
        result.theme.styles.get("go_style").map(String::as_str),
        Some("")
    );
}

#[test]
fn translate_languages_overlays_only_configured_languages_on_base() {
    use super::translate_languages;
    use crate::import::starship::model::parse;
    let mut base = LanguageTheme {
        format: Some("via [$symbol]($attr fg:$color) ".to_owned()),
        ..LanguageTheme::default()
    };
    for (key, color) in [("java_bg_color", "red"), ("swift_bg_color", "orange")] {
        base.overrides
            .insert(key.to_owned(), ColorSpec::new(color).unwrap());
    }
    base.styles
        .insert("java_style".to_owned(), "dimmed".to_owned());
    base.symbols
        .insert("java_symbol".to_owned(), "☕".to_owned());
    base.symbols
        .insert("swift_symbol".to_owned(), "🐦".to_owned());
    // A plain-bold style clears base `dimmed`; a source symbol drops the base one.
    let model = parse("[java]\nstyle = \"bold blue\"\nsymbol = \"J \"\n").expect("parse");
    let mut warnings = Warnings::new();
    let result = translate_languages(&model, base, NO_PALETTE, &mut warnings);
    assert_eq!(
        result.theme.format.as_deref(),
        Some("via [$symbol]($attr fg:$color) ")
    );
    let color = |key: &str| result.theme.overrides.get(key).map(ColorSpec::as_str);
    assert_eq!(color("java_bg_color"), Some("blue"));
    assert_eq!(color("swift_bg_color"), Some("orange"));
    assert!(!result.theme.styles.contains_key("java_style"));
    assert!(!result.theme.symbols.contains_key("java_symbol"));
    assert_eq!(
        result.theme.symbols.get("swift_symbol").map(String::as_str),
        Some("🐦")
    );
}

#[test]
fn translate_duration_maps_min_time() {
    use super::translate_duration;
    let module =
        table("min_time = 2000\nformat = \"took [$duration]($style) \"\nstyle = \"bold yellow\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_duration(&module, &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("took [$duration](bold yellow) ")
    );
    assert_eq!(theme.show_if_exceeds_ms, 2000_u64);
}

#[test]
fn translate_duration_disables_show_milliseconds() {
    use super::translate_duration;
    let module =
        table("min_time = 2000\nformat = \"took [$duration]($style) \"\nstyle = \"bold yellow\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_duration(&module, &mut warnings);
    assert!(
        !theme.show_milliseconds,
        "imported duration theme should not show millisecond precision by default"
    );
}

#[test]
fn translate_character_extracts_symbols_and_colors() {
    use super::translate_character;
    let module = table("success_symbol = \"[❯](bold green)\"\nerror_symbol = \"[❯](bold red)\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_character(&module, NO_PALETTE, &mut warnings);
    assert_eq!(theme.success_symbol, "❯");
    assert_eq!(theme.error_symbol, "❯");
    assert_eq!(theme.success_color.as_str(), "green");
    assert_eq!(theme.error_color.as_str(), "red");
    assert_eq!(theme.format.as_deref(), Some("[$symbol](bold $style) "));
}

#[test]
fn character_symbol_with_trailing_space_is_parsed() {
    use super::translate_character;
    let mut w = Warnings::new();
    let theme = translate_character(
        &table("success_symbol = \"[➜](bold green) \"\n"),
        NO_PALETTE,
        &mut w,
    );
    assert_eq!(theme.success_symbol, "➜ ");
    assert_eq!(theme.success_color.as_str(), "green");
    assert_eq!(theme.format.as_deref(), Some("[$symbol](bold $style) "));
    assert!(w.is_empty());
}

#[test]
fn character_multi_group_symbol_warns_lossy() {
    use super::translate_character;
    use crate::import::starship::WarningKind;
    use crate::theme::CharacterTheme;
    let mut w = Warnings::new();
    let theme = translate_character(
        &table("success_symbol = \"[❯](bold green)[❯](bold yellow)\"\n"),
        NO_PALETTE,
        &mut w,
    );
    assert_eq!(
        theme.success_symbol,
        CharacterTheme::default().success_symbol
    );
    let lossy = w
        .iter()
        .filter(|warning| warning.kind == WarningKind::LossyMapping)
        .count();
    assert_eq!(lossy, 1_usize);
    assert!(
        w.iter()
            .all(|warning| warning.kind != WarningKind::InvalidColor)
    );
    assert!(
        w.iter()
            .any(|warning| warning.message.contains("[❯](bold green)[❯](bold yellow)"))
    );
}

#[test]
fn character_bare_symbol_has_no_color_and_is_not_bold() {
    use super::translate_character;
    let mut w = Warnings::new();
    let theme = translate_character(
        &table("success_symbol = \"> \"\nerror_symbol = \"x \"\n"),
        NO_PALETTE,
        &mut w,
    );
    assert_eq!(theme.success_symbol, "> ");
    assert_eq!(theme.format.as_deref(), Some("[$symbol]($style) "));
}

#[test]
fn character_symbol_with_leading_and_trailing_space_is_parsed() {
    use super::translate_character;
    let mut w = Warnings::new();
    let theme = translate_character(
        &table("error_symbol = \" [✗](bold red)  \"\n"),
        NO_PALETTE,
        &mut w,
    );
    assert_eq!(theme.error_symbol, " ✗  ");
    assert_eq!(theme.error_color.as_str(), "red");
}

#[test]
fn character_symbol_with_variable_warns_lossy() {
    use super::translate_character;
    use crate::theme::CharacterTheme;
    let mut w = Warnings::new();
    let theme = translate_character(
        &table("error_symbol = \"[$x](bold red)\"\n"),
        NO_PALETTE,
        &mut w,
    );
    assert_eq!(theme.error_symbol, CharacterTheme::default().error_symbol);
    assert_eq!(w.len(), 1_usize);
}

#[test]
fn translate_hostname_inverts_ssh_only() {
    use super::translate_hostname;
    let module = table("ssh_only = false\n");
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&module, &mut warnings);
    assert!(theme.show_always, "ssh_only=false -> show_always=true");
}

#[test]
fn translate_hostname_defaults_ssh_only_to_true() {
    use super::translate_hostname;
    let module = table("");
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&module, &mut warnings);
    assert!(
        !theme.show_always,
        "ssh_only absent defaults to true -> show_always=false"
    );
}

#[test]
fn translate_hostname_maps_ssh_symbol_to_icon() {
    use super::translate_hostname;
    let module = table("ssh_symbol = \"🌐 \"\nstyle = \"bold dimmed green\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&module, &mut warnings);
    assert_eq!(theme.icon.as_deref(), Some("🌐 "));
    assert!(warnings.is_empty());
}

#[test]
fn translate_hostname_defaults_ssh_symbol_to_globe_when_ssh_only() {
    use super::translate_hostname;
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&table(""), &mut warnings);
    assert_eq!(theme.icon.as_deref(), Some("🌐 "), "Starship's default");
}

#[test]
fn translate_hostname_keeps_explicit_ssh_symbol_over_default() {
    use super::translate_hostname;
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&table("ssh_symbol = \"@\"\n"), &mut warnings);
    assert_eq!(theme.icon.as_deref(), Some("@"));
}

/// SSH gating happens agent-side, so the default globe is always imported.
#[test]
fn translate_hostname_defaults_ssh_symbol_to_globe_when_not_ssh_only() {
    use super::translate_hostname;
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&table("ssh_only = false\n"), &mut warnings);
    assert_eq!(theme.icon.as_deref(), Some("🌐 "), "Starship's default");
}

#[test]
fn translate_hostname_passes_through_trim_at() {
    use super::translate_hostname;
    let mut warnings = Warnings::new();

    let custom = table("trim_at = \"-\"\n");
    assert_eq!(translate_hostname(&custom, &mut warnings).trim_at, "-");

    let absent = table("");
    assert_eq!(translate_hostname(&absent, &mut warnings).trim_at, ".");
}

#[test]
fn translate_hostname_renames_ssh_symbol_and_inlines_style() {
    use super::translate_hostname;
    let module =
        table("format = \"[$ssh_symbol$hostname]($style) in \"\nstyle = \"bold dimmed green\"\n");
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&module, &mut warnings);
    assert_eq!(
        theme.format.as_deref(),
        Some("[$symbol$hostname](bold dimmed green) in ")
    );
    assert!(warnings.is_empty());
}

#[test]
fn translate_hostname_drops_unknown_vars() {
    use super::translate_hostname;
    let module = table("format = \"[$ssh_symbol$hostname$sep_close]($style) \"\n");
    let mut warnings = Warnings::new();
    let theme = translate_hostname(&module, &mut warnings);
    let format = theme.format.expect("hostname format produced");
    assert!(format.contains("$symbol"));
    assert!(format.contains("$hostname"));
    assert!(!format.contains("sep_close"));
    assert!(warnings.iter().any(|w| w.message.contains("sep_close")));
}

#[test]
fn translate_username_maps_show_always() {
    use super::translate_username;
    let mut warnings = Warnings::new();

    let shown = table("show_always = true\n");
    assert!(translate_username(&shown, &mut warnings).show_always);

    let hidden = table("");
    assert!(
        !translate_username(&hidden, &mut warnings).show_always,
        "show_always absent defaults to false (root/sudo/SSH only)"
    );
}

#[test]
fn translate_username_renames_user_and_inlines_root_and_user_styles() {
    use super::translate_username;
    let module = table(
        "format = \"[$user]($style) in \"\nstyle_root = \"bold red\"\nstyle_user = \"green\"\n",
    );
    let mut warnings = Warnings::new();
    let theme = translate_username(&module, &mut warnings);
    assert_eq!(theme.format.as_deref(), Some("[$username](bold red) in "));
    assert_eq!(theme.user_format.as_deref(), Some("[$username](green) in "));
    assert!(warnings.is_empty());
}

#[test]
fn translate_username_defaults_styles_when_absent() {
    use super::translate_username;
    let module = table("");
    let mut warnings = Warnings::new();
    let theme = translate_username(&module, &mut warnings);
    assert_eq!(theme.format.as_deref(), Some("[$username](bold red) in "));
    assert_eq!(
        theme.user_format.as_deref(),
        Some("[$username](bold yellow) in ")
    );
}

#[test]
fn translate_username_drops_unknown_vars_and_warns_once() {
    use super::translate_username;
    let module = table("format = \"[$user$sep_close]($style) \"\n");
    let mut warnings = Warnings::new();
    let theme = translate_username(&module, &mut warnings);
    for produced in [theme.format, theme.user_format] {
        let format = produced.expect("username format produced");
        assert!(format.contains("$username"));
        assert!(!format.contains("sep_close"));
    }
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.message.contains("sep_close"))
            .count(),
        1
    );
}

#[test]
fn translate_time_maps_strftime_best_effort() {
    use super::translate_time;
    let twelve = table("time_format = \"%I:%M %p\"\n");
    let mut warnings = Warnings::new();
    let clock = translate_time(&twelve, &mut warnings);
    assert_eq!(clock.time_format.as_deref(), Some("12"));

    let twenty_four = table("time_format = \"%H:%M:%S\"\n");
    let clock_24 = translate_time(&twenty_four, &mut warnings);
    assert_eq!(clock_24.time_format.as_deref(), Some("24"));
    assert_eq!(clock_24.show_seconds, Some(true));
}
