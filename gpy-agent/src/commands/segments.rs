//! `gpy segments`, `gpy enable`, and `gpy disable` command handlers.

use super::output;
use super::utils::{active_config_path, load_active_config, read_config, reload_agent_and_notify};
use crate::config::Config;
use crate::config::loader::save_config;
use crate::plugin::{BUILTIN_ORDER, BuiltinSegment, SegmentName, discover_plugins};
use crate::shell::Shell;
use crate::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

/// Enable a prompt segment, inserting a built-in at its canonical position.
///
/// # Errors
/// Returns an error if the segment name is invalid or config cannot be saved.
pub fn enable(segment: &str) -> Result<()> {
    let canonical = canonicalize_segment(segment)?;
    ensure_segment_known(&canonical, true)?;

    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    apply_enable(&mut config, &canonical);
    save_config(&path, &original, &config, &[])?;

    output::plain(format_args!("Enabled {canonical} segment"));
    if is_file_plugin_segment(&canonical)
        && let Some(note) = skipped_by_note(
            &canonical,
            &super::setup::installed_integrations()
                .iter()
                .map(|integration| integration.shell)
                .collect::<Vec<_>>(),
        )
    {
        output::plain(note);
    }
    reload_agent_and_notify();
    Ok(())
}

/// Whether `name` comes from a discovered v1 `.fish` plugin (`entry_type = "file"`).
///
/// `script` plugins run in every shell, so they never get the Fish-only note.
fn is_file_plugin_segment(name: &str) -> bool {
    BuiltinSegment::try_from(name).is_err()
        && discover_plugins().plugins.iter().any(|plugin| {
            !matches!(plugin.source, crate::plugin::PluginSource::FirstParty)
                && matches!(plugin.manifest.entry_type, crate::plugin::EntryType::File)
                && plugin
                    .manifest
                    .provided_segments
                    .iter()
                    .any(|segment| segment.as_str() == name)
        })
}

/// The note `gpy enable` prints for a v1 plugin segment when a Bash or Zsh
/// integration is installed; `None` when only Fish (or nothing) is installed.
///
/// v1 plugin segments are `.fish` files, which only Fish sources.
fn skipped_by_note(segment: &str, installed: &[Shell]) -> Option<String> {
    let skipping: Vec<&str> = installed
        .iter()
        .filter(|shell| matches!(shell, Shell::Bash | Shell::Zsh))
        .map(|shell| shell.as_str())
        .collect();
    if skipping.is_empty() {
        return None;
    }
    Some(format!(
        "Note: {segment} is a v1 `.fish` plugin segment, so the {} prompt{} will skip it. \
         Run `gpy disable {segment}` to drop it, or use Fish.",
        skipping.join(" and "),
        if skipping.len() > 1 { "s" } else { "" },
    ))
}

/// Disable a prompt segment.
///
/// Removes the entry from `ui.enabled_segments`; a later `gpy enable` puts a
/// built-in back at its default position.
///
/// # Errors
/// Returns an error if the segment name is invalid or config cannot be saved.
pub fn disable(segment: &str) -> Result<()> {
    let canonical = canonicalize_segment(segment)?;
    ensure_segment_known(&canonical, false)?;

    remove_from_enabled_segments(&canonical)?;

    output::plain(format_args!("Disabled {canonical} segment"));
    reload_agent_and_notify();
    Ok(())
}

/// List all available prompt segments with their owner and whether they are
/// on by default.
///
/// # Errors
/// Returns an error if the active config file cannot be loaded.
pub fn list() -> Result<()> {
    let config = read_config()?;
    let available = available_segments();
    let plugin_owners = plugin_segment_owners();
    let defaults = Config::default().ui.enabled_segments;

    output::heading("Prompt Segments:");

    let rows: Vec<(&String, String)> = available
        .iter()
        .map(|segment| {
            let owner = if BuiltinSegment::try_from(segment.as_str()).is_ok() {
                crate::plugin::registry::FIRST_PARTY_ID.to_owned()
            } else {
                plugin_owners.get(segment).cloned().unwrap_or_default()
            };
            (segment, owner)
        })
        .collect();
    let name_width = rows
        .iter()
        .map(|(segment, _)| segment.chars().count())
        .max()
        .unwrap_or_default();
    let owner_width = rows
        .iter()
        .map(|(_, owner)| owner.chars().count())
        .max()
        .unwrap_or_default();

    for (segment, owner) in &rows {
        let status = if config.segment_enabled(segment) {
            "x"
        } else {
            " "
        };
        if defaults.contains(segment) {
            output::plain(format_args!(
                "[{status}] {segment:<name_width$}  {owner:<owner_width$}  default"
            ));
        } else {
            output::plain(format_args!("[{status}] {segment:<name_width$}  {owner}"));
        }
    }

    Ok(())
}

