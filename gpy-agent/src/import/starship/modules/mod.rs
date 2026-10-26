//! Pure per-module translators: a Starship module table → a GPY segment theme.

mod style;

use crate::config::LanguageTheme;
use crate::config::types::{ColorSpec, DirectoryTruncationLength, DirectoryTruncationSymbol, Icon};
use crate::import::starship::format::{
    Token, inline_style_vars, remove_literal, rename_vars, retain_known_vars, tokens,
};
use crate::import::starship::model::StarshipConfig;
use crate::import::starship::warn::{WarningKind, Warnings};
use crate::template::ast::Node;
use crate::theme::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitTheme, HostnameTheme,
    RecommendedDirectory, UsernameTheme,
};
use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use style::{StyleColor, attr_tokens, style_color};

/// Granular `git_status` flags, approximated as GPY's aggregate `$status`.
const GRANULAR_STATUS_FLAGS: [&str; 7_usize] = [
    "conflicted",
    "untracked",
    "modified",
    "staged",
    "renamed",
    "deleted",
    "stashed",
];

/// Collapse granular `git_status` flag variables onto a single `$status`.
///
/// Only the first flag becomes `$status`; later ones are removed, so
/// back-to-back flags don't render `$status` repeatedly. Warns once.
fn collapse_granular_status_flags(format: &str, warnings: &mut Warnings) -> String {
    let mut collapsed_first = false;
    let mut found: Vec<&'static str> = Vec::new();
    let out: String = tokens(format)
        .into_iter()
        .filter_map(|token| match token {
            Token::Literal(text) => Some(text),
            Token::Var { name, raw } => GRANULAR_STATUS_FLAGS
                .iter()
                .copied()
                .find(|&candidate| candidate == name)
                .map_or(Some(raw), |flag| {
                    // First granular token → `$status`; later ones dropped.
                    if !found.contains(&flag) {
                        found.push(flag);
                    }
                    if collapsed_first {
                        None
                    } else {
                        collapsed_first = true;
                        Some("$status")
                    }
                }),
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

/// Translate `directory` truncation settings into the advisory
/// `[ui.recommended.directory]` block; invalid values warn instead of clamping.
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

/// Compose a GPY git `format` from `git_branch`, `git_status`, and `git_state`.
///
/// A missing half uses Starship's defaults; both absent yields `None`.
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
        let retained = retain_known_vars(
            &inlined,
            &["symbol", "branch", "status", "ahead_behind", "style"],
            "git",
            warnings,
        );
        // Dropping `$remote_branch` leaves its `(:$remote_branch)` group as an empty `(:)`.
        remove_literal(&retained, "(:)")
    });

    let status_part = status_table.map_or_else(String::new, |table| {
        let filled = with_default_style(table, "style", "bold red");
        let raw = table
            .get("format")
            .and_then(toml::Value::as_str)
            .unwrap_or(r"([\[$all_status$ahead_behind\]]($style) )");
        let inlined = inline_style_vars(raw, &filled);
        let remapped = rename_vars(&inlined, &[("all_status", "status")]);
        // Before retain_known_vars, so per-flag presets keep a status marker.
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
        (false, true) => Some(branch_part),
        (true, false) => Some(status_part),
        (false, false) => Some(format!("{branch_part}{status_part}")),
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

/// Translated language theme plus colors to fold into the palette.
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

/// Translate present Starship language modules onto `base`.
///
/// Background and per-module `format` are lossy (one shared language format).
#[must_use]
pub fn translate_languages<S: BuildHasher>(
    model: &StarshipConfig,
    base: LanguageTheme,
    palette: Option<&HashMap<String, String, S>>,
    warnings: &mut Warnings,
) -> LanguageTranslation {
    let mut theme = base;
    let mut palette_colors: BTreeMap<String, String> = BTreeMap::new();
    for (starship_name, canonical) in LANGUAGE_MODULES {
        let Some(table) = model.module_table(starship_name) else {
            continue;
        };
        if let Some(style) = table.get("style").and_then(toml::Value::as_str) {
            match style_color(style, palette) {
                StyleColor::Resolved { fg, bg } => {
                    if let Some(color) = fg
                        && let Ok(spec) = ColorSpec::new(&color)
                    {
                        theme
                            .overrides
                            .insert(format!("{canonical}_bg_color"), spec);
                        palette_colors.insert((*canonical).to_owned(), color);
                    }
                    if let Some(background) = bg {
                        warnings.push(
                            WarningKind::LossyMapping,
                            format!(
                                "language {starship_name}: background style '{background}' cannot be applied by GPY's shared language format; skipped"
                            ),
                        );
                    }
                }
                StyleColor::Invalid(token) => warnings.push(
                    WarningKind::InvalidColor,
                    format!("language {starship_name}: style color '{token}' is invalid; skipped"),
                ),
            }
            // Renderer defaults to bold; an empty string suppresses it.
            let attrs = attr_tokens(style);
            let style_key = format!("{canonical}_style");
            if attrs == ["bold"] {
                theme.styles.remove(&style_key);
            } else {
                theme.styles.insert(style_key, attrs.join(" "));
            }
        }
        if table.contains_key("format") {
            warnings.push(
                WarningKind::LossyMapping,
                format!("language {starship_name}: per-language format is not carried over (GPY uses one shared language format)"),
            );
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

/// A parsed character symbol: text, style spec, and whether it is bold.
type StyledSymbol = (String, Option<String>, bool);

/// Parse a styled symbol like `[❯](bold green)`; `None` for unsupported shapes.
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
    let bold = attr_tokens(&spec).contains(&"bold");
    Some((symbol, Some(spec), bold))
}

/// One side (`success` or `error`) of a translated `character` module.
struct CharacterSide {
    /// The symbol text, when the source gave a representable one.
    symbol: Option<String>,
    /// The foreground color, when the source's style names a valid one.
    color: Option<ColorSpec>,
    /// The background color from the source's style, if any.
    background: Option<String>,
    /// Whether bold; an absent symbol keeps Starship's bold default.
    bold: bool,
}

/// Translate the `<side>_symbol` key of a `character` table.
fn translate_character_side<S: BuildHasher>(
    table: &toml::value::Table,
    side: &str,
    palette: Option<&HashMap<String, String, S>>,
    warnings: &mut Warnings,
) -> CharacterSide {
    let mut translated = CharacterSide {
        symbol: None,
        color: None,
        background: None,
        bold: true,
    };
    let key = format!("{side}_symbol");
    let Some(raw) = table.get(&key).and_then(toml::Value::as_str) else {
        return translated;
    };
    let Some((symbol, spec, bold)) = parse_styled_symbol(raw) else {
        warnings.push(
            WarningKind::LossyMapping,
            format!("character: {key} '{raw}' cannot be represented; kept default"),
        );
        return translated;
    };
    translated.symbol = Some(symbol);
    translated.bold = bold;
    if let Some(spec_text) = spec {
        match style_color(&spec_text, palette) {
            StyleColor::Resolved { fg, bg } => {
                translated.color = fg.and_then(|color| ColorSpec::new(&color).ok());
                translated.background = bg;
            }
            StyleColor::Invalid(token) => warnings.push(
                WarningKind::InvalidColor,
                format!("character {side} color '{token}' is invalid; kept default"),
            ),
        }
    }
    translated
}

/// Translate Starship's `character` module into a GPY character theme.
///
/// Bold and background are shared across sides; disagreement warns as lossy.
#[must_use]
pub fn translate_character<S: BuildHasher>(
    table: &toml::value::Table,
    palette: Option<&HashMap<String, String, S>>,
    warnings: &mut Warnings,
) -> CharacterTheme {
    let mut theme = CharacterTheme::default();
    let success = translate_character_side(table, "success", palette, warnings);
    if let Some(symbol) = success.symbol {
        theme.success_symbol = symbol;
    }
    if let Some(color) = success.color {
        theme.success_color = color;
    }
    let error = translate_character_side(table, "error", palette, warnings);
    if let Some(symbol) = error.symbol {
        theme.error_symbol = symbol;
    }
    if let Some(color) = error.color {
        theme.error_color = color;
    }
    if success.bold != error.bold {
        warnings.push(
            WarningKind::LossyMapping,
            "character: success/error styles disagree on bold; GPY renders both through one shared format and kept bold".to_owned(),
        );
    }
    let background = if success.background == error.background {
        success.background
    } else {
        warnings.push(
            WarningKind::LossyMapping,
            "character: success/error styles disagree on the background; GPY renders both through one shared format and dropped it".to_owned(),
        );
        None
    };
    let base = if success.bold || error.bold {
        "bold $style"
    } else {
        "$style"
    };
    theme.format = Some(background.map_or_else(
        || format!("[$symbol]({base}) "),
        |color| format!("[$symbol]({base} bg:{color}) "),
    ));
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

/// Starship's default `ssh_symbol` for the `hostname` module.
const STARSHIP_SSH_SYMBOL: &str = "🌐 ";

/// Translate Starship's `hostname` module into a GPY hostname theme.
///
/// `$ssh_symbol` → `$symbol`; `ssh_only` inverts onto `show_always`.
#[must_use]
pub fn translate_hostname(table: &toml::value::Table, warnings: &mut Warnings) -> HostnameTheme {
    let mut theme = HostnameTheme {
        show_always: !table
            .get("ssh_only")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        ..Default::default()
    };
    let raw_symbol = table
        .get("ssh_symbol")
        .and_then(toml::Value::as_str)
        .unwrap_or(STARSHIP_SSH_SYMBOL);
    match Icon::new(raw_symbol) {
        Ok(icon) => theme.icon = Some(icon),
        Err(_) => warnings.push(
            WarningKind::UnrepresentableOption,
            format!("hostname: ssh_symbol '{raw_symbol}' is invalid; skipped"),
        ),
    }
    if let Some(trim_at) = table.get("trim_at").and_then(toml::Value::as_str) {
        trim_at.clone_into(&mut theme.trim_at);
    }
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("[$ssh_symbol$hostname]($style) in ");
    let renamed = rename_vars(raw, &[("ssh_symbol", "symbol")]);
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
/// `$user` → `$username`; `$style` is inlined from `style_root` into `format` and from
/// `style_user` into `user_format`, the shell telling the agent which session it is.
#[must_use]
pub fn translate_username(table: &toml::value::Table, warnings: &mut Warnings) -> UsernameTheme {
    let raw = table
        .get("format")
        .and_then(toml::Value::as_str)
        .unwrap_or("[$user]($style) in ");
    let renamed = rename_vars(raw, &[("user", "username")]);
    // `$style` is `style_root` in `format` and `style_user` in `user_format`.
    let styled_with = |key: &str, default: &str, sink: &mut Warnings| {
        let mut styles = table.clone();
        let style = table.get(key).cloned().unwrap_or_else(|| default.into());
        styles.insert("style".to_owned(), style);
        let inlined = inline_style_vars(&renamed, &styles);
        retain_known_vars(&inlined, &["username", "symbol"], "username", sink)
    };
    let format = styled_with("style_root", "bold red", warnings);
    // Same template, so the same dropped-variable warnings: report them once.
    let user_format = styled_with("style_user", "bold yellow", &mut Warnings::new());
    UsernameTheme {
        show_always: table.get("show_always").and_then(toml::Value::as_bool) == Some(true),
        format: Some(format),
        user_format: Some(user_format),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
