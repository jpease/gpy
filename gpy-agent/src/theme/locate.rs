//! Locates the offending key of a `[segments.<plugin>]` theme table.
//!
//! `SegmentThemes` collects plugin tables through `#[serde(flatten)]`, which buffers
//! them and drops source spans, so serde reports the first `[segments.*]` header
//! instead of the bad key. This re-checks each plugin key on its own and takes the
//! position from the `toml_edit` document, which keeps spans. Theme overlay error
//! attribution reuses the per-key check and the position helpers.

use super::PluginSegmentTheme;
use toml_edit::{Document, TableLike};

/// Tables under `[segments]` that deserialize into typed fields instead of the plugin map.
pub(super) const BUILTIN_TABLES: &[&str] = &[
    "clock",
    "character",
    "directory",
    "duration",
    "status",
    "git",
    "language",
];

/// Finds the first invalid key in a plugin segment table of `contents`, in file order.
///
/// `value` is the already parsed `contents`. Returns
/// `segments.<segment>.<key> (line L, column C): <reason>`, or `None` when every plugin
/// key is valid (the failure then lies in a typed table, whose spans serde reports itself).
pub(super) fn plugin_key_error(contents: &str, value: &toml::Value) -> Option<String> {
    let document = Document::parse(contents).ok()?;
    let segments = document.get("segments")?.as_table_like()?;
    segments
        .iter()
        .filter(|(name, _)| !BUILTIN_TABLES.contains(name))
        .filter_map(|(name, item)| Some((name, item.as_table_like()?)))
        .find_map(|(name, table)| first_bad_key(contents, value, name, table))
}

fn first_bad_key(
    contents: &str,
    value: &toml::Value,
    segment: &str,
    table: &dyn TableLike,
) -> Option<String> {
    let parsed = value.get("segments")?.get(segment)?;
    table.iter().find_map(|(key, _)| {
        let reason = plugin_key_failure(key, parsed.get(key)?)?;
        let (line, column) = line_column(contents, key_offset(table, key).unwrap_or(0));
        Some(format!(
            "segments.{segment}.{key} (line {line}, column {column}): {}",
            reason.message().trim()
        ))
    })
}

/// Why `key = value` alone is not a valid plugin segment table, if it is not.
/// Every plugin segment field defaults, so a one-key table is valid input.
pub(super) fn plugin_key_failure(key: &str, value: &toml::Value) -> Option<toml::de::Error> {
    let single = toml::Table::from_iter([(key.to_owned(), value.clone())]);
    toml::Value::Table(single)
        .try_into::<PluginSegmentTheme>()
        .err()
}

/// Byte offset of `key` in `table`: its key's span, else its value's.
pub(super) fn key_offset(table: &dyn TableLike, key: &str) -> Option<usize> {
    let (found, item) = table.get_key_value(key)?;
    found.span().or_else(|| item.span()).map(|span| span.start)
}

/// 1-based line and column of byte `offset` in `contents`.
pub(super) fn line_column(contents: &str, offset: usize) -> (usize, usize) {
    let before = contents.get(..offset).unwrap_or_default();
    let line = before.matches('\n').count().saturating_add(1);
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |tail| tail.chars().count())
        .saturating_add(1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::BUILTIN_TABLES;
    use crate::theme::SegmentThemes;

    #[test]
    fn builtin_tables_match_typed_segment_fields() {
        let serialized = toml::Value::try_from(SegmentThemes::default()).expect("serialize");
        let mut keys: Vec<&str> = serialized
            .as_table()
            .expect("table")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut expected = BUILTIN_TABLES.to_vec();
        expected.sort_unstable();
        assert_eq!(keys, expected);
    }
}
