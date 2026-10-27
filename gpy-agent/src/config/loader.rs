//! Configuration file loading, parsing, and comment-preserving saving.

use super::{Config, schema, validation};
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Load the first available config file, fresh from disk.
///
/// # Errors
/// Returns an error if the config file cannot be read or parsed.
pub fn load_config() -> Result<Config> {
    match schema::resolve_active_config(&schema::get_config_paths()) {
        Some((_, Some(existing))) => load_config_from_file(&existing),
        _ => Ok(Config::default()),
    }
}

/// Load configuration from a specific TOML file.
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub fn load_config_from_file(path: &Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        Error::config(format!(
            "Failed to read config file {}: {e}",
            path.display()
        ))
    })?;

    parse_config_toml(&contents)
}

/// Parse TOML configuration content, normalizing user-spelled names.
///
/// # Errors
/// Returns an error if the TOML is malformed, doesn't deserialize, or is invalid.
pub fn parse_config_toml(toml_content: &str) -> Result<Config> {
    let mut config: Config = toml::from_str(toml_content)
        .map_err(|e| Error::config(format!("Failed to parse TOML: {e}")))?;

    validation::normalize_config(&mut config)?;

    Ok(config)
}

/// Write only changed leaves (plus `explicit_keys`) into the file in place,
/// preserving comments and layout; symlinks are written through. `updated` is
/// normalized first, so the file holds what the next load would produce.
///
/// # Errors
/// Returns an error if `updated` is invalid or the file can't be read, parsed, or written.
pub fn save_config(
    path: &Path,
    original: &Config,
    updated: &Config,
    explicit_keys: &[&str],
) -> Result<()> {
    let mut normalized = updated.clone();
    validation::normalize_config(&mut normalized)?;

    let original_leaves = config_leaves(original)?;
    let updated_leaves = config_leaves(&normalized)?;

    let target = resolve_write_target(path)?;
    // Held through the rename: a second writer must read this one's result.
    let _lock = lock_for_save(&target)?;
    let existing = read_existing(&target)?;
    let mut document: toml_edit::DocumentMut = match existing.as_deref() {
        Some(text) => text.parse().map_err(|e| {
            Error::config(format!(
                "Failed to parse existing config {}: {e}",
                path.display()
            ))
        })?,
        None => toml_edit::DocumentMut::new(),
    };

    let mut changes: BTreeMap<Vec<String>, Option<&toml::Value>> = BTreeMap::new();
    for (leaf, value) in &updated_leaves {
        if original_leaves.get(leaf) != Some(value) {
            changes.insert(leaf.clone(), Some(value));
        }
    }
    for leaf in original_leaves.keys() {
        if !updated_leaves.contains_key(leaf) {
            changes.insert(leaf.clone(), None);
        }
    }
    for key in explicit_keys {
        let leaf: Vec<String> = key.split('.').map(str::to_owned).collect();
        if let Some(value) = updated_leaves.get(&leaf) {
            changes.insert(leaf, Some(value));
        }
    }

    for (leaf, change) in &changes {
        match change {
            Some(value) => set_leaf(&mut document, leaf, value)?,
            None => remove_leaf(&mut document, leaf),
        }
    }

    let rendered = document.to_string();
    if existing.as_deref() == Some(rendered.as_str()) || (existing.is_none() && changes.is_empty())
    {
        return Ok(());
    }
    write_atomically(&target, &rendered)
}

/// Flatten to sorted `dotted-path -> leaf`; arrays are single leaves.
///
/// # Errors
/// Returns an error if the config cannot be converted to a TOML table.
fn config_leaves(config: &Config) -> Result<BTreeMap<Vec<String>, toml::Value>> {
    let table = toml::Table::try_from(config)
        .map_err(|e| Error::config(format!("Failed to serialize config to TOML: {e}")))?;
    let mut leaves = BTreeMap::new();
    flatten_table(&table, &mut Vec::new(), &mut leaves);
    Ok(leaves)
}

fn flatten_table(
    table: &toml::Table,
    prefix: &mut Vec<String>,
    out: &mut BTreeMap<Vec<String>, toml::Value>,
) {
    for (key, value) in table {
        prefix.push(key.clone());
        match value {
            toml::Value::Table(inner) => flatten_table(inner, prefix, out),
            leaf => {
                out.insert(prefix.clone(), leaf.clone());
            }
        }
        prefix.pop();
    }
}

