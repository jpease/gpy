//! Pure per-module translators: a Starship module table → a GPY segment theme.

use crate::config::LanguageTheme;
use crate::config::types::{ColorSpec, DirectoryTruncationLength, DirectoryTruncationSymbol, Icon};
use crate::import::starship::model::StarshipConfig;
use crate::import::starship::{WarningKind, Warnings};
use crate::template::ast::Node;
use crate::theme::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitTheme, HostnameTheme,
    RecommendedDirectory, UsernameTheme,
};
use std::collections::BTreeMap;

/// Replace style-position variables (`$style`, `$*_style`) in `format` with the
/// module's literal style values. Content variables are left untouched.
#[must_use]
pub fn inline_style_vars(format: &str, table: &toml::value::Table) -> String {
    let mut style_keys: Vec<(&String, &str)> = table
        .iter()
        .filter(|(key, _)| key.ends_with("style"))
        .filter_map(|(key, value)| value.as_str().map(|text| (key, text)))
        .collect();
    // Longest key first so `$read_only_style` is replaced before `$style`.
    style_keys.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    let mut out = format.to_owned();
    for (key, style) in style_keys {
        out = out.replace(&format!("${{{key}}}"), style);
        out = out.replace(&format!("${key}"), style);
    }
    out
}

/// One lexical unit of a Starship format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// Literal text, verbatim — this includes backslash-escape sequences
    /// (`\$`, `\\`, etc.) exactly as they appeared in the source, since every
    /// existing consumer re-emits an escape untouched without inspecting the
    /// escaped character.
    Literal(&'a str),
    /// A `$name` or `${name}` variable reference.
    Var {
        /// The bare variable name (no `$`/braces).
        name: &'a str,
        /// The token's original source text, e.g. `$name` or `${name}`, for
        /// verbatim re-emission by a consumer that decides to keep it.
        raw: &'a str,
    },
}

/// Tokenize a Starship format string into literal runs and variable
/// references. Pure, no allocation beyond the returned `Vec` and its
/// `&str` slices (all borrowed from `format`).
///
/// See [`retain_known_vars`] and `collapse_granular_status_flags` for the
/// canonical consumers, and this module's tests for the exact semantics
/// (bare `$name`, braced `${name}`, backslash escapes, and the malformed-`$`
/// fallback).
#[must_use]
pub fn tokens(format: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut literal_start = 0_usize;
    // Byte index of the next unprocessed char; `$`, `{`, `}`, `\`, and
    // identifier chars are all ASCII, so single-byte advances on those are
    // safe, while literal spans between them are pushed via string slicing
    // (which respects UTF-8 boundaries since both ends land on ASCII markers
    // or the string's own start/end).
    let mut index = 0_usize;
    let bytes_len = format.len();
    while index < bytes_len {
        // Only ASCII bytes are ever inspected directly; anything else is part
        // of a multi-byte literal char and falls through to the `_` arm.
        match format.as_bytes().get(index) {
            Some(b'\\') => {
                // A backslash plus the next char (or just the backslash, if
                // it's the last byte) is consumed as literal text without
                // inspecting what follows.
                let after_backslash = index.saturating_add(1_usize);
                let next_index = format
                    .get(after_backslash..)
                    .and_then(|rest| rest.chars().next())
                    .map_or(after_backslash, |ch| {
                        after_backslash.saturating_add(ch.len_utf8())
                    });
                index = next_index;
            }
            Some(b'$') => {
                if let Some((name, raw_end)) = scan_var(format, index) {
                    // Flush any pending literal run before the var token.
                    if literal_start < index
                        && let Some(text) = format.get(literal_start..index)
                    {
                        out.push(Token::Literal(text));
                    }
                    if let Some(raw) = format.get(index..raw_end) {
                        out.push(Token::Var { name, raw });
                    }
                    index = raw_end;
                    literal_start = index;
                } else {
                    // Malformed `$`: extend the ongoing literal run past it.
                    index = index.saturating_add(1_usize);
                }
            }
            _ => {
                let next_index = format
                    .get(index..)
                    .and_then(|rest| rest.chars().next())
                    .map_or_else(|| bytes_len, |ch| index.saturating_add(ch.len_utf8()));
                index = next_index;
            }
        }
    }
    if literal_start < bytes_len
        && let Some(text) = format.get(literal_start..bytes_len)
    {
        out.push(Token::Literal(text));
    }
    out
}

