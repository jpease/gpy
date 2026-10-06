//! TOML configuration schema definitions
//!
//! Defines the structure and validation rules for GPY configuration files
//! with the simplified ~15 key structure.

/// Get example TOML configuration content
#[must_use]
pub const fn get_example_config() -> &'static str {
    r#"# GPY Agent Configuration
# This is the simplified configuration with ~15 essential keys

[agent]
# Enable background agent process
enabled = true
# Request timeout in seconds
timeout_seconds = 5
# Enable live prompt updates via signals
live_updates = true

[agent.supervisor]
# Enable supervisor for automatic agent restart
enabled = true
# Supervisor health check interval in seconds
check_interval_seconds = 30
# Maximum restart attempts before giving up
max_restart_attempts = 5

[git]
# Enable git status detection
enabled = true
# Show ahead/behind commit counts
show_upstream = true
# Git command timeout in seconds
timeout_seconds = 10
# Maximum branch name length to display (0 = unlimited)
max_branch_length = 0
# Maximum ahead/behind count to report (0 = unlimited); higher counts are
# clamped to this value and shown with a trailing "+"
max_ahead_behind = 999
# Watch the working tree (not just .git) for instant updates on file edits
# (honors .gitignore). Override at runtime with the GPY_WATCH_WORKTREE env var.
watch_worktree = true

[language]
# Enable programming language detection
enabled = true
# Show language versions
show_versions = true
# Version cache TTL in hours
cache_ttl_hours = 24
# Specific languages to detect (empty = detect all)
# enabled_languages = ["rust", "node", "python", "go"]
enabled_languages = []

[ui]
# Show language/tool icons
show_icons = true
# Color theme: default, text, starship
theme = "default"

[ui.directory]
# Display mode: basename, abbreviated, truncated, full
display = "basename"
# Trailing path components kept when display = "truncated"
truncation_length = 3
# Prefix shown before a truncated path (e.g. "…/")
truncation_symbol = ""
# Maximum path length to display (character cap)
max_length = 80
# Anchor the path at the enclosing git repo root (affects truncated/full)
truncate_to_repo = false
"#
}

