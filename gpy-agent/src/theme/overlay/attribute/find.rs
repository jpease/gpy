//! Finding the failing key of a merged theme table (ADR-0012 section 2,
//! attribution step 1). The key path of a `toml::de::Error` is private, and a
//! merged table has no spans, so the key is found structurally: each table,
//! then each direct key of the first failing table, is deserialized alone.
//! That works because every container defaults its missing fields.

use super::super::merge::nested_language_path;
use crate::config::language_icons::{LanguageMap, LanguageTheme};
use crate::config::types::ColorSpec;
use crate::config::validation::theme::DUPLICATE_SECTION;
use crate::config::validation::{ValidationError, ValidationErrorKind};
use crate::plugin::SegmentName;
use crate::theme::locate::{BUILTIN_TABLES, plugin_key_failure};
use crate::theme::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitTheme, PluginSegmentTheme,
    StatusTheme, UiTheme,
};

/// A failing path with why it fails.
pub(in crate::theme::overlay) type Failing = (Vec<String>, String);

/// The first key of `merged` whose value fails to deserialize alone: plugin
/// entries of `[segments]` first, then the typed segment tables, then `[ui]`.
/// A table no single key of which fails alone gives the table's path. `None`
/// when no table fails alone.
pub(in crate::theme::overlay) fn deserialize_failure(merged: &toml::Table) -> Option<Failing> {
    if let Some(segments) = merged.get("segments").and_then(toml::Value::as_table) {
        let plugin = segments
            .iter()
            .filter(|(id, _)| !BUILTIN_TABLES.contains(&id.as_str()))
            .find_map(|(id, value)| plugin_failure(id, value));
        if plugin.is_some() {
            return plugin;
        }
        let typed = BUILTIN_TABLES
            .iter()
            .find_map(|&name| typed_failure(&["segments", name], name, segments.get(name)?));
        if typed.is_some() {
            return typed;
        }
    }
    typed_failure(&["ui"], "ui", merged.get("ui")?)
}

/// `[segments.<id>]` alone as a plugin segment table: a non-table gives
/// `segments.<id>`, a table its first key failing alone.
fn plugin_failure(id: &str, value: &toml::Value) -> Option<Failing> {
    let error = value.clone().try_into::<PluginSegmentTheme>().err()?;
    let path = vec!["segments".to_owned(), id.to_owned()];
    let Some(table) = value.as_table() else {
        return Some((path, reason(&error)));
    };
    let found = table.iter().find_map(|(key, item)| {
        let failure = plugin_key_failure(key, item)?;
        Some((child(&path, key), reason(&failure)))
    });
    Some(found.unwrap_or_else(|| (path, reason(&error))))
}

/// The typed table `name` (at `prefix`) alone, then each of its keys alone. A
/// failing language map (`overrides`, `symbols`, `styles`) is descended into.
fn typed_failure(prefix: &[&str], name: &str, value: &toml::Value) -> Option<Failing> {
    let error = deserialize_typed(name, value.clone())?;
    let path: Vec<String> = prefix.iter().map(|part| (*part).to_owned()).collect();
    let Some(table) = value.as_table() else {
        return Some((path, reason(&error)));
    };
    let found = table.iter().find_map(|(key, item)| {
        let single = toml::Table::from_iter([(key.clone(), item.clone())]);
        let failure = deserialize_typed(name, toml::Value::Table(single))?;
        let key_path = child(&path, key);
        if name == "language"
            && let Some(map) = language_map(key)
            && let Some(entries) = item.as_table()
            && let Some(entry) = entries.iter().find_map(|(entry_key, entry_value)| {
                map_entry_failure(map, entry_value)
                    .map(|bad| (child(&key_path, entry_key), reason(&bad)))
            })
        {
            return Some(entry);
        }
        Some((key_path, reason(&failure)))
    });
    Some(found.unwrap_or_else(|| (path, reason(&error))))
}

/// Why `value` fails as the typed table `name`, if it does.
fn deserialize_typed(name: &str, value: toml::Value) -> Option<toml::de::Error> {
    match name {
        "clock" => value.try_into::<ClockTheme>().err(),
        "character" => value.try_into::<CharacterTheme>().err(),
        "directory" => value.try_into::<DirectoryTheme>().err(),
        "duration" => value.try_into::<DurationTheme>().err(),
        "status" => value.try_into::<StatusTheme>().err(),
        "git" => value.try_into::<GitTheme>().err(),
        "language" => value.try_into::<LanguageTheme>().err(),
        "ui" => value.try_into::<UiTheme>().err(),
        _ => None,
    }
}

/// The language map a `[segments.language]` key names, if it names one.
fn language_map(key: &str) -> Option<LanguageMap> {
    [
        LanguageMap::Overrides,
        LanguageMap::Symbols,
        LanguageMap::Styles,
    ]
    .into_iter()
    .find(|map| map.key() == key)
}

/// Why `value` fails as an entry of the language map `map`.
fn map_entry_failure(map: LanguageMap, value: &toml::Value) -> Option<toml::de::Error> {
    match map {
        LanguageMap::Overrides => value.clone().try_into::<ColorSpec>().err(),
        LanguageMap::Symbols | LanguageMap::Styles => value.clone().try_into::<String>().err(),
    }
}

/// The paths of `merged` a validation failure names, mapped back to raw
/// `[segments]` keys: a normalised segment name maps to every key that
/// normalises to it, a collision to both sides, and a flat language path to
/// the nested spelling.
pub(in crate::theme::overlay) fn validation_paths(
    merged: &toml::Table,
    error: &ValidationError,
) -> Vec<Vec<String>> {
    let sections = merged.get("segments").and_then(toml::Value::as_table);
    // Sorted, so the message never depends on the table's key order.
    let named = |name: &SegmentName| -> Vec<Vec<String>> {
        let mut keys: Vec<&String> = sections
            .into_iter()
            .flat_map(toml::Table::keys)
            .filter(|key| SegmentName::new(key).ok().as_ref() == Some(name))
            .collect();
        keys.sort_unstable();
        keys.into_iter().map(|key| section_path(key)).collect()
    };
    match &error.kind {
        ValidationErrorKind::InvalidSegmentName { value } => vec![section_path(value)],
        ValidationErrorKind::InvalidValue { expected, value } if *expected == DUPLICATE_SECTION => {
            SegmentName::new(value)
                .ok()
                .map_or_else(|| vec![section_path(value)], |name| named(&name))
        }
        ValidationErrorKind::SegmentTokenCollision { collision } => {
            let mut paths = named(&collision.first);
            paths.extend(named(&collision.second));
            paths
        }
        ValidationErrorKind::PluginVariableCollision { collision } => collision
            .first
            .iter()
            .chain(std::iter::once(&collision.second))
            .map(|field| dotted(field))
            .collect(),
        _ => vec![nested_language_path(dotted(&error.field_path))],
    }
}

/// `segments.<key>`.
fn section_path(key: &str) -> Vec<String> {
    vec!["segments".to_owned(), key.to_owned()]
}

/// `a.b.c` → `["a", "b", "c"]`.
fn dotted(path: &str) -> Vec<String> {
    path.split('.').map(str::to_owned).collect()
}

/// `path` extended by `key`.
fn child(path: &[String], key: &str) -> Vec<String> {
    let mut child = path.to_vec();
    child.push(key.to_owned());
    child
}

/// The deserializer's message, without serde's trailing newline.
pub(in crate::theme::overlay) fn reason(error: &toml::de::Error) -> String {
    error.message().trim().to_owned()
}