/// Scan a `$name` or `${name}` token starting at `start` (the byte index of
/// `$`).
///
/// Returns the bare name and the byte index just past the token, or `None`
/// if the `$` is malformed (not followed by `{`, and not followed by an
/// alphanumeric/underscore char) — including an unclosed `${`.
fn scan_var(format: &str, start: usize) -> Option<(&str, usize)> {
    let after_dollar = start.saturating_add(1_usize);
    if format.as_bytes().get(after_dollar) == Some(&b'{') {
        let name_start = after_dollar.saturating_add(1_usize);
        let close = format.get(name_start..)?.find('}')?;
        let name_end = name_start.saturating_add(close);
        let name = format.get(name_start..name_end)?;
        return Some((name, name_end.saturating_add(1_usize)));
    }
    let name_start = after_dollar;
    let mut name_end = name_start;
    while let Some(ch) = format.get(name_end..).and_then(|rest| rest.chars().next()) {
        if ch.is_alphanumeric() || ch == '_' {
            name_end = name_end.saturating_add(ch.len_utf8());
        } else {
            break;
        }
    }
    if name_end == name_start {
        return None;
    }
    let name = format.get(name_start..name_end)?;
    Some((name, name_end))
}

/// Drop variable references not present in `known`, warning once per dropped name.
#[must_use]
pub fn retain_known_vars(
    format: &str,
    known: &[&str],
    segment: &str,
    warnings: &mut Warnings,
) -> String {
    tokens(format)
        .into_iter()
        .map(|token| match token {
            Token::Literal(text) => text.to_owned(),
            Token::Var { name, raw } => {
                if known.contains(&name) {
                    raw.to_owned()
                } else {
                    warnings.push(
                        WarningKind::LossyMapping,
                        format!("{segment}: variable '${name}' has no GPY equivalent; dropped"),
                    );
                    String::new()
                }
            }
        })
        .collect()
}

/// Granular Starship `git_status` flag variables.
///
/// GPY has no per-flag-type status distinction, so any reference to these is
/// approximated as the aggregate `$status` (the same collapse Starship's
/// `$all_status` already receives).
const GRANULAR_STATUS_FLAGS: [&str; 7_usize] = [
    "conflicted",
    "untracked",
    "modified",
    "staged",
    "renamed",
    "deleted",
    "stashed",
];

/// Collapse granular Starship `git_status` flag variables (`$conflicted`,
/// `$untracked`, `$modified`, `$staged`, `$renamed`, `$deleted`, `$stashed`) onto
/// GPY's single aggregate `$status`.
///
/// The *first* granular token encountered (in document order) becomes `$status`;
/// every subsequent one is removed. This avoids the duplicate-rendering bug a
/// blind `str::replace` per flag name would cause for back-to-back flags like the
/// Pure Preset's `$conflicted$untracked$modified$staged$renamed$deleted`, which
/// would otherwise expand to six `$status` markers. Scanning is char-by-char
/// (mirroring [`retain_known_vars`]) so `$name`/`${name}` forms and backslash
/// escapes are honored and a flag name is never matched inside a longer variable.
///
/// Pushes exactly one [`WarningKind::LossyMapping`] warning naming the flags that
/// were collapsed, and only when at least one was present.
// `map_or_else` would need two closures each mutably borrowing `found` and
// `collapsed_first`, which the borrow checker rejects even though only one
// ever runs; the if/else keeps the shared mutable state legible.
#[allow(clippy::option_if_let_else)]
fn collapse_granular_status_flags(format: &str, warnings: &mut Warnings) -> String {
    let mut collapsed_first = false;
    let mut found: Vec<&'static str> = Vec::new();
    let out: String = tokens(format)
        .into_iter()
        .filter_map(|token| match token {
            Token::Literal(text) => Some(text.to_owned()),
            Token::Var { name, raw } => {
                if let Some(flag) = GRANULAR_STATUS_FLAGS
                    .iter()
                    .copied()
                    .find(|&candidate| candidate == name)
                {
                    // First granular token → aggregate `$status`; every later
                    // one is dropped.
                    if !found.contains(&flag) {
                        found.push(flag);
                    }
                    if collapsed_first {
                        None
                    } else {
                        collapsed_first = true;
                        Some("$status".to_owned())
                    }
                } else {
                    // Non-granular variable: re-emit the original token verbatim.
                    Some(raw.to_owned())
                }
            }
        })
        .collect();
    if !found.is_empty() {
        let list = found
            .iter()
            .map(|flag| format!("${flag}"))
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(
            WarningKind::LossyMapping,
            format!(
                "git_status: granular flags ({list}) have no per-flag GPY equivalent; approximated as aggregate $status"
            ),
        );
    }
    out
}