/// Plugin-provided segment name → `<plugin id> (<source>)`; a later source
/// (user over bundled) wins.
fn plugin_segment_owners() -> BTreeMap<String, String> {
    let mut owners = BTreeMap::new();
    for plugin in discover_plugins().plugins {
        if matches!(plugin.source, crate::plugin::PluginSource::FirstParty) {
            continue;
        }
        for segment in &plugin.manifest.provided_segments {
            let _ = owners.insert(
                segment.to_string(),
                format!("{} ({})", plugin.manifest.id, plugin.source),
            );
        }
    }
    owners
}

/// Canonicalize a segment name.
fn canonicalize_segment(segment: &str) -> Result<String> {
    let normalized = if segment == "lang" {
        "language"
    } else {
        segment
    };
    let name = SegmentName::new(normalized).map_err(|e| Error::Config {
        message: format!("invalid segment '{segment}'"),
        source: Some(Box::new(e)),
    })?;
    Ok(name.to_string())
}

/// Apply `gpy disable <segment>` to an in-memory config: drop the list entry.
fn apply_disable(config: &mut Config, segment: &str) {
    config.ui.enabled_segments.retain(|s| s != segment);
}

/// Apply `gpy enable <segment>` to an in-memory config.
fn apply_enable(config: &mut Config, segment: &str) {
    insert_enabled_segment(&mut config.ui.enabled_segments, segment);
}

/// Insert `segment` into `ui.enabled_segments` unless already listed. A
/// built-in goes at its canonical position: immediately before the first
/// later [`BUILTIN_ORDER`] built-in that is listed, else at the end, so the
/// relative order of every other entry is kept. Plugin segments are appended.
pub(crate) fn insert_enabled_segment(list: &mut Vec<String>, segment: &str) {
    if list.iter().any(|s| s == segment) {
        return;
    }
    if let Ok(builtin) = BuiltinSegment::try_from(segment)
        && let Some(at) = BUILTIN_ORDER
            .iter()
            .skip_while(|candidate| **candidate != builtin)
            .skip(1)
            .find_map(|later| list.iter().position(|s| s == later.as_str()))
    {
        list.insert(at, segment.to_owned());
        return;
    }
    list.push(segment.to_owned());
}

/// Remove a segment from the enabled segments list.
fn remove_from_enabled_segments(segment: &str) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    apply_disable(&mut config, segment);
    save_config(&path, &original, &config, &[])?;
    Ok(())
}

pub(crate) fn available_segments() -> Vec<String> {
    available_segments_with_plugin_dirs(&[
        crate::plugin::bundled_plugins_dir(),
        crate::plugin::user_plugins_dir(),
    ])
}

/// Core of [`available_segments`], parameterized on the plugin roots so tests
/// need not mutate the environment.
fn available_segments_with_plugin_dirs(plugin_dirs: &[std::path::PathBuf]) -> Vec<String> {
    let mut ordered = Vec::new();
    ordered.extend(
        BUILTIN_ORDER
            .iter()
            .map(|builtin| builtin.as_str().to_owned()),
    );

    // First-party plugins are skipped below, so only bundled and user plugins
    // can add segments; without either root the builtins are complete. Keeps
    // `gpy __complete segment` cheap on every TAB.
    if !plugin_dirs.iter().any(|dir| dir.exists()) {
        return ordered;
    }

    let mut plugin_segments: BTreeSet<String> = BTreeSet::new();
    for plugin in discover_plugins().plugins {
        if matches!(plugin.source, crate::plugin::PluginSource::FirstParty) {
            continue;
        }
        for segment in plugin.manifest.provided_segments {
            let segment_name = segment.to_string();
            if BuiltinSegment::try_from(segment_name.as_str()).is_err() {
                let _ = plugin_segments.insert(segment_name);
            }
        }
    }

    ordered.extend(plugin_segments);
    ordered
}

