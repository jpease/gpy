//! Configuration file loading and parsing
//!
//! Handles TOML parsing, file system access, and merging of configuration
//! sources with proper error handling.

use super::{Config, schema, validation};
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Config loading with XDG path discovery.
///
/// Load configuration from the first available config file. Always reads
/// fresh from disk (see #608 -- this used to cache the result in a process-
/// global static; that cache is gone, since every long-lived caller of
/// configuration state goes through `ConfigManager`'s own `Arc<RwLock<Arc<_>>>`
/// instead, and every remaining caller of this function is a short-lived,
/// one-shot CLI process for which a cache bought nothing but staleness).
///
/// # Errors
///
/// Returns an error if no valid configuration file is found or if the
/// configuration file cannot be read or parsed.
pub fn load_config() -> Result<Config> {
    schema::get_config_paths()
        .iter()
        .find(|path| Path::new(path).exists())
        .map_or_else(|| Ok(Config::default()), |path| load_config_from_file(path))
}

/// Load configuration from a specific TOML file
///
/// # Errors
///
/// Returns an error if the file cannot be read or if the TOML content
/// cannot be parsed into a valid configuration.
pub fn load_config_from_file(path: &str) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| Error::config(format!("Failed to read config file {path}: {e}")))?;

    parse_config_toml(&contents)
}

/// Parse TOML configuration content
///
/// # Errors
///
/// Returns an error if the TOML content is malformed or cannot be
/// deserialized into a valid configuration structure.
pub fn parse_config_toml(toml_content: &str) -> Result<Config> {
    // Parse TOML content with the toml crate
    let parsed_config: Config = toml::from_str(toml_content)
        .map_err(|e| Error::config(format!("Failed to parse TOML: {e}")))?;

    // Fill in any missing fields with defaults (functional transformation)
    let finalized_config = apply_defaults(parsed_config);

    // Validate the configuration
    validation::validate_config(&finalized_config)?;

    Ok(finalized_config)
}

/// Apply default values to missing configuration fields (functional transformation)
///
/// Currently a no-op pass-through: every field on [`Config`] is either a
/// plain type with a `#[serde(default = "...")]` fn, or a bounded newtype
/// (`AgentTimeout`, `GitTimeout`, `SupervisorCheckInterval`, etc.) that
/// supplies its own `Default` and validates itself at deserialize time via
/// `#[serde(try_from = "...")]`. Serde already fills in a missing key's
/// default before this function ever runs, so there is nothing left here to
/// backfill.
///
/// This used to also coerce an explicit `0` in
/// `agent.supervisor.check_interval_seconds`/`max_restart_attempts` to the
/// real default (`use_if_zero`), to work around those two fields being raw
/// `u64`/`u32` with no validated range. That silently accepted `0` while
/// `validate_config` hard-rejected `1..=4` for the same field — two invalid
/// values, handled inconsistently. #597 replaced the raw ints with
/// `types::SupervisorCheckInterval`/`types::SupervisorMaxRestartAttempts`, so
/// an explicit `0` (or any other out-of-range value) is now rejected at
/// deserialize time like every other bounded field, instead of silently
/// becoming the default. This is a disclosed behavior change: a saved config
/// with `check_interval_seconds = 0` (or `max_restart_attempts = 0`) that
/// used to load silently now fails config load with a clear error.
///
/// Kept as an explicit pipeline stage (rather than removed and inlined into
/// its one call site) so a future cross-field default that genuinely can't be
/// expressed as a per-field serde default has an obvious place to live.
const fn apply_defaults(config: Config) -> Config {
    config
}

/// Save the changes between `original` and `updated` into the config file at
/// `path`, editing the document on disk instead of regenerating it.
///
/// Only leaf values that differ between the two configs, plus every dotted
/// path in `explicit_keys`, are written; comments, blank lines, key order,
/// unknown keys, and keys the user never wrote are left as they were. A
/// missing file is created from an empty document. A symlinked `path` is
/// resolved first, so the link target receives the change and the link stays
/// a link. Nothing is written when the resulting text equals the existing text.
///
/// # Errors
///
/// Returns an error if `updated` is invalid, the existing file cannot be read
/// or parsed as TOML, or the file cannot be written.
pub fn save_config(
    path: &str,
    original: &Config,
    updated: &Config,
    explicit_keys: &[&str],
) -> Result<()> {
    write_config(path, original, updated, explicit_keys, false)
}

/// Like [`save_config`], but discards whatever the file at `path` currently
/// holds and starts from an empty document (`gpy-agent init --force`).
///
/// # Errors
///
/// Returns an error if `updated` is invalid or the file cannot be written.
pub fn overwrite_config(
    path: &str,
    original: &Config,
    updated: &Config,
    explicit_keys: &[&str],
) -> Result<()> {
    write_config(path, original, updated, explicit_keys, true)
}