/// Translate Starship's `directory` module into a GPY directory theme.
#[must_use]
pub fn translate_directory(table: &toml::value::Table, warnings: &mut Warnings) -> DirectoryTheme {
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("[$path]($style)[$read_only]($read_only_style) ");
    let inlined = inline_style_vars(
        raw,
        &with_default_style(
            &with_default_style(table, "style", "bold cyan"),
            "read_only_style",
            "red",
        ),
    );
    let mapped = retain_known_vars(
        &inlined,
        &["path", "read_only", "style"],
        "directory",
        warnings,
    );
    DirectoryTheme {
        format: Some(mapped),
        ..Default::default()
    }
}

/// Translate Starship's `directory` module truncation settings into GPY's
/// advisory `[ui.recommended.directory]` layout block.
///
/// These settings live on GPY's general-config `[ui.directory]` subsystem (the
/// path resolver), not on the render-only [`DirectoryTheme`], so they are carried
/// as a recommendation applied by `gpy theme use --force` rather than baked into
/// the theme. `display` and `truncate_to_repo` are not Starship config keys —
/// they encode Starship's actual default *behavior* (always truncate to the
/// trailing path components, anchoring at the repo root when inside one), so
/// they are always recommended as `Truncated`/`true` regardless of whether the
/// source table overrides anything else. `truncation_length`/`truncation_symbol`
/// are only set when Starship's table actually specifies them; anything absent
/// stays `None`. Values outside GPY's supported range or otherwise invalid are
/// dropped with a [`WarningKind::UnrepresentableOption`] warning, never clamped.
#[must_use]
pub fn translate_directory_layout(
    table: &toml::value::Table,
    warnings: &mut Warnings,
) -> RecommendedDirectory {
    let mut recommended = RecommendedDirectory {
        display: Some(crate::config::types::DirectoryDisplay::Truncated),
        truncate_to_repo: Some(true),
        ..Default::default()
    };
    if let Some(raw) = table.get("truncation_length") {
        match raw
            .as_integer()
            .and_then(|value| usize::try_from(value).ok())
            .and_then(DirectoryTruncationLength::new)
        {
            Some(length) => recommended.truncation_length = Some(length),
            None => warnings.push(
                WarningKind::UnrepresentableOption,
                format!(
                    "directory: truncation_length {raw} is outside GPY's supported range (1..=255); dropped"
                ),
            ),
        }
    }
    if let Some(raw) = table.get("truncation_symbol") {
        match raw
            .as_str()
            .and_then(|value| DirectoryTruncationSymbol::new(value.to_owned()))
        {
            Some(symbol) => recommended.truncation_symbol = Some(symbol),
            None => warnings.push(
                WarningKind::UnrepresentableOption,
                format!(
                    "directory: truncation_symbol {raw} is invalid (must be a string without control characters); dropped"
                ),
            ),
        }
    }
    recommended
}

