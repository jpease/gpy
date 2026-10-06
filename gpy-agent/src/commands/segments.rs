//! `gpy segments`, `gpy enable`, and `gpy disable` command handlers.
//!
//! Segment commands edit the `ui.enabled_segments` configuration list while
//! preserving the typed config loading and persistence rules used by other CLI
//! settings. Rendering and detection of the actual prompt segments remains in
//! the Fish layer and formatter modules.

use super::utils::{
    active_config_path, load_active_config, read_config, reload_agent_and_notify, save_config_to,
};
use crate::config::Config;
use crate::plugin::{BUILTIN_ORDER, BuiltinSegment, SegmentName, discover_plugins};
use crate::{Error, Result};
use std::collections::BTreeSet;

/// A builtin segment whose enabled state also depends on a dedicated config field.
///
/// `git` and `language` render only when they are listed in
/// `ui.enabled_segments` **and** `config.git.enabled` /
/// `config.language.enabled` is set (see [`is_effectively_enabled`]).
/// [`BuiltinSegment`] has other variants (clock/duration/directory/status/
/// username/hostname) with no such field, so this is a separate,
/// exhaustively-matched type rather than widening `set_feature_enabled` to
/// accept a `BuiltinSegment` directly and needing a dead/unreachable arm for
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureToggle {
    Git,
    Language,
}

/// Whether the prompt will actually render `name` under `config`.
///
/// Mirrors the authoritative filter in `theme::export` (the list every shell
/// iterates): a segment must be listed in `ui.enabled_segments`, and `git` /
/// `language` additionally need `config.git.enabled` /
/// `config.language.enabled`. `gpy segments` and the wizard's starting
/// selection use this so they report what the prompt renders.
#[must_use]
pub(crate) fn is_effectively_enabled(config: &Config, name: &str) -> bool {
    let listed = config.ui.enabled_segments.iter().any(|s| s == name);
    listed
        && match BuiltinSegment::try_from(name) {
            Ok(BuiltinSegment::Git) => config.git.enabled,
            Ok(BuiltinSegment::Language) => config.language.enabled,
            _ => true,
        }
}

/// Enable a prompt segment
///
/// `git` / `language` also get their feature flag set, so the segment renders
/// even when only one of the two conditions was missing.
///
/// # Errors
///
/// Returns an error if the segment name is invalid or config cannot be saved.
pub fn enable(segment: &str) -> Result<()> {
    let canonical = canonicalize_segment(segment)?;
    ensure_segment_known(&canonical, true)?;

    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    apply_enable(&mut config, &canonical);
    save_config_to(&path, &original, &config, &[])?;

    println!("✅ Enabled {canonical} segment");
    reload_agent_and_notify();
    Ok(())
}

/// Disable a prompt segment
///
/// `git` / `language` only have their feature flag cleared: the list entry is
/// kept so a later `gpy enable` restores the original position.
///
/// # Errors
///
/// Returns an error if the segment name is invalid or config cannot be saved.
pub fn disable(segment: &str) -> Result<()> {
    let canonical = canonicalize_segment(segment)?;
    ensure_segment_known(&canonical, false)?;

    match BuiltinSegment::try_from(canonical.as_str()) {
        Ok(BuiltinSegment::Git) => set_feature_enabled(FeatureToggle::Git, false)?,
        Ok(BuiltinSegment::Language) => set_feature_enabled(FeatureToggle::Language, false)?,
        _ => remove_from_enabled_segments(&canonical)?,
    }

    println!("✅ Disabled {canonical} segment");
    reload_agent_and_notify();
    Ok(())
}

/// List all available prompt segments
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    let available = available_segments();

    println!("Prompt Segments:");
    println!("================\n");

    for segment in &available {
        let enabled = is_effectively_enabled(&config, segment);

        let status = if enabled { "✓" } else { " " };
        println!("[{status}] {segment}");
    }

    Ok(())
}