/// Create the config file at `path` with literal `contents` (used by
/// `config open` to bootstrap a comment-only file).
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_new_config(path: &str, contents: &str) -> Result<()> {
    write_atomically(Path::new(path), contents)
}

/// Shared body of [`save_config`] and [`overwrite_config`].
///
/// # Errors
///
/// Returns an error if `updated` is invalid, the existing file cannot be read
/// or parsed, or the file cannot be written.
fn write_config(
    path: &str,
    original: &Config,
    updated: &Config,
    explicit_keys: &[&str],
    start_empty: bool,
) -> Result<()> {
    validation::validate_config(updated)?;

    let original_leaves = config_leaves(original)?;
    let updated_leaves = config_leaves(updated)?;

    let target = resolve_write_target(Path::new(path))?;
    let existing = if start_empty {
        None
    } else {
        read_existing(&target)?
    };
    let mut document: toml_edit::DocumentMut = match existing.as_deref() {
        Some(text) => text
            .parse()
            .map_err(|e| Error::config(format!("Failed to parse existing config {path}: {e}")))?,
        None => toml_edit::DocumentMut::new(),
    };

    // `BTreeMap` iteration is sorted, so application order (and the order of
    // newly created keys) is deterministic regardless of `HashMap` seeds.
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

/// Flatten a config into sorted `dotted-path -> leaf value` pairs. Arrays are
/// single leaves; every entry of a table (including each `language.icons`
/// entry) is its own leaf.
///
/// # Errors
///
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

/// Convert a leaf value into its `toml_edit` equivalent.
///
/// # Errors
///
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
///
/// Returns an error if a parent key exists but is not a table, or the value
/// cannot be converted.
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

/// Resolve a symlinked config path to its target so the link survives a write.
///
/// # Errors
///
/// Returns an error if `path` is a symlink that cannot be resolved.
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

/// Read the file if present; `Ok(None)` when it does not exist.
///
/// # Errors
///
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

/// Write `contents` to a temp file next to `target`, then rename it over
/// `target`, keeping the existing file's permissions.
///
/// # Errors
///
/// Returns an error if the directory, temp file, or rename fails.
fn write_atomically(target: &Path, contents: &str) -> Result<()> {
    use std::io::Write as _;

    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| Error::config(format!("Failed to create config directory: {e}")))?;

    let file_name = target.file_name().map_or_else(
        || "config.toml".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let temp = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents.as_bytes())?;
        if let Ok(meta) = std::fs::metadata(target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        std::fs::rename(&temp, target)
    })();

    write_result.map_err(|e| {
        drop(std::fs::remove_file(&temp));
        Error::config(format!("Failed to write config file: {e}"))
    })
}

/// Load theme configuration from ~/.`config/gpy/themes/{theme_name}.toml`
///
/// Always reads and parses fresh from disk (see #608 -- this used to cache
/// the result in a process-global static; every caller is a short-lived,
/// one-shot CLI process for which a cache bought nothing but staleness).
///
/// # Errors
///
/// Returns an error if the theme file cannot be read or parsed, or if
/// `theme_name` does not resolve to a user theme, plugin theme, or builtin
/// (#572).
pub fn load_theme(theme_name: &str) -> Result<crate::theme::ThemeConfig> {
    // Resolve via the shared resolver (#572): the same resolution the daemon's
    // `ThemeManager::theme_path` uses, so oneshot can see plugin themes the
    // daemon can render, and an unresolvable name errors instead of silently
    // rendering a blank theme. This intentionally converges oneshot's
    // resolution with `crate::paths::config_root_for` (via
    // `ThemeManager::user_theme_path`), including for the previously-diverging
    // "XDG_CONFIG_HOME/HOME set to an empty string" edge case #477 preserved
    // in this function's now-deleted, hand-rolled `format!`-based predecessor
    // -- the daemon's own resolver already used `config_root_for` for that
    // case, so this unification makes oneshot match the daemon instead of
    // disagreeing with it, which is the point of #572.
    let theme_path = crate::theme::ThemeManager::resolve_path(theme_name).ok_or_else(|| {
        Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        ))
    })?;

    load_theme_from_path(&theme_path.display().to_string())
}