/// Dotted keys set in `raw` that `Config` does not read (serde ignores them,
/// so typos go unnoticed), sorted. Keys under a dynamic-map prefix such as
/// `language.icons.` are never unknown.
#[must_use]
pub fn unknown_config_keys(raw: &toml::Table) -> Vec<String> {
    let Ok(known) = config_leaves(&Config::default()) else {
        return Vec::new();
    };
    let mut set = BTreeMap::new();
    flatten_table(raw, &mut Vec::new(), &mut set);
    let mut unknown: Vec<String> = set
        .into_keys()
        .filter(|path| !known.contains_key(path))
        .map(|path| path.join("."))
        .filter(|key| {
            !super::metadata::DYNAMIC_KEY_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .collect();
    unknown.sort();
    unknown
}

/// # Errors
/// Returns an error if a datetime value cannot be converted.
fn to_edit_value(value: &toml::Value) -> Result<toml_edit::Value> {
    Ok(match value {
        toml::Value::String(s) => toml_edit::Value::from(s.as_str()),
        toml::Value::Integer(i) => toml_edit::Value::from(*i),
        toml::Value::Float(f) => toml_edit::Value::from(*f),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(dt) => {
            let parsed: toml_edit::Datetime = dt.to_string().parse().map_err(|e| {
                Error::config(format!("Failed to convert datetime config value: {e}"))
            })?;
            toml_edit::Value::from(parsed)
        }
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(to_edit_value(item)?);
            }
            toml_edit::Value::Array(array)
        }
        toml::Value::Table(table) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, item) in table {
                inline.insert(key, to_edit_value(item)?);
            }
            toml_edit::Value::InlineTable(inline)
        }
    })
}

/// Set the value at `leaf`, creating missing parent tables.
///
/// # Errors
/// Returns an error if a parent key is not a table or the value can't convert.
fn set_leaf(
    document: &mut toml_edit::DocumentMut,
    leaf: &[String],
    value: &toml::Value,
) -> Result<()> {
    let Some((key, parents)) = leaf.split_last() else {
        return Ok(());
    };
    let mut new_value = to_edit_value(value)?;
    let mut current: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for segment in parents {
        if !current.contains_key(segment) {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            current.insert(segment, toml_edit::Item::Table(table));
        }
        current = current
            .get_mut(segment)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| {
                Error::config(format!(
                    "Cannot set {}: `{segment}` exists in the config file but is not a table",
                    leaf.join(".")
                ))
            })?;
    }
    // Keep the existing value's surrounding whitespace and trailing comment.
    if let Some(old) = current.get_mut(key) {
        if let Some(old_value) = old.as_value() {
            *new_value.decor_mut() = old_value.decor().clone();
        }
        *old = toml_edit::Item::Value(new_value);
    } else {
        current.insert(key, toml_edit::Item::Value(new_value));
    }
    Ok(())
}

fn remove_leaf(document: &mut toml_edit::DocumentMut, leaf: &[String]) {
    let Some((key, parents)) = leaf.split_last() else {
        return;
    };
    let mut current: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for segment in parents {
        let Some(next) = current
            .get_mut(segment)
            .and_then(toml_edit::Item::as_table_like_mut)
        else {
            return;
        };
        current = next;
    }
    current.remove(key);
}

/// Resolve a symlinked config path so the link survives a write.
///
/// # Errors
/// Returns an error if the symlink cannot be resolved.
fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path).map_err(|e| {
            Error::config(format!(
                "Failed to resolve config symlink {}: {e}",
                path.display()
            ))
        }),
        _ => Ok(path.to_path_buf()),
    }
}

/// # Errors
/// Returns an error if the file exists but cannot be read.
fn read_existing(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::config(format!(
            "Failed to read config file {}: {e}",
            path.display()
        ))),
    }
}

/// Longest a save waits for another writer; one save takes about 15 ms.
const SAVE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Lock `target` against concurrent saves, creating its directory first.
///
/// # Errors
/// Returns an error if the directory or lock file cannot be created, or the
/// lock stays held past [`SAVE_LOCK_WAIT`].
fn lock_for_save(target: &Path) -> Result<crate::fs_util::PathLock> {
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("Failed to create config directory: {e}")))?;
    }
    crate::fs_util::lock_beside(target, SAVE_LOCK_WAIT).map_err(|e| {
        Error::config(format!(
            "Failed to lock config file {}: {e}",
            target.display()
        ))
    })
}