/// Compose a single GPY git `format` from Starship's `git_branch`,
/// `git_status`, and `git_state` modules.
///
/// GPY git vocabulary: `$symbol $branch $status $ahead_behind $style`.
/// Starship `$all_status` and `${all_status}` are remapped to `$status`; `$remote_branch`,
/// `$remote_name`, and any `git_state`-only variable are dropped with a
/// [`WarningKind::LossyMapping`] warning.
///
/// When only one of `branch` / `status` is present, the other half is
/// translated from an empty table, so Starship's default `format` and `style`
/// for that module still apply (Starship renders both modules by default).
/// When both are absent the returned `format` is `None`; the caller keeps the
/// preset's git rendering in that case.
#[must_use]
pub fn translate_git(
    branch: Option<&toml::value::Table>,
    status: Option<&toml::value::Table>,
    state: Option<&toml::value::Table>,
    warnings: &mut Warnings,
) -> GitTheme {
    let empty = toml::value::Table::new();
    let (branch_table, status_table) = if branch.is_none() && status.is_none() {
        (None, None)
    } else {
        (
            Some(branch.unwrap_or(&empty)),
            Some(status.unwrap_or(&empty)),
        )
    };

    let branch_part = branch_table.map_or_else(String::new, |table| {
        let filled = with_default_style(table, "style", "bold purple");
        let raw = table
            .get("format")
            .and_then(toml::Value::as_str)
            .unwrap_or("on [$symbol$branch]($style) ");
        let inlined = inline_style_vars(raw, &filled);
        retain_known_vars(
            &inlined,
            &["symbol", "branch", "status", "ahead_behind", "style"],
            "git",
            warnings,
        )
        .replace("(:)", "")
    });

    let status_part = status_table.map_or_else(String::new, |table| {
        let filled = with_default_style(table, "style", "bold red");
        let raw = table
            .get("format")
            .and_then(toml::Value::as_str)
            .unwrap_or(r"([\[$all_status$ahead_behind\]]($style) )");
        let inlined = inline_style_vars(raw, &filled);
        // Map Starship's $all_status / ${all_status} onto GPY's $status before
        // retaining known vars. The braced form must be replaced first so the
        // unbraced replacement never partially matches inside `${all_status}`.
        let remapped = inlined
            .replace("${all_status}", "$status")
            .replace("$all_status", "$status");
        // Collapse granular per-flag variables ($conflicted, $modified, ...) onto
        // $status too, so presets that build the dirty indicator from individual
        // flags (e.g. Starship's Pure Preset) keep a working status marker instead
        // of losing it entirely. Runs before retain_known_vars so those tokens are
        // gone (mapped or removed) before it would drop them one-by-one.
        let collapsed = collapse_granular_status_flags(&remapped, warnings);
        retain_known_vars(
            &collapsed,
            &["symbol", "branch", "status", "ahead_behind", "style"],
            "git",
            warnings,
        )
    });

    if state.is_some() {
        warnings.push(
            WarningKind::LossyMapping,
            "git_state (rebase/merge progress) has no GPY equivalent; dropped".to_owned(),
        );
    }

    let format = match (branch_part.is_empty(), status_part.is_empty()) {
        (true, true) => None,
        (false, true) => Some(format!("{} ", branch_part.trim_end())),
        (true, false) => Some(status_part),
        (false, false) => Some(format!("{} {status_part}", branch_part.trim_end())),
    };
    GitTheme {
        format,
        ..Default::default()
    }
}

/// Clone `table` and insert `default` under `key` if that key is absent.
fn with_default_style(table: &toml::value::Table, key: &str, default: &str) -> toml::value::Table {
    let mut filled = table.clone();
    filled
        .entry(key.to_owned())
        .or_insert_with(|| toml::Value::String(default.to_owned()));
    filled
}

/// Result of language translation: the merged language theme plus the colors to
/// fold into the emitted palette.
#[derive(Debug, Clone, Default)]
pub struct LanguageTranslation {
    /// The merged language segment theme.
    pub theme: LanguageTheme,
    /// `canonical → concrete color` entries to add to the palette.
    pub palette_colors: BTreeMap<String, String>,
}

/// Starship language modules GPY maps onto its single `language` segment.
const LANGUAGE_MODULES: &[(&str, &str)] = &[
    ("rust", "rust"),
    ("python", "python"),
    ("nodejs", "node"),
    ("golang", "go"),
    ("java", "java"),
    ("ruby", "ruby"),
    ("php", "php"),
    ("swift", "swift"),
    ("elixir", "elixir"),
    ("c", "c"),
    ("cpp", "cpp"),
    ("csharp", "csharp"),
    ("erlang", "erlang"),
];

/// Map a Starship language module name to GPY's canonical language name.
#[must_use]
pub fn canonical_language(module_name: &str) -> Option<&'static str> {
    LANGUAGE_MODULES
        .iter()
        .find(|(starship, _)| *starship == module_name)
        .map(|(_, canonical)| *canonical)
}

/// The eight style attributes recognized in a Starship `style` string.
const STYLE_ATTRS: [&str; 8_usize] = [
    "bold",
    "italic",
    "underline",
    "dimmed",
    "inverted",
    "blink",
    "hidden",
    "strikethrough",
];

/// Return the first non-attribute token of a style string (the color), if any.
#[must_use]
pub fn first_color_token(style: &str) -> Option<&str> {
    style
        .split_whitespace()
        .find(|token| !STYLE_ATTRS.contains(token))
}

