//! Translators for Starship's `git_branch`, `git_status` and `git_state` modules.

use super::with_default_style;
use crate::config::defaults::BRANCH_GLYPH;
use crate::config::types::Icon;
use crate::import::starship::format::{
    Token, inline_style_vars, remove_literal, rename_vars, replace_var, retain_known_vars, tokens,
};
use crate::import::starship::warn::{WarningKind, Warnings};
use crate::theme::GitTheme;

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

/// `git_status` symbol keys that GPY draws from `[segments.git]` `*_icon`.
const MAPPED_STATUS_SYMBOLS: [&str; 4_usize] = ["staged", "modified", "untracked", "conflicted"];

/// `git_status` symbol keys with no GPY theme equivalent.
const UNMAPPED_STATUS_SYMBOLS: [&str; 16_usize] = [
    "ahead",
    "behind",
    "diverged",
    "up_to_date",
    "stashed",
    "renamed",
    "deleted",
    "typechanged",
    "worktree_added",
    "worktree_deleted",
    "worktree_modified",
    "worktree_typechanged",
    "index_added",
    "index_deleted",
    "index_modified",
    "index_typechanged",
];

/// `git_state` keys GPY cannot carry: it renders its own `$state` text.
const STATE_LABEL_KEYS: [&str; 8_usize] = [
    "format",
    "rebase",
    "merge",
    "revert",
    "cherry_pick",
    "bisect",
    "am",
    "am_or_rebase",
];

/// Starship's default `git_branch` `symbol`: the branch glyph and a space.
const DEFAULT_BRANCH_SYMBOL: &str = "\u{e0a0} ";

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

/// Resolve `$symbol` in a `git_branch` format.
///
/// GPY's `$symbol` is the bare branch glyph, while Starship's carries its own
/// trailing space, so the default symbol keeps `$symbol` and re-adds that
/// space (dropped with the glyph when icons are off). Any other symbol has no
/// GPY setting and is inlined as written.
fn apply_branch_symbol(
    format: &str,
    table: &toml::value::Table,
    warnings: &mut Warnings,
) -> String {
    let symbol = table
        .get("symbol")
        .and_then(toml::Value::as_str)
        .unwrap_or(DEFAULT_BRANCH_SYMBOL);
    let glyph = symbol.trim_end();
    if glyph == BRANCH_GLYPH {
        let gap = symbol.strip_prefix(glyph).unwrap_or_default();
        return if gap.is_empty() {
            format.to_owned()
        } else {
            replace_var(format, "symbol", &format!("($symbol{gap})"))
        };
    }
    if crate::template::parse::parse(symbol).is_err() {
        warnings.push(
            WarningKind::UnrepresentableOption,
            format!(
                "git_branch: symbol '{symbol}' is not a valid format string; using GPY's branch glyph"
            ),
        );
        return replace_var(format, "symbol", "($symbol )");
    }
    replace_var(format, "symbol", symbol)
}

/// A Starship status symbol as plain text, or `None` when it uses variables
/// (`$count`) or styling groups that a GPY icon cannot hold.
fn plain_symbol(raw: &str) -> Option<String> {
    let mut plain = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => plain.push(chars.next()?),
            '$' | '[' | ']' | '(' | ')' => return None,
            other => plain.push(other),
        }
    }
    Some(plain)
}

/// Carry the `git_status` symbol `key` over as a theme icon; warns when it cannot.
fn status_icon(table: &toml::value::Table, key: &str, warnings: &mut Warnings) -> Option<String> {
    let raw = table.get(key)?.as_str()?;
    let Some(plain) = plain_symbol(raw) else {
        warnings.push(
            WarningKind::LossyMapping,
            format!(
                "git_status: {key} '{raw}' uses variables or styling that a GPY status icon cannot carry; skipped"
            ),
        );
        return None;
    };
    let Ok(icon) = Icon::new(plain) else {
        warnings.push(
            WarningKind::UnrepresentableOption,
            format!("git_status: {key} '{raw}' is invalid; skipped"),
        );
        return None;
    };
    Some(icon.as_str().to_owned())
}

/// Warn once for the `git_status` symbols GPY's theme cannot hold.
fn warn_unmapped_status_symbols(table: &toml::value::Table, warnings: &mut Warnings) {
    let present: Vec<&str> = UNMAPPED_STATUS_SYMBOLS
        .iter()
        .copied()
        .filter(|key| table.contains_key(*key))
        .collect();
    if !present.is_empty() {
        warnings.push(
            WarningKind::LossyMapping,
            format!(
                "git_status: symbols ({}) have no GPY theme equivalent; not transferred (ahead/behind icons are set with [git.icons] in config.toml)",
                present.join(", ")
            ),
        );
    }
}

/// The `git_state` part: GPY's `$state` (e.g. `REBASING 3/5`) in Starship's style.
fn translate_state(table: &toml::value::Table, warnings: &mut Warnings) -> String {
    let ignored: Vec<&str> = STATE_LABEL_KEYS
        .iter()
        .copied()
        .filter(|key| table.contains_key(*key))
        .collect();
    if !ignored.is_empty() {
        warnings.push(
            WarningKind::LossyMapping,
            format!(
                "git_state: {} are not carried over; GPY renders its own $state text (e.g. REBASING 3/5)",
                ignored.join(", ")
            ),
        );
    }
    inline_style_vars(
        r"([\($state\)]($style) )",
        &with_default_style(table, "style", "bold yellow"),
    )
}

/// Compose a GPY git `format` from `git_branch`, `git_state`, and `git_status`.
///
/// A missing branch or status half uses Starship's defaults; all three absent
/// yields no format. The `git_status` symbols that map cleanly become the
/// theme's status icons.
#[must_use]
pub fn translate_git(
    branch: Option<&toml::value::Table>,
    status: Option<&toml::value::Table>,
    state: Option<&toml::value::Table>,
    warnings: &mut Warnings,
) -> GitTheme {
    let empty = toml::value::Table::new();
    let (branch_table, status_table) = if branch.is_none() && status.is_none() && state.is_none() {
        (None, None)
    } else {
        (
            Some(branch.unwrap_or(&empty)),
            Some(status.unwrap_or(&empty)),
        )
    };

    let branch_part = branch_table.map_or_default(|table| {
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
        let without_remote = remove_literal(&retained, "(:)");
        apply_branch_symbol(&without_remote, table, warnings)
    });

    let state_part = state.map_or_default(|table| translate_state(table, warnings));

    let status_part = status_table.map_or_default(|table| {
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

    let mut theme = GitTheme::default();
    if let Some(table) = status_table {
        let [staged, unstaged, untracked, conflicts] =
            MAPPED_STATUS_SYMBOLS.map(|key| status_icon(table, key, warnings));
        theme.staged_icon = staged;
        theme.unstaged_icon = unstaged;
        theme.untracked_icon = untracked;
        theme.conflicts_icon = conflicts;
        warn_unmapped_status_symbols(table, warnings);
    }

    let format = format!("{branch_part}{state_part}{status_part}");
    theme.format = (!format.is_empty()).then_some(format);
    theme
}