/// Temp-file-and-rename write, keeping existing permissions.
///
/// # Errors
/// Returns an error if the directory, temp file, or rename fails.
fn write_atomically(target: &Path, contents: &str) -> Result<()> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| Error::config(format!("Failed to create config directory: {e}")))?;

    crate::fs_util::write_atomically(target, contents.as_bytes())
        .map_err(|e| Error::config(format!("Failed to write config file: {e}")))
}

/// Load a theme by name (user, plugin, or builtin), fresh from disk.
///
/// # Errors
/// Returns an error if the theme is unresolvable or fails to parse.
pub fn load_theme(theme_name: &str) -> Result<crate::theme::ThemeConfig> {
    // Same resolver as the daemon so oneshot agrees on plugin themes.
    let theme_path = crate::theme::ThemeManager::resolve_path(theme_name).ok_or_else(|| {
        Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        ))
    })?;

    load_theme_from_path(&theme_path)
}

/// Load a theme from a path, falling back to the builtin with that stem.
///
/// This is the single funnel for theme load errors (CLI, daemon, oneshot,
/// export, `doctor`). When the failing file is the user copy of an embedded
/// builtin, the returned error already ends with
/// [`user_copy::recovery_hint`](crate::theme::user_copy::recovery_hint) (it
/// says the file overrides the builtin). Callers MUST NOT append the hint again.
///
/// # Errors
/// Returns an error if the file can't be read or parsed, or matches no builtin.
pub fn load_theme_from_path(path: &Path) -> Result<crate::theme::ThemeConfig> {
    load_theme_from_path_inner(path)
        .map_err(|error| crate::theme::user_copy::annotate_load_error(path, error))
}

fn load_theme_from_path_inner(path: &Path) -> Result<crate::theme::ThemeConfig> {
    use crate::theme::manager::BUILTIN_THEMES;

    // `file_stem`, not a `/` suffix match: Windows paths use `\`.
    let stem = path.file_stem().and_then(|s| s.to_str());
    let contents = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| {
            Error::config(format!("Failed to read theme file {}: {e}", path.display()))
        })?
    } else if let Some((_, content)) = BUILTIN_THEMES.iter().find(|(name, _)| stem == Some(*name)) {
        (*content).to_owned()
    } else {
        let theme_name = stem.map_or_else(|| path.display().to_string(), str::to_owned);
        return Err(Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        )));
    };

    crate::theme::parse(&contents, &path.display().to_string())
}

#[cfg(test)]
mod unknown_config_keys_tests {
    use super::unknown_config_keys;

    #[test]
    fn unknown_config_keys_reports_typos_and_skips_dynamic_prefixes() {
        let raw: toml::Table = "[git]\nenabeld = true\n[git.icons]\nahead = \"A\"\n[language.icons]\nzig = \"Z\"\n[ui]\ntheme = \"text\"\n"
            .parse()
            .expect("valid TOML");

        assert_eq!(unknown_config_keys(&raw), vec!["git.enabeld".to_owned()]);
    }
}

#[cfg(test)]
mod load_theme_from_path_tests {
    use super::load_theme_from_path;
    use std::path::Path;

    #[test]
    fn nonexistent_user_theme_path_falls_back_to_embedded_builtin() {
        let sep = std::path::MAIN_SEPARATOR;
        let fake_user_theme_path = format!("nonexistent-dir{sep}gpy{sep}themes{sep}default.toml");

        let theme = load_theme_from_path(Path::new(&fake_user_theme_path))
            .expect("embedded fallback must succeed");

        // The bare default stub's `prompt_close` is empty; the builtin's is U+E0B4.
        assert_eq!(
            theme.ui.get_prompt_close_delimiter(),
            "\u{e0b4}",
            "expected the embedded `default` theme's real closing-cap icon, got the bare \
             ThemeConfig::default() stub's empty one instead -- builtin-theme matching didn't fire"
        );
    }

    #[test]
    fn path_matching_no_builtin_theme_name_returns_an_error() {
        let err = load_theme_from_path(Path::new("nonexistent-dir/not-a-real-theme.toml"))
            .expect_err("a path matching no builtin theme name must error, not silently succeed");

        let message = err.to_string();
        assert!(
            message.contains("not-a-real-theme"),
            "error message must name the theme (its file stem), got: {message}"
        );
    }
}