/// Return the attribute tokens of a style string (everything `first_color_token`
/// skips), in declaration order.
#[must_use]
pub fn attr_tokens(style: &str) -> Vec<&str> {
    style
        .split_whitespace()
        .filter(|token| STYLE_ATTRS.contains(token))
        .collect()
}

/// Translate all known Starship language modules onto `base`, the language
/// theme the import starts from (the builtin `starship` preset's, which
/// encodes Starship's per-language defaults).
///
/// Only languages whose module table is present are touched: a module `style`
/// replaces that language's color and attributes, and a module `symbol` drops
/// the base's symbol for that language so `[language.icons]` config applies.
#[must_use]
pub fn translate_languages(
    model: &StarshipConfig,
    base: LanguageTheme,
    warnings: &mut Warnings,
) -> LanguageTranslation {
    let mut theme = base;
    let mut palette_colors: BTreeMap<String, String> = BTreeMap::new();
    for (starship_name, canonical) in LANGUAGE_MODULES {
        let Some(table) = model.module_table(starship_name) else {
            continue;
        };
        if let Some(style) = table.get("style").and_then(toml::Value::as_str) {
            if let Some(color) = first_color_token(style) {
                match ColorSpec::new(color) {
                    Ok(spec) => {
                        theme
                            .overrides
                            .insert(format!("{canonical}_bg_color"), spec);
                        palette_colors.insert((*canonical).to_owned(), color.to_owned());
                    }
                    Err(_) => warnings.push(
                        WarningKind::InvalidColor,
                        format!(
                            "language {starship_name}: style color '{color}' is invalid; skipped"
                        ),
                    ),
                }
            }
            // Capture attribute tokens. The renderer defaults to `bold`, so only
            // emit `<lang>_style` when the attrs differ (an empty string here is
            // intentional: it suppresses the bold default for a bare-color style).
            // A plain `bold` style clears any non-bold attrs inherited from `base`.
            let attrs = attr_tokens(style);
            let style_key = format!("{canonical}_style");
            if attrs == ["bold"] {
                theme.styles.remove(&style_key);
            } else {
                theme.styles.insert(style_key, attrs.join(" "));
            }
        }
        if table.get("symbol").and_then(toml::Value::as_str).is_some() {
            theme.symbols.remove(&format!("{canonical}_symbol"));
            warnings.push(
                WarningKind::LossyMapping,
                format!("language {starship_name}: per-language symbol is set in [language.icons] config, not the prompt theme; not transferred"),
            );
        }
    }
    LanguageTranslation {
        theme,
        palette_colors,
    }
}

/// Translate Starship's `cmd_duration` module into a GPY duration theme.
#[must_use]
pub fn translate_duration(table: &toml::value::Table, warnings: &mut Warnings) -> DurationTheme {
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("took [$duration]($style) ");
    let inlined = inline_style_vars(raw, &with_default_style(table, "style", "bold yellow"));
    let mapped = retain_known_vars(&inlined, &["duration", "style"], "duration", warnings);
    let show_if_exceeds_ms = table
        .get("min_time")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(2000_u64);
    DurationTheme {
        format: Some(mapped),
        show_if_exceeds_ms,
        show_milliseconds: false,
        ..Default::default()
    }
}

/// A parsed character symbol: text, color token, and whether it is bold.
type StyledSymbol = (String, Option<String>, bool);

/// Extract the symbol text, color, and bold flag from a Starship styled symbol
/// template such as `[❯](bold green)` or `[➜](bold green) `.
///
/// Built on the real template grammar. Accepted shapes are a bare literal
/// (no color, not bold) and exactly one styled group of literal text with
/// optional whitespace-only literals around it, which are kept in the symbol.
/// Returns `None` for anything else (several groups, variables, conditionals,
/// non-whitespace text outside the group, or a template that does not parse).
fn parse_styled_symbol(template: &str) -> Option<StyledSymbol> {
    let nodes = crate::template::parse::parse(template).ok()?;
    let mut symbol = String::new();
    let mut style_spec: Option<String> = None;
    for node in &nodes {
        match node {
            Node::Literal(text) => {
                if nodes.len() > 1_usize && !text.trim().is_empty() {
                    return None;
                }
                symbol.push_str(text);
            }
            Node::Styled {
                style_spec: spec,
                children,
            } => {
                if style_spec.is_some() {
                    return None;
                }
                style_spec = Some(spec.clone());
                for child in children {
                    let Node::Literal(text) = child else {
                        return None;
                    };
                    symbol.push_str(text);
                }
            }
            Node::Var(_) | Node::Conditional(_) => return None,
        }
    }
    let Some(spec) = style_spec else {
        return Some((symbol, None, false));
    };
    let color = first_color_token(&spec).map(str::to_owned);
    let bold = attr_tokens(&spec).contains(&"bold");
    Some((symbol, color, bold))
}