/// Canonicalize a segment name
///
/// # Errors
///
/// Returns an error if the segment name is not valid.
fn canonicalize_segment(segment: &str) -> Result<String> {
    let normalized = if segment == "lang" {
        "language"
    } else {
        segment
    };
    let name = SegmentName::new(normalized)
        .map_err(|_| Error::config(format!("Invalid segment: {segment}")))?;
    Ok(name.to_string())
}

/// Apply a feature toggle to an in-memory config.
///
/// Pure — no I/O, no config-path resolution — so it can be unit tested
/// directly (see `tests::apply_feature_toggle_*` below) without depending on
/// the active config path, which tests in this crate cannot override
/// in-process (`unsafe_code` is forbidden crate-wide, so no `std::env::set_var`).
const fn apply_feature_toggle(
    config: &mut crate::config::Config,
    feature: FeatureToggle,
    enabled: bool,
) {
    match feature {
        FeatureToggle::Git => config.git.enabled = enabled,
        FeatureToggle::Language => config.language.enabled = enabled,
    }
}

/// Set a feature enabled/disabled
///
/// # Errors
///
/// Returns an error if config cannot be saved.
fn set_feature_enabled(feature: FeatureToggle, enabled: bool) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    apply_feature_toggle(&mut config, feature, enabled);

    save_config_to(&path, &original, &config, &[])?;
    Ok(())
}

/// Apply `gpy enable <segment>` to an in-memory config.
///
/// Sets the `git` / `language` feature flag when `segment` is one of them,
/// then inserts `segment` into `ui.enabled_segments` if absent. Pure (no I/O)
/// so the enable rule is unit-testable and lands in one load/save cycle.
fn apply_enable(config: &mut Config, segment: &str) {
    match BuiltinSegment::try_from(segment) {
        Ok(BuiltinSegment::Git) => apply_feature_toggle(config, FeatureToggle::Git, true),
        Ok(BuiltinSegment::Language) => {
            apply_feature_toggle(config, FeatureToggle::Language, true);
        }
        _ => {}
    }
    insert_enabled_segment(config, segment);
}

/// Add `segment` to `ui.enabled_segments` if it is not already listed.
///
/// Builtins are placed in [`BUILTIN_ORDER`] (see [`rebuild_enabled_segments`]);
/// any other segment is appended.
fn insert_enabled_segment(config: &mut Config, segment: &str) {
    if config.ui.enabled_segments.iter().any(|s| s == segment) {
        return;
    }

    if let Ok(builtin_segment) = BuiltinSegment::try_from(segment) {
        let mut enabled_builtin: BTreeSet<BuiltinSegment> = config
            .ui
            .enabled_segments
            .iter()
            .filter_map(|s| BuiltinSegment::try_from(s.as_str()).ok())
            .collect();
        enabled_builtin.insert(builtin_segment);

        let plugin_segments: Vec<String> = config
            .ui
            .enabled_segments
            .iter()
            .filter(|s| BuiltinSegment::try_from(s.as_str()).is_err())
            .cloned()
            .collect();

        config.ui.enabled_segments =
            rebuild_enabled_segments(|b| enabled_builtin.contains(&b), plugin_segments);
    } else {
        config.ui.enabled_segments.push(segment.to_owned());
    }
}

/// Rebuild an ordered `enabled_segments` list: builtins from [`BUILTIN_ORDER`]
/// that `is_enabled` accepts, in `BUILTIN_ORDER`'s order, followed by
/// `plugin_segments` in the order given.
///
/// Pure — no I/O, no config reads — so both `insert_enabled_segment` (the
/// `gpy enable` CLI path) and `WizardState::apply_to` (the wizard's save
/// path) can share one rebuild implementation instead of maintaining two
/// near-identical copies.
pub(crate) fn rebuild_enabled_segments(
    is_enabled: impl Fn(BuiltinSegment) -> bool,
    plugin_segments: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut rebuilt: Vec<String> = BUILTIN_ORDER
        .iter()
        .copied()
        .filter(|builtin| is_enabled(*builtin))
        .map(BuiltinSegment::as_str)
        .map(str::to_owned)
        .collect();
    rebuilt.extend(plugin_segments);
    rebuilt
}

