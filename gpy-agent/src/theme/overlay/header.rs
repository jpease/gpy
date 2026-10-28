//! The overlay keys of a theme file: `extends` and `unset` (ADR-0012 section 1).

use super::{merge, overlay_error};
use crate::Result;
use crate::config::discovery::validate_config_name;
use crate::config::types::invalid_name_message;

/// One `unset` entry of an overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnsetEntry {
    /// Position in the file's `unset` array, for its span.
    pub(super) index: usize,
    /// The dotted path, components split, in the nested language spelling.
    pub(super) path: Vec<String>,
}

/// A theme file's overlay keys, taken out of its table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Header {
    /// The base theme's name; `None` for a full theme.
    pub(super) extends: Option<String>,
    /// Paths to remove from the base before this file merges over it. Always
    /// empty for a full theme, where `unset` does nothing.
    pub(super) unset: Vec<UnsetEntry>,
}

const EXTENDS_FIX: &str =
    "Set `extends = \"<theme>\"` to the name of the theme this file builds on, never a path.";
const UNSET_FIX: &str =
    "List the keys to remove as dotted paths, e.g. `unset = [\"segments.character.format\"]`.";

impl Header {
    /// Remove `extends` and `unset` from `table` and check them. A file
    /// without `extends` is a full theme; its `unset`, if any, is dropped.
    ///
    /// # Errors
    /// Returns an error naming `label` if `extends` is not a string or not a
    /// valid theme name, or (in an overlay) `unset` is not an array of dotted
    /// key paths whose components match `^[A-Za-z0-9_-]+$`.
    pub(super) fn take(table: &mut toml::Table, label: &str) -> Result<Self> {
        let base = match table.remove("extends") {
            None => None,
            Some(toml::Value::String(name)) => Some(name),
            Some(other) => {
                return Err(overlay_error(
                    label,
                    &format!(
                        "`extends` must be a theme name string, not {}",
                        other.type_str()
                    ),
                    EXTENDS_FIX,
                ));
            }
        };
        let unset_value = table.remove("unset");
        let Some(extends) = base else {
            return Ok(Self::default());
        };
        if validate_config_name("theme", &extends).is_err() {
            return Err(overlay_error(
                label,
                &format!("`extends`: {}", invalid_name_message("theme", &extends)),
                EXTENDS_FIX,
            ));
        }
        let unset = unset_value
            .map(|value| parse_unset(value, label))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            extends: Some(extends),
            unset,
        })
    }
}

/// The entries of an `unset` array.
///
/// # Errors
/// Returns an error naming `label` and the first malformed entry.
fn parse_unset(value: toml::Value, label: &str) -> Result<Vec<UnsetEntry>> {
    let kind = value.type_str();
    let toml::Value::Array(items) = value else {
        return Err(overlay_error(
            label,
            &format!("`unset` must be an array of dotted key paths, not {kind}"),
            UNSET_FIX,
        ));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let path = item.as_str().and_then(split_path).ok_or_else(|| {
                overlay_error(
                    label,
                    &format!(
                        "`unset` entry {} ({item}) is not a dotted key path of letters, digits, `_` and `-`",
                        index.saturating_add(1)
                    ),
                    UNSET_FIX,
                )
            })?;
            Ok(UnsetEntry {
                index,
                path: merge::nested_language_path(path),
            })
        })
        .collect()
}

/// `a.b-c.d_e` → `["a", "b-c", "d_e"]`; `None` if a component is empty or
/// holds anything but ASCII letters, digits, `_` and `-`.
pub(crate) fn split_path(text: &str) -> Option<Vec<String>> {
    text.split('.')
        .map(|component| {
            (!component.is_empty()
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
            .then(|| component.to_owned())
        })
        .collect()
}