/// Translate Starship's `character` module into a GPY character theme.
///
/// GPY renders success and error through one shared `format` template, so the
/// `bold` attribute can only be applied uniformly. It is derived from the
/// source's `success_symbol`/`error_symbol` styles: bold unless both styles
/// explicitly omit it (an absent symbol keeps Starship's real default of
/// bold, matching [`CharacterTheme`]'s own default colors). A source that
/// requests bold on one side but not the other emits a
/// [`WarningKind::LossyMapping`] warning, since GPY cannot represent that split.
#[must_use]
pub fn translate_character(table: &toml::value::Table, warnings: &mut Warnings) -> CharacterTheme {
    let mut theme = CharacterTheme::default();
    let mut success_bold = true;
    if let Some(raw) = table.get("success_symbol").and_then(toml::Value::as_str) {
        if let Some((symbol, color, bold)) = parse_styled_symbol(raw) {
            theme.success_symbol = symbol;
            success_bold = bold;
            if let Some(color_name) = color {
                match ColorSpec::new(&color_name) {
                    Ok(spec) => theme.success_color = spec,
                    Err(_) => warnings.push(
                        WarningKind::InvalidColor,
                        format!("character success color '{color_name}' is invalid; kept default"),
                    ),
                }
            }
        } else {
            warnings.push(
                WarningKind::LossyMapping,
                format!("character: success_symbol '{raw}' cannot be represented; kept default"),
            );
        }
    }
    let mut error_bold = true;
    if let Some(raw) = table.get("error_symbol").and_then(toml::Value::as_str) {
        if let Some((symbol, color, bold)) = parse_styled_symbol(raw) {
            theme.error_symbol = symbol;
            error_bold = bold;
            if let Some(color_name) = color {
                match ColorSpec::new(&color_name) {
                    Ok(spec) => theme.error_color = spec,
                    Err(_) => warnings.push(
                        WarningKind::InvalidColor,
                        format!("character error color '{color_name}' is invalid; kept default"),
                    ),
                }
            }
        } else {
            warnings.push(
                WarningKind::LossyMapping,
                format!("character: error_symbol '{raw}' cannot be represented; kept default"),
            );
        }
    }
    if success_bold != error_bold {
        warnings.push(
            WarningKind::LossyMapping,
            "character: success/error styles disagree on bold; GPY renders both through one shared format and kept bold".to_owned(),
        );
    }
    let bold = success_bold || error_bold;
    theme.format = Some(if bold {
        "[$symbol](bold $style) ".to_owned()
    } else {
        "[$symbol]($style) ".to_owned()
    });
    theme
}

/// Translate Starship's `time` module into a GPY clock theme (best-effort).
#[must_use]
pub fn translate_time(table: &toml::value::Table, warnings: &mut Warnings) -> ClockTheme {
    let mut theme = ClockTheme::default();
    let Some(format) = table.get("time_format").and_then(toml::Value::as_str) else {
        return theme;
    };
    let twelve_hour = format.contains("%I") || format.contains("%p");
    theme.time_format = Some(if twelve_hour {
        "12".to_owned()
    } else {
        "24".to_owned()
    });
    theme.show_seconds = Some(format.contains("%S"));
    if !format.contains("%H") && !format.contains("%I") {
        warnings.push(
            WarningKind::LossyMapping,
            format!("time: time_format '{format}' mapped only to coarse 12/24-hour clock"),
        );
    }
    theme
}

