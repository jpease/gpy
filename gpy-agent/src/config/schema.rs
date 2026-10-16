//! TOML configuration schema and config-file resolution.

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
# Collapse each submitted prompt to clock, prompt icon and command (Fish 4.2+, Zsh)
transient_prompt = true

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

/// Configuration file locations in priority order, for an explicit environment.
///
/// Pure so precedence is testable without `set_var` (forbidden by `unsafe_code`).
/// Empty `GPY_CONFIG_PATH` is dropped (else `config set` writes to `""`);
/// `XDG_CONFIG_HOME` goes through [`crate::paths::xdg_value`]. Matches Fish's
/// `__gpy_user_config_candidates`. No `~/.gpy.toml` or project-local file: the
/// daemon serves every directory, so CLI and prompt must agree on one file.
#[must_use]
pub fn config_candidates_for(
    custom_path: Option<&str>,
    xdg_config: Option<&str>,
    home: Option<&str>,
    os: crate::paths::Os,
) -> Vec<String> {
    [
        custom_path
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned),
        crate::paths::xdg_value(xdg_config, os).map(|xdg| format!("{xdg}/gpy/config.toml")),
        crate::paths::xdg_value(home, os).map(|h| format!("{h}/.config/gpy/config.toml")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Configuration file locations in priority order, from the ambient environment.
///
/// A relative `GPY_CONFIG_PATH` is made absolute against the cwd at start, so
/// the daemon (in `/`) and CLI watch the same path.
#[must_use]
pub fn get_config_paths() -> Vec<String> {
    let custom_path = crate::paths::root_var_os("GPY_CONFIG_PATH").map(|value| {
        crate::paths::absolutize(std::path::Path::new(&value))
            .to_string_lossy()
            .into_owned()
    });
    let home = crate::paths::home_dir();
    let xdg_config = crate::paths::root_var_os("XDG_CONFIG_HOME")
        .map(|value| value.to_string_lossy().into_owned());

    config_candidates_for(
        custom_path.as_deref(),
        xdg_config.as_deref(),
        home.as_deref(),
        crate::paths::Os::host(),
    )
}

/// Pick the config file in effect from `candidates` (priority order).
///
/// Returns `(write_path, existing)`; with no existing file, writes target the
/// highest-priority candidate so new files aren't shadowed. Shared by CLI,
/// status and `ConfigManager` so they cannot drift.
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

#[cfg(test)]
mod config_path_tests {
    use super::config_candidates_for;
    use crate::paths::Os;

    /// `GPY_CONFIG_PATH`, `XDG_CONFIG_HOME`, `HOME`, expected candidates.
    type CandidateCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        &'static [&'static str],
    );

    const CANDIDATE_CASES: [CandidateCase; 9] = [
        (
            Some("/custom.toml"),
            Some("/xdgcfg"),
            Some("/home/u"),
            &[
                "/custom.toml",
                "/xdgcfg/gpy/config.toml",
                "/home/u/.config/gpy/config.toml",
            ],
        ),
        // Relative GPY_CONFIG_PATH passes through; `get_config_paths` absolutizes it.
        (
            Some("custom.toml"),
            None,
            Some("/home/u"),
            &["custom.toml", "/home/u/.config/gpy/config.toml"],
        ),
        (
            Some(""),
            Some("/xdgcfg"),
            Some("/home/u"),
            &["/xdgcfg/gpy/config.toml", "/home/u/.config/gpy/config.toml"],
        ),
        (
            None,
            Some(""),
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml"],
        ),
        // A relative XDG_CONFIG_HOME is invalid and ignored.
        (
            None,
            Some("cfg"),
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml"],
        ),
        (
            None,
            None,
            Some("/home/u"),
            &["/home/u/.config/gpy/config.toml"],
        ),
        // No home at all: no candidate remains.
        (None, None, None, &[]),
        // An empty or relative HOME is unset, as in the shells' __gpy_home.
        (
            None,
            Some("/xdgcfg"),
            Some("rel/home"),
            &["/xdgcfg/gpy/config.toml"],
        ),
        (
            None,
            Some("/xdgcfg"),
            Some("/home/u"),
            &["/xdgcfg/gpy/config.toml", "/home/u/.config/gpy/config.toml"],
        ),
    ];

    /// # Panics
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn config_candidate_precedence() {
        for (custom, xdg_config, home, expected) in CANDIDATE_CASES {
            assert_eq!(
                config_candidates_for(custom, xdg_config, home, Os::Unix),
                expected.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                "candidates for (GPY_CONFIG_PATH={custom:?}, XDG_CONFIG_HOME={xdg_config:?}, HOME={home:?})"
            );
        }
    }

    /// Windows-absolute `XDG_CONFIG_HOME` is used on Windows, ignored on Unix.
    ///
    /// # Panics
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn config_candidates_honour_windows_xdg_only_under_windows_rules() {
        let xdg = Some(r"C:\Users\u\cfg");
        assert_eq!(
            config_candidates_for(None, xdg, Some("/h"), Os::Windows),
            [
                r"C:\Users\u\cfg/gpy/config.toml",
                "/h/.config/gpy/config.toml"
            ],
        );
        assert_eq!(
            config_candidates_for(None, xdg, Some("/h"), Os::Unix),
            ["/h/.config/gpy/config.toml"],
        );
    }
}

#[cfg(test)]
mod resolve_active_config_tests {
    #![allow(clippy::missing_panics_doc)]

    use super::resolve_active_config;
    use std::path::PathBuf;

    #[test]
    fn none_exist_writes_to_the_first_candidate_and_loads_nothing() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let candidates = [temp.path().join("a.toml"), temp.path().join("b.toml")];

        assert_eq!(
            resolve_active_config(&candidates),
            Some((temp.path().join("a.toml"), None))
        );
    }

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

    #[test]
    fn no_candidates_resolves_to_nothing() {
        assert_eq!(resolve_active_config::<PathBuf>(&[]), None);
    }
}