/// Verify that a segment exists in built-ins or discovered plugins.
fn ensure_segment_known(segment: &str, enabling: bool) -> Result<()> {
    if BuiltinSegment::try_from(segment).is_ok() {
        return Ok(());
    }

    let available = available_segments();
    if available.contains(&segment.to_owned()) {
        return Ok(());
    }

    // Allow disabling a segment whose plugin was removed.
    if !enabling {
        let config = read_config()?;
        if config.ui.enabled_segments.contains(&segment.to_owned()) {
            return Ok(());
        }
    }

    Err(Error::config(format!(
        "unknown segment '{segment}'; install a plugin that provides it or check the spelling"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn skipped_by_note_names_only_installed_bash_and_zsh() {
        assert_eq!(skipped_by_note("seg", &[]), None);
        assert_eq!(skipped_by_note("seg", &[Shell::Fish]), None);
        let bash = skipped_by_note("seg", &[Shell::Fish, Shell::Bash]).expect("bash note");
        assert!(bash.contains("the bash prompt will skip it"), "{bash}");
        assert!(bash.contains("gpy disable seg"), "{bash}");
        assert!(bash.contains("use Fish"), "{bash}");
        let both = skipped_by_note("seg", &[Shell::Zsh, Shell::Bash]).expect("both note");
        assert!(both.contains("zsh and bash prompts will skip it"), "{both}");
    }

    fn list_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn insert_enabled_segment_places_builtins_canonically_and_appends_plugins() {
        let mut list = list_of(&["git", "clock", "directory"]);
        insert_enabled_segment(&mut list, "language");
        assert_eq!(list, ["git", "clock", "language", "directory"]);

        insert_enabled_segment(&mut list, "language");
        assert_eq!(
            list,
            ["git", "clock", "language", "directory"],
            "existing name is a no-op"
        );

        insert_enabled_segment(&mut list, "status");
        assert_eq!(list, ["git", "clock", "language", "directory", "status"]);

        insert_enabled_segment(&mut list, "k8s");
        assert_eq!(
            list,
            ["git", "clock", "language", "directory", "status", "k8s"]
        );

        let mut no_later = list_of(&["directory", "clock"]);
        insert_enabled_segment(&mut no_later, "duration");
        assert_eq!(no_later, ["duration", "directory", "clock"]);
    }

    #[test]
    fn disable_git_removes_list_entry() {
        let mut config = Config::default();
        apply_disable(&mut config, "git");
        assert!(!config.ui.enabled_segments.iter().any(|s| s == "git"));
        assert!(!config.segment_enabled("git"));
    }

    #[test]
    fn reenabled_builtin_returns_to_canonical_position() {
        let mut config = Config::default();
        let default = config.ui.enabled_segments.clone();
        apply_disable(&mut config, "language");
        apply_enable(&mut config, "language");
        assert_eq!(config.ui.enabled_segments, default);
        assert_eq!(
            config.ui.enabled_segments,
            ["clock", "duration", "language", "directory", "git"]
        );
    }

    #[test]
    fn apply_enable_restores_list_entry() {
        let mut config = Config::default();
        config.ui.enabled_segments = list_of(&["clock", "duration", "directory"]);
        apply_enable(&mut config, "git");
        assert_eq!(
            config.ui.enabled_segments,
            vec!["clock", "duration", "directory", "git"]
        );
        assert!(config.segment_enabled("git"));

        apply_enable(&mut config, "language");
        assert_eq!(
            config.ui.enabled_segments,
            vec!["clock", "duration", "language", "directory", "git"]
        );
    }

    #[test]
    fn canonicalize_segment_accepts_plugin_names() {
        use std::error::Error as _;

        assert_eq!(canonicalize_segment("lang").unwrap(), "language");
        assert_eq!(canonicalize_segment("k8s-tools").unwrap(), "k8s-tools");
        let err = canonicalize_segment("bad segment").expect_err("should reject invalid name");
        assert!(err.source().is_some(), "error should preserve source cause");
    }

    #[test]
    fn available_segments_returns_builtins_only_without_plugin_dirs() {
        let temp = TempDir::new().expect("tempdir");
        let missing = [
            temp.path().join("share").join("plugins"),
            temp.path().join("gpy").join("plugins"),
        ];

        // The fast path must return exactly the builtins.
        let segments = available_segments_with_plugin_dirs(&missing);

        assert_eq!(
            segments,
            vec![
                "clock",
                "duration",
                "language",
                "directory",
                "git",
                "status"
            ],
            "with no plugin dir, available_segments() must equal BUILTIN_ORDER exactly"
        );
    }

    #[test]
    fn available_segments_takes_slow_path_when_a_plugin_dir_exists() {
        let temp = TempDir::new().expect("tempdir");
        let user_dir = temp.path().join("gpy").join("plugins");
        std::fs::create_dir_all(&user_dir).expect("create user plugin dir");

        // `discover_plugins()` reads the ambient environment, so only assert
        // the builtins survive; end-to-end coverage is in `gpy-agent/tests/gpy_cli_tests/`.
        let segments = available_segments_with_plugin_dirs(&[user_dir]);

        for builtin in BUILTIN_ORDER {
            assert!(
                segments.contains(&builtin.as_str().to_owned()),
                "builtin {builtin} missing from available_segments()"
            );
        }
    }
}