/// Configuration file locations in priority order, for an explicit
/// environment.
///
/// Pure: every input is a parameter, so the whole precedence chain is
/// table-testable without `std::env::set_var` (unavailable under this crate's
/// `#![forbid(unsafe_code)]` on edition 2024). [`get_config_paths`] is the
/// thin wrapper that reads the ambient environment and calls this.
///
/// Two normalisations, both #626, both shared with the Fish resolver
/// (`__gpy_user_config_candidates`):
///
/// - An **empty** `GPY_CONFIG_PATH` is dropped rather than kept as an empty
///   highest-priority candidate. `commands::utils::active_config_path`
///   returns the first candidate when none exists, so an empty first entry
///   meant `gpy config set` would write to the path `""`.
/// - `XDG_CONFIG_HOME` goes through [`crate::paths::xdg_value`], so an empty
///   or relative value falls through to `$HOME/.config` instead of building
///   `/gpy/config.toml` at the filesystem root.
///
/// There is deliberately no `$HOME/.gpy.toml` candidate: Fish used to offer
/// one that this list never had, so a user with that file got one config in
/// the prompt and a different one from the CLI (#626). `.gpy.toml` is a
/// project-local override, resolved relative to the current directory only.
#[must_use]
pub fn config_candidates_for(
    custom_path: Option<&str>,
    xdg_config: Option<&str>,
    home: Option<&str>,
) -> Vec<String> {
    [
        custom_path
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned),
        crate::paths::xdg_value(xdg_config, crate::paths::Os::Unix)
            .map(|xdg| format!("{xdg}/gpy/config.toml")),
        home.map(|h| format!("{h}/.config/gpy/config.toml")),
        Some(".gpy.toml".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Configuration file locations in priority order.
///
/// Ambient-environment wrapper over [`config_candidates_for`]; `HOME` comes
/// from [`crate::paths::home_dir`], which falls back to the passwd database
/// exactly as Fish and Zsh do (#626).
#[must_use]
pub fn get_config_paths() -> Vec<String> {
    let custom_path =
        std::env::var_os("GPY_CONFIG_PATH").map(|value| value.to_string_lossy().into_owned());
    let home = crate::paths::home_dir();
    let xdg_config =
        std::env::var_os("XDG_CONFIG_HOME").map(|value| value.to_string_lossy().into_owned());

    config_candidates_for(
        custom_path.as_deref(),
        xdg_config.as_deref(),
        home.as_deref(),
    )
}

/// Pick the config file in effect from `candidates` (priority order).
///
/// Returns `(write_path, existing)`: the path a command should write to, and
/// the file to load, where `None` means no candidate exists and the built-in
/// defaults apply. The first existing candidate is both; when none exists the
/// write path is the highest-priority candidate, so a new file lands where it
/// will actually be loaded from rather than at a shadowed lower-priority
/// location. `None` overall means the candidate list is empty.
///
/// The one rule behind the CLI's reads and writes
/// (`commands::utils::active_config_path`), `gpy-agent status`, and every
/// `ConfigManager` load and reload, so they cannot drift apart (#788, #179).
#[must_use]
pub fn resolve_active_config<P: AsRef<std::path::Path>>(
    candidates: &[P],
) -> Option<(std::path::PathBuf, Option<std::path::PathBuf>)> {
    candidates
        .iter()
        .map(AsRef::as_ref)
        .find(|candidate| candidate.exists())
        .map(|existing| (existing.to_path_buf(), Some(existing.to_path_buf())))
        .or_else(|| {
            candidates
                .first()
                .map(|first| (first.as_ref().to_path_buf(), None))
        })
}

// Configuration reference (see docs/ARCHITECTURE.md):
// - Layout: agent, git, language, ui sections with strictly validated defaults
// - Environment variable overrides limited to the documented set (e.g., GPY_AGENT_SOCKET_PATH)
// - Validation: bounds checking for timeouts/TTL, theme existence, language list filtering

#[cfg(test)]
mod config_path_tests {
    use super::config_candidates_for;

    /// One row: `GPY_CONFIG_PATH`, `XDG_CONFIG_HOME`, `HOME`, then the
    /// expected candidate list.
    type CandidateCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        &'static [&'static str],
    );

    /// Every row of the candidate precedence, hoisted out of the test body
    /// so the assertion loop stays short.
    const CANDIDATE_CASES: [CandidateCase; 8] = [
        (
            Some("/custom.toml"),
            Some("/xdgcfg"),
            Some("/home/u"),
            &[
                "/custom.toml",
                "/xdgcfg/gpy/config.toml",
                "/home/u/.config/gpy/config.toml",
                ".gpy.toml",
            ],
        ),
        // A relative GPY_CONFIG_PATH is still honoured: unlike the XDG
        // variables, it is an explicit "use this file" and the spec's
        // absolute-path rule does not cover it.
        (
            Some("custom.toml"),
            None,
            Some("/home/u"),
            &[
                "custom.toml",
                "/home/u/.config/gpy/config.toml",
                ".gpy.toml",
            ],
        ),
        // Empty GPY_CONFIG_PATH is unset, not an empty highest-priority
        // candidate that `gpy config set` would write to (#626).
        (
            Some(""),
            Some("/xdgcfg"),
            Some("/home/u"),
            &[
                "/xdgcfg/gpy/config.toml",
                "/home/u/.config/gpy/config.toml",
                ".gpy.toml",
            ],
        ),
        // Empty XDG_CONFIG_HOME is unset: it used to build
        // "/gpy/config.toml" at the filesystem root.
        (
            None,
            Some(""),
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml", ".gpy.toml"],
        ),
        // A relative XDG_CONFIG_HOME is invalid and ignored.
        (
            None,
            Some("cfg"),
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml", ".gpy.toml"],
        ),
        (
            None,
            None,
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml", ".gpy.toml"],
        ),
        // No home at all (`paths::home_dir` found neither HOME nor a
        // passwd entry): only the project-local override remains.
        (None, None, None, &[".gpy.toml"]),
        // There is no `$HOME/.gpy.toml` candidate, in any environment:
        // Fish used to offer one this list never had (#626).
        (
            None,
            Some("/xdgcfg"),
            Some("/home/u"),
            &[
                "/xdgcfg/gpy/config.toml",
                "/home/u/.config/gpy/config.toml",
                ".gpy.toml",
            ],
        ),
    ];

    /// The candidate list's precedence, including #626's two normalisations.
    ///
    /// Expressed against the pure resolver because the environment cannot be
    /// mutated in-process: `std::env::set_var` is unavailable under
    /// `#![forbid(unsafe_code)]` on edition 2024.
    ///
    /// # Panics
    ///
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn config_candidate_precedence() {
        for (custom, xdg_config, home, expected) in CANDIDATE_CASES {
            assert_eq!(
                config_candidates_for(custom, xdg_config, home),
                expected.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                "candidates for (GPY_CONFIG_PATH={custom:?}, XDG_CONFIG_HOME={xdg_config:?}, HOME={home:?})"
            );
        }
    }
}

#[cfg(test)]
mod resolve_active_config_tests {
    #![allow(clippy::missing_panics_doc)]

    use super::resolve_active_config;
    use std::path::PathBuf;

    /// No candidate exists: nothing to load, and the write target is the
    /// highest-priority candidate.
    #[test]
    fn none_exist_writes_to_the_first_candidate_and_loads_nothing() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let candidates = [temp.path().join("a.toml"), temp.path().join("b.toml")];

        assert_eq!(
            resolve_active_config(&candidates),
            Some((temp.path().join("a.toml"), None))
        );
    }

    /// Only a lower-priority candidate exists: it is the active file, for
    /// reading and writing alike.
    #[test]
    fn only_a_lower_candidate_existing_is_active() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let lower = temp.path().join("b.toml");
        std::fs::write(&lower, "").expect("write lower");
        let candidates = [temp.path().join("a.toml"), lower.clone()];

        assert_eq!(
            resolve_active_config(&candidates),
            Some((lower.clone(), Some(lower)))
        );
    }

    /// Both exist: the higher-priority one shadows the other.
    #[test]
    fn the_highest_priority_existing_candidate_wins() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let higher = temp.path().join("a.toml");
        let lower = temp.path().join("b.toml");
        std::fs::write(&higher, "").expect("write higher");
        std::fs::write(&lower, "").expect("write lower");

        assert_eq!(
            resolve_active_config(&[higher.clone(), lower]),
            Some((higher.clone(), Some(higher)))
        );
    }

    /// An empty candidate list has no write target at all.
    #[test]
    fn no_candidates_resolves_to_nothing() {
        assert_eq!(resolve_active_config::<PathBuf>(&[]), None);
    }
}