/// Translate Starship's `hostname` module into a GPY hostname theme.
///
/// GPY hostname vocabulary: `$hostname $symbol` (no `$style` — see
/// [`crate::formatter::hostname_resolver::HostnameResolver`]). Starship's
/// `$ssh_symbol` is renamed to `$symbol` and `ssh_only` is inverted onto
/// `show_always`, since GPY's agent has no SSH knowledge (#259) and always
/// renders the symbol when the segment renders; SSH-gating happens in the
/// shell's `segment_hostname_detect` instead.
#[must_use]
pub fn translate_hostname(table: &toml::value::Table, warnings: &mut Warnings) -> HostnameTheme {
    let mut theme = HostnameTheme {
        show_always: !table
            .get("ssh_only")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        ..Default::default()
    };
    if let Some(raw_icon) = table.get("ssh_symbol").and_then(toml::Value::as_str) {
        match Icon::new(raw_icon) {
            Ok(icon) => theme.icon = Some(icon),
            Err(_) => warnings.push(
                WarningKind::UnrepresentableOption,
                format!("hostname: ssh_symbol '{raw_icon}' is invalid; skipped"),
            ),
        }
    }
    if let Some(trim_at) = table.get("trim_at").and_then(toml::Value::as_str) {
        trim_at.clone_into(&mut theme.trim_at);
    }
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("[$ssh_symbol$hostname]($style) in ");
    // Braced form first so the unbraced replacement never partially matches
    // inside `${ssh_symbol}` (same precaution as `translate_git`'s all_status).
    let renamed = raw
        .replace("${ssh_symbol}", "${symbol}")
        .replace("$ssh_symbol", "$symbol");
    let inlined = inline_style_vars(
        &renamed,
        &with_default_style(table, "style", "bold dimmed green"),
    );
    let mapped = retain_known_vars(&inlined, &["hostname", "symbol"], "hostname", warnings);
    theme.format = Some(mapped);
    theme
}

