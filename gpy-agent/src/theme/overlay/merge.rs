//! Table merge rules of theme overlays (ADR-0012): tables merge
//! deeply at every depth, anything else (arrays included) replaces, `unset`
//! removes a leaf or a table, and `[segments.language]` has one spelling.

use crate::config::language_icons::{LANGUAGE_FIELDS, flat_language_map, nest_language_table};

/// Rewrite the `[segments.language]` table of a theme file's `table` to the
/// nested spelling, so a flat key and its nested twin from another file of
/// the chain land on the same path.
pub(super) fn canonicalise(table: &mut toml::Table) {
    if let Some(toml::Value::Table(segments)) = table.get_mut("segments")
        && let Some(toml::Value::Table(language)) = segments.get_mut("language")
    {
        nest_language_table(language);
    }
}

/// `path` in the nested language spelling: `segments.language.<flat key>…`
/// becomes `segments.language.<map>.<flat key>…`. Other paths are unchanged.
pub(crate) fn nested_language_path(mut path: Vec<String>) -> Vec<String> {
    let map = match path.as_slice() {
        [segments, language, key, ..]
            if segments == "segments"
                && language == "language"
                && !LANGUAGE_FIELDS.contains(&key.as_str()) =>
        {
            flat_language_map(key).key()
        }
        _ => return path,
    };
    path.insert(2, map.to_owned());
    path
}

/// The flat spelling of a nested language `path`, if it is one:
/// `segments.language.<map>.<key>…` → `segments.language.<key>…`, when `<key>`
/// is a flat key that belongs to `<map>`.
pub(super) fn flat_language_path(path: &[String]) -> Option<Vec<String>> {
    let [segments, language, map, key, rest @ ..] = path else {
        return None;
    };
    let flat = segments == "segments"
        && language == "language"
        && !LANGUAGE_FIELDS.contains(&key.as_str())
        && flat_language_map(key).key() == map.as_str();
    flat.then(|| {
        [segments, language, key]
            .into_iter()
            .chain(rest)
            .cloned()
            .collect()
    })
}

/// Remove the value at `path` from `table`. A path the table lacks, or one
/// that crosses a non-table, removes nothing.
pub(super) fn remove_path(table: &mut toml::Table, path: &[String]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = table;
    for key in parents {
        let Some(toml::Value::Table(next)) = current.get_mut(key) else {
            return;
        };
        current = next;
    }
    current.remove(last);
}

/// Merge `overlay` over `base`: where both hold a table the merge recurses,
/// anywhere else the overlay's value replaces the base's.
pub(super) fn deep_merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        let meets_table = matches!(base.get(&key), Some(toml::Value::Table(_)));
        match value {
            toml::Value::Table(inner) if meets_table => {
                if let Some(toml::Value::Table(existing)) = base.get_mut(&key) {
                    deep_merge(existing, inner);
                }
            }
            other => {
                base.insert(key, other);
            }
        }
    }
}