/// Load theme directly from a known file path.
///
/// Exposed for testing and tooling that want to load themes without relying on
/// global configuration discovery.
///
/// # Errors
///
/// Returns a configuration error if the file cannot be read, parsed, or fails
/// schema validation. Also returns an error naming the theme when `path`
/// matches neither a real file nor a builtin theme name (#572) -- this used
/// to silently return `ThemeConfig::default()`.
pub fn load_theme_from_path(path: &str) -> Result<crate::theme::ThemeConfig> {
    use crate::theme::manager::BUILTIN_THEMES;

    let contents = if Path::new(path).exists() {
        std::fs::read_to_string(path)
            .map_err(|e| Error::config(format!("Failed to read theme file {path}: {e}")))?
    } else if let Some((_, content)) = BUILTIN_THEMES.iter().find(|(name, _)| {
        // `Path::file_stem`, not a hardcoded `/`-separator suffix match: `path`
        // is built from a `PathBuf` (via `.display()`), so on Windows it is
        // `\`-separated, and `path.ends_with("/default.toml")` never matched
        // there -- a non-existent user theme file silently fell all the way
        // through to the bare `ThemeConfig::default()` stub instead of the
        // embedded builtin, dropping every icon/color the real theme sets.
        // `Path::new(path)` parses separators per the HOST's convention, which
        // is correct here because `path` was always built on this same host.
        Path::new(path).file_stem().and_then(|s| s.to_str()) == Some(*name)
    }) {
        (*content).to_owned()
    } else {
        let theme_name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        return Err(Error::config(format!(
            "theme '{theme_name}' not found (no user theme, plugin theme, or builtin matches this name)"
        )));
    };

    crate::theme::parse(&contents, path)
}

#[cfg(test)]
mod load_theme_from_path_tests {
    use super::load_theme_from_path;

    /// Regression lock for #482/discovered-live: `load_theme_from_path`'s builtin-theme
    /// fallback used to match via `path.ends_with(&format!("/{name}.toml"))`, a hardcoded `/`.
    ///
    /// `path` here is always built from a `PathBuf` via `.display()`, so on Windows it is
    /// `\`-separated and never matched -- confirmed on a real Windows VM
    /// (`scripts/test-windows-vm.sh`) that a non-existent user theme path like
    /// `C:\Users\x\AppData\Local\gpy\themes\default.toml` fell all the way through to the bare
    /// `ThemeConfig::default()` stub instead of the embedded `default` theme, dropping its
    /// `ui.prompt_close` icon (the symptom that surfaced as
    /// `clock_demo_spans_renders_closing_cap_on_the_real_default_theme` failing there).
    ///
    /// `std::path::MAIN_SEPARATOR` rather than a hardcoded `/`, so this test
    /// exercises the same separator convention `load_theme_from_path` itself
    /// resolves on whichever host it runs on -- meaningful proof on any
    /// platform, unlike a literal `\`-joined string tested from a Unix host
    /// (`Path` only parses `\` as a separator when the crate is actually
    /// compiled for Windows, so that specific case can only be verified by
    /// actually running there).
    ///
    /// # Panics
    ///
    /// Panics if a non-existent user theme path fails to fall back to the
    /// embedded builtin theme.
    #[test]
    fn nonexistent_user_theme_path_falls_back_to_embedded_builtin() {
        let sep = std::path::MAIN_SEPARATOR;
        let fake_user_theme_path = format!("nonexistent-dir{sep}gpy{sep}themes{sep}default.toml");

        let theme =
            load_theme_from_path(&fake_user_theme_path).expect("embedded fallback must succeed");

        // `UiTheme::default()` (the bare stub) sets `prompt_close` to
        // `Some(DelimiterConfig::simple(""))` -- an EMPTY icon, per
        // `default_ui_prompt_close()` -- so `Option::is_some()` can't tell
        // the embedded theme apart from the stub; both have a `Some`. The
        // icon TEXT is what actually differs: the stub's is empty, the real
        // embedded `default` theme's is the Nerd Font powerline glyph
        // U+E0B4 (confirmed against `config/themes/default.toml`, same as
        // `clock_demo_spans_renders_closing_cap_on_the_real_default_theme`).
        assert_eq!(
            theme.ui.get_prompt_close_delimiter(),
            "\u{e0b4}",
            "expected the embedded `default` theme's real closing-cap icon, got the bare \
             ThemeConfig::default() stub's empty one instead -- builtin-theme matching didn't fire"
        );
    }

    /// Regression lock for #572: this used to assert the OPPOSITE.
    ///
    /// A path matching no real file and no builtin theme name silently fell
    /// back to `ThemeConfig::default()`'s bare stub, which is exactly the
    /// silent-blank-theme bug #572 fixes (`ui.theme = "nrod"` rendering a
    /// blank prompt with no error). It now must error, naming the theme.
    ///
    /// # Panics
    ///
    /// Panics if a path that matches neither a real file nor any builtin
    /// theme name doesn't return an error naming the theme.
    #[test]
    fn path_matching_no_builtin_theme_name_returns_an_error() {
        let err = load_theme_from_path("nonexistent-dir/not-a-real-theme.toml")
            .expect_err("a path matching no builtin theme name must error, not silently succeed");

        let message = err.to_string();
        assert!(
            message.contains("not-a-real-theme"),
            "error message must name the theme (its file stem), got: {message}"
        );
    }
}