/// Translate Starship's `username` module into a GPY username theme.
///
/// GPY username vocabulary: `$username $symbol` (no `$style` — see
/// [`crate::formatter::username_resolver::UsernameResolver`]). Starship's
/// `$user` is renamed to `$username`. GPY renders this segment only when
/// root/sudo (gated shell-side by `segment_username_detect`), so `$style` is
/// inlined from Starship's `style_root` (default `bold red`), not the
/// context-switching `$style` variable GPY's resolver has no concept of.
/// Starship's `show_always` maps directly onto GPY's `show_always`.
#[must_use]
pub fn translate_username(table: &toml::value::Table, warnings: &mut Warnings) -> UsernameTheme {
    let mut theme = UsernameTheme {
        show_always: table
            .get("show_always")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        ..Default::default()
    };
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("[$user]($style) ");
    // Braced form first so the unbraced replacement never partially matches
    // inside `${user}` (same precaution as `translate_hostname`).
    let renamed = raw
        .replace("${user}", "${username}")
        .replace("$user", "$username");
    // Inline `$style` from `style_root` before the generic `*_style` pass so it
    // never survives to `retain_known_vars` as an unknown (dropped) variable.
    let style_root = table
        .get("style_root")
        .and_then(toml::Value::as_str)
        .unwrap_or("bold red");
    let styled = renamed
        .replace("${style}", style_root)
        .replace("$style", style_root);
    let inlined = inline_style_vars(&styled, table);
    let mapped = retain_known_vars(&inlined, &["username", "symbol"], "username", warnings);
    theme.format = Some(mapped);
    theme
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{Token, inline_style_vars, retain_known_vars, tokens, translate_directory};
    use crate::config::LanguageTheme;
    use crate::config::types::{ColorSpec, DirectoryTruncationLength, DirectoryTruncationSymbol};
    use crate::import::starship::Warnings;

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
        // Both the backslash and the `$` survive as literal text; the `$` is
        // not treated as the start of a variable reference.
        assert_eq!(tokens(r"\$"), vec![Token::Literal(r"\$")]);
        assert_eq!(
            tokens(r"a\$b"),
            vec![Token::Literal(r"a\$b")],
            "escape doesn't split the surrounding literal run"
        );
    }

    #[test]
    fn tokens_braced_var_followed_by_literal_braces() {
        // A braced var immediately followed by literal `{`/`}` characters that
        // are not part of another var token must not be mis-scanned as part
        // of the var, and must not itself start a new var scan.
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
        // No closing `}` anywhere in the rest of the string: malformed, the
        // whole rest is literal text starting with (and including) the `$`.
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

        // Starship's Pure Preset only sets `style` on `[directory]` — no
        // truncation_length/truncation_symbol overrides — yet Starship still
        // truncates to the trailing path components and anchors at the repo
        // root by default. The recommendation must reflect that regardless.
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
        // Starship's official Pure Preset builds the dirty indicator entirely from
        // granular per-flag variables, with `$stashed` living in a different bracket
        // group alongside `$ahead_behind`. GPY has no per-flag status distinction, so
        // every granular flag collapses onto the single aggregate `$status`.
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
        // The first granular token maps to `$status`; every later one (even in a
        // separate bracket group) is removed, so the collapse is global across the
        // whole format string, not scoped to one group.
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
    fn first_color_token_skips_attributes() {
        use super::first_color_token;
        assert_eq!(first_color_token("bold green"), Some("green"));
        assert_eq!(first_color_token("fg:#ff0000"), Some("fg:#ff0000"));
        assert_eq!(first_color_token("bold"), None);
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
        let model = parse(
            "[rust]\nsymbol = \" \"\nstyle = \"bold red\"\n[python]\nstyle = \"bold yellow\"\n",
        )
        .unwrap();
        let mut warnings = Warnings::new();
        let result = translate_languages(&model, LanguageTheme::default(), &mut warnings);
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
        let result = translate_languages(&model, LanguageTheme::default(), &mut warnings);
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
        // java: a plain-bold style clears the base's `dimmed`; a source symbol
        // drops the base symbol so `[language.icons]` applies. swift: untouched.
        let model = parse("[java]\nstyle = \"bold blue\"\nsymbol = \"J \"\n").expect("parse");
        let mut warnings = Warnings::new();
        let result = translate_languages(&model, base, &mut warnings);
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
        let module = table(
            "min_time = 2000\nformat = \"took [$duration]($style) \"\nstyle = \"bold yellow\"\n",
        );
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
        let module = table(
            "min_time = 2000\nformat = \"took [$duration]($style) \"\nstyle = \"bold yellow\"\n",
        );
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
        let module =
            table("success_symbol = \"[❯](bold green)\"\nerror_symbol = \"[❯](bold red)\"\n");
        let mut warnings = Warnings::new();
        let theme = translate_character(&module, &mut warnings);
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
        let theme = translate_character(&table("success_symbol = \"[➜](bold green) \"\n"), &mut w);
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
            &mut w,
        );
        assert_eq!(theme.success_symbol, "> ");
        assert_eq!(theme.format.as_deref(), Some("[$symbol]($style) "));
    }

    #[test]
    fn character_symbol_with_leading_and_trailing_space_is_parsed() {
        use super::translate_character;
        let mut w = Warnings::new();
        let theme = translate_character(&table("error_symbol = \" [✗](bold red)  \"\n"), &mut w);
        assert_eq!(theme.error_symbol, " ✗  ");
        assert_eq!(theme.error_color.as_str(), "red");
    }

    #[test]
    fn character_symbol_with_variable_warns_lossy() {
        use super::translate_character;
        use crate::theme::CharacterTheme;
        let mut w = Warnings::new();
        let theme = translate_character(&table("error_symbol = \"[$x](bold red)\"\n"), &mut w);
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
        let module = table(
            "format = \"[$ssh_symbol$hostname]($style) in \"\nstyle = \"bold dimmed green\"\n",
        );
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
            "show_always absent defaults to false (root/sudo only)"
        );
    }

    #[test]
    fn translate_username_renames_user_and_inlines_style_root() {
        use super::translate_username;
        let module = table(
            "format = \"[$user]($style) in \"\nstyle_root = \"bold red\"\nstyle_user = \"bold yellow\"\n",
        );
        let mut warnings = Warnings::new();
        let theme = translate_username(&module, &mut warnings);
        assert_eq!(theme.format.as_deref(), Some("[$username](bold red) in "));
        assert!(warnings.is_empty());
    }

    #[test]
    fn translate_username_defaults_style_root_when_absent() {
        use super::translate_username;
        let module = table("");
        let mut warnings = Warnings::new();
        let theme = translate_username(&module, &mut warnings);
        assert_eq!(theme.format.as_deref(), Some("[$username](bold red) "));
    }

    #[test]
    fn translate_username_drops_unknown_vars() {
        use super::translate_username;
        let module = table("format = \"[$user$sep_close]($style) \"\n");
        let mut warnings = Warnings::new();
        let theme = translate_username(&module, &mut warnings);
        let format = theme.format.expect("username format produced");
        assert!(format.contains("$username"));
        assert!(!format.contains("sep_close"));
        assert!(warnings.iter().any(|w| w.message.contains("sep_close")));
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
}