/// Remove a segment from enabled segments list
///
/// # Errors
///
/// Returns an error if config cannot be saved.
fn remove_from_enabled_segments(segment: &str) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    config.ui.enabled_segments.retain(|s| s != segment);
    save_config_to(&path, &original, &config, &[])?;
    Ok(())
}

pub(crate) fn available_segments() -> Vec<String> {
    available_segments_with_user_dir(&crate::plugin::user_plugins_dir())
}

/// Core of [`available_segments`], parameterized on the user plugin directory.
///
/// This lets tests exercise the fast-path/discovery split without mutating
/// process-wide environment variables (this crate forbids `unsafe_code`, so
/// tests cannot call `std::env::set_var` themselves).
fn available_segments_with_user_dir(user_plugin_dir: &std::path::Path) -> Vec<String> {
    let mut ordered = Vec::new();
    ordered.extend(
        BUILTIN_ORDER
            .iter()
            .map(|builtin| builtin.as_str().to_owned()),
    );

    // Fast path (#346): the loop below already skips `PluginSource::FirstParty`
    // entries, so only a *user-installed* plugin (under `user_plugins_dir()`)
    // can ever add a segment beyond `BUILTIN_ORDER`. In normal (non-test)
    // operation that is the only non-first-party root `discover_plugins()`
    // scans — `GPY_BUNDLED_PLUGIN_DIR` is test-only plumbing and unset in
    // production. So when the user plugin directory doesn't exist, builtins
    // are already the complete, correct answer: skip the read_dir + every
    // `plugin.toml` parse entirely rather than doing that work just to
    // discard it. This is the common case (no plugins installed) and is what
    // makes `gpy __complete segment` cheap on every TAB press.
    if !user_plugin_dir.exists() {
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
///
/// # Errors
///
/// Returns an error if the segment is unknown and cannot be safely enabled/disabled.
fn ensure_segment_known(segment: &str, enabling: bool) -> Result<()> {
    if BuiltinSegment::try_from(segment).is_ok() {
        return Ok(());
    }

    let available = available_segments();
    if available.contains(&segment.to_owned()) {
        return Ok(());
    }

    // Allow disabling unknown segments if they are still in config (plugin removed).
    if !enabling {
        let config = read_config()?;
        if config.ui.enabled_segments.contains(&segment.to_owned()) {
            return Ok(());
        }
    }

    Err(Error::config(format!(
        "Unknown segment '{segment}'. Install a plugin that provides it or check segment spelling."
    )))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn apply_feature_toggle_sets_git_independently_of_language() {
        let mut config = crate::config::Config::default();
        assert!(config.git.enabled, "git.enabled should default to true");
        assert!(
            config.language.enabled,
            "language.enabled should default to true"
        );

        apply_feature_toggle(&mut config, FeatureToggle::Git, false);
        assert!(!config.git.enabled);
        assert!(
            config.language.enabled,
            "toggling git must not affect language"
        );

        apply_feature_toggle(&mut config, FeatureToggle::Git, true);
        assert!(config.git.enabled);
    }

    #[test]
    fn apply_feature_toggle_sets_language_independently_of_git() {
        let mut config = crate::config::Config::default();

        apply_feature_toggle(&mut config, FeatureToggle::Language, false);
        assert!(!config.language.enabled);
        assert!(config.git.enabled, "toggling language must not affect git");

        apply_feature_toggle(&mut config, FeatureToggle::Language, true);
        assert!(config.language.enabled);
    }

    /// A config listing `segments`, with both `git.enabled` and
    /// `language.enabled` set to `flags`.
    fn config_with(segments: &[&str], flags: bool) -> Config {
        let mut config = Config::default();
        config.ui.enabled_segments = segments.iter().map(|s| (*s).to_owned()).collect();
        config.git.enabled = flags;
        config.language.enabled = flags;
        config
    }

    #[test]
    fn is_effectively_enabled_requires_flag_and_list_membership() {
        // flag true + absent from the list: not rendered.
        let absent = config_with(&["directory"], true);
        assert!(!is_effectively_enabled(&absent, "git"));
        assert!(!is_effectively_enabled(&absent, "language"));

        // flag false + present in the list: not rendered.
        let flag_off = config_with(&["language", "directory", "git"], false);
        assert!(!is_effectively_enabled(&flag_off, "git"));
        assert!(!is_effectively_enabled(&flag_off, "language"));

        // flag true + present: rendered.
        let both = config_with(&["language", "directory", "git"], true);
        assert!(is_effectively_enabled(&both, "git"));
        assert!(is_effectively_enabled(&both, "language"));

        // Other segments are plain list membership.
        assert!(is_effectively_enabled(&absent, "directory"));
        assert!(!is_effectively_enabled(&absent, "clock"));
    }

    #[test]
    fn apply_enable_git_sets_flag_and_restores_list_entry() {
        let mut config = config_with(&["clock", "duration", "directory"], false);
        apply_enable(&mut config, "git");
        assert!(config.git.enabled);
        assert_eq!(
            config.ui.enabled_segments,
            vec!["clock", "duration", "directory", "git"]
        );
        assert!(is_effectively_enabled(&config, "git"));

        apply_enable(&mut config, "language");
        assert!(config.language.enabled);
        assert_eq!(
            config.ui.enabled_segments,
            vec!["clock", "duration", "language", "directory", "git"]
        );
    }

    #[test]
    fn canonicalize_segment_accepts_plugin_names() {
        assert_eq!(canonicalize_segment("lang").unwrap(), "language");
        assert_eq!(canonicalize_segment("k8s-tools").unwrap(), "k8s-tools");
        assert!(canonicalize_segment("bad segment").is_err());
    }

    #[test]
    fn available_segments_returns_builtins_only_when_no_user_plugin_dir() {
        let temp = TempDir::new().expect("tempdir");
        // Never created: exercises the "directory does not exist" branch.
        let missing_user_dir = temp.path().join("gpy").join("plugins");

        // With no user plugin dir, the `available_segments()` fast path
        // (#346) must short-circuit before ever calling `discover_plugins()`
        // and return exactly the builtins.
        let segments = available_segments_with_user_dir(&missing_user_dir);

        assert_eq!(
            segments,
            vec![
                "clock",
                "duration",
                "language",
                "directory",
                "git",
                "status",
                "username",
                "hostname"
            ],
            "with no user plugin dir, available_segments() must equal BUILTIN_ORDER exactly"
        );
    }

    #[test]
    fn available_segments_takes_slow_path_when_user_dir_exists() {
        let temp = TempDir::new().expect("tempdir");
        let user_dir = temp.path().join("gpy").join("plugins");
        std::fs::create_dir_all(&user_dir).expect("create user plugin dir");

        // An existing user plugin dir must not take the fast-path early
        // return; it falls through to `discover_plugins()` (real behavior
        // covered end-to-end by `test_enable_discovered_plugin_segment` in
        // `gpy-agent/tests/gpy_cli_tests.rs`, which sets up an isolated
        // process environment). Here we only assert the builtins are still
        // present, since `discover_plugins()` itself reads the *ambient*
        // process environment rather than `user_dir` (this crate forbids
        // `unsafe_code`, so unit tests cannot override that env in-process).
        let segments = available_segments_with_user_dir(&user_dir);

        for builtin in BUILTIN_ORDER {
            assert!(
                segments.contains(&builtin.as_str().to_owned()),
                "builtin {builtin} missing from available_segments()"
            );
        }
    }
}
