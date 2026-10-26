//! The Starship format-string tokenizer and the token-level rewrites built on it.

use crate::import::starship::warn::{WarningKind, Warnings};
use std::borrow::Cow;

/// One lexical unit of a Starship format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    /// Literal text, with backslash escapes kept verbatim.
    Literal(&'a str),
    /// A `$name` or `${name}` variable reference.
    Var {
        /// Bare variable name.
        name: &'a str,
        /// Original source text, for verbatim re-emission.
        raw: &'a str,
    },
}

/// Tokenize a Starship format string into literal runs and variable references.
///
/// A backslash makes the next character literal; a `$` that starts no well-formed
/// reference (including an unclosed `${`) is literal too.
#[must_use]
pub fn tokens(format: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut literal_start = 0_usize;
    // Markers are ASCII, so slicing between them respects UTF-8 boundaries.
    let mut index = 0_usize;
    let bytes_len = format.len();
    while index < bytes_len {
        match format.as_bytes().get(index) {
            Some(b'\\') => {
                // Backslash plus next char is literal.
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

/// Scan a `$name`/`${name}` token at `start`; `None` if malformed (incl. unclosed `${`).
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

/// Rename variable references whose whole name matches a `(from, to)` pair,
/// keeping each reference's `$name`/`${name}` form.
#[must_use]
pub fn rename_vars(format: &str, renames: &[(&str, &str)]) -> String {
    tokens(format)
        .into_iter()
        .map(|token| match token {
            Token::Literal(text) => Cow::Borrowed(text),
            Token::Var { name, raw } => renames.iter().find(|(from, _)| *from == name).map_or(
                Cow::Borrowed(raw),
                |(_, to)| {
                    Cow::Owned(if raw.starts_with("${") {
                        format!("${{{to}}}")
                    } else {
                        format!("${to}")
                    })
                },
            ),
        })
        .collect()
}

/// Replace `$style`/`$*_style` references with the module's literal style values.
///
/// References without a string value in `table` are kept.
#[must_use]
pub fn inline_style_vars(format: &str, table: &toml::value::Table) -> String {
    tokens(format)
        .into_iter()
        .map(|token| match token {
            Token::Literal(text) => text,
            Token::Var { name, raw } => {
                if name.ends_with("style") {
                    table.get(name).and_then(toml::Value::as_str).unwrap_or(raw)
                } else {
                    raw
                }
            }
        })
        .collect()
}

/// Drop variable references not present in `known`, warning once per dropped reference.
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
            Token::Literal(text) => text,
            Token::Var { name, raw } => {
                if known.contains(&name) {
                    raw
                } else {
                    warnings.push(
                        WarningKind::LossyMapping,
                        format!("{segment}: variable '${name}' has no GPY equivalent; dropped"),
                    );
                    ""
                }
            }
        })
        .collect()
}

/// Remove every unescaped occurrence of `needle` from the literal text of `format`.
#[must_use]
pub fn remove_literal(format: &str, needle: &str) -> String {
    let mut out = String::with_capacity(format.len());
    for token in tokens(format) {
        match token {
            Token::Var { raw, .. } => out.push_str(raw),
            Token::Literal(text) => {
                let mut rest = text;
                while !rest.is_empty() {
                    if let Some(after) = rest.strip_prefix(needle) {
                        rest = after;
                        continue;
                    }
                    // Step one character, or a whole escape so its target never matches.
                    let mut chars = rest.chars();
                    if chars.next() == Some('\\') {
                        chars.next();
                    }
                    let remaining = chars.as_str();
                    let step = rest.len().saturating_sub(remaining.len());
                    out.push_str(rest.get(..step).unwrap_or(rest));
                    rest = remaining;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "tests: a fixture that fails to parse is a test failure"
    )]

    use super::{Token, inline_style_vars, remove_literal, rename_vars, retain_known_vars, tokens};
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
        assert_eq!(tokens(r"\$"), vec![Token::Literal(r"\$")]);
        assert_eq!(
            tokens(r"a\$b"),
            vec![Token::Literal(r"a\$b")],
            "escape doesn't split the surrounding literal run"
        );
    }

    #[test]
    fn tokens_braced_var_followed_by_literal_braces() {
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
        assert_eq!(tokens("${unclosed"), vec![Token::Literal("${unclosed")]);
        assert_eq!(
            tokens("${rust$directory"),
            vec![
                Token::Literal("${rust"),
                Token::Var {
                    name: "directory",
                    raw: "$directory"
                },
            ],
            "a later reference still tokenizes after an unclosed `${{`"
        );
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
    fn rename_vars_matches_whole_names_and_keeps_form() {
        assert_eq!(
            rename_vars(
                r"$user ${user} $username $userfoo \$user",
                &[("user", "username")]
            ),
            r"$username ${username} $username $userfoo \$user"
        );
    }

    #[test]
    fn rename_vars_applies_every_pair() {
        assert_eq!(rename_vars("$a$b$c", &[("a", "x"), ("c", "z")]), "$x$b$z");
    }

    #[test]
    fn inlines_style_and_read_only_style() {
        let module = table("style = \"bold cyan\"\nread_only_style = \"bold red\"\n");
        let out = inline_style_vars("[$path]($style)[$read_only]($read_only_style) ", &module);
        assert_eq!(out, "[$path](bold cyan)[$read_only](bold red) ");
    }

    #[test]
    fn inline_style_vars_skips_escapes_and_longer_names() {
        let module = table("style = \"bold\"\n");
        assert_eq!(
            inline_style_vars(r"\$style ${style} $style_x $style", &module),
            r"\$style bold $style_x bold"
        );
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
        assert_eq!(out, "[$path]() $read_only");
        assert_eq!(warnings.len(), 1_usize);
    }

    #[test]
    fn remove_literal_skips_escaped_and_variable_text() {
        assert_eq!(remove_literal("on $branch(:) ", "(:)"), "on $branch ");
        assert_eq!(remove_literal(r"\(:) \\(:)", "(:)"), r"\(:) \\");
        assert_eq!(remove_literal("${a(:)b}", "(:)"), "${a(:)b}");
    }
}
