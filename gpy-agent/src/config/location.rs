//! Which `config.toml` is active: candidate order and resolution.

use std::path::{Path, PathBuf};

/// Configuration file locations in priority order, for an explicit environment.
///
/// Pure so precedence is testable without `set_var` (forbidden by `unsafe_code`).
/// Empty `GPY_CONFIG_PATH` is dropped (else `config set` writes to `""`);
/// `XDG_CONFIG_HOME` goes through [`crate::paths::xdg_path`]. No `~/.gpy.toml` or project-local file: the
/// daemon serves every directory, so CLI and prompt must agree on one file.
#[must_use]
pub fn config_candidates_for(
    custom_path: Option<&Path>,
    xdg_config: Option<&Path>,
    home: Option<&Path>,
    os: crate::paths::Os,
) -> Vec<PathBuf> {
    [
        custom_path
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf),
        crate::paths::xdg_path(xdg_config, os).map(|xdg| xdg.join("gpy").join("config.toml")),
        crate::paths::xdg_path(home, os)
            .map(|root| root.join(".config").join("gpy").join("config.toml")),
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
pub fn get_config_paths() -> Vec<PathBuf> {
    let custom_path = crate::env_config::CONFIG_PATH
        .path()
        .map(|value| crate::paths::absolutize(&value));
    let home = crate::paths::home_path();
    let xdg_config = crate::paths::xdg_config_home();

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
pub fn resolve_active_config<P: AsRef<Path>>(
    candidates: &[P],
) -> Option<(PathBuf, Option<PathBuf>)> {
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
    use std::path::{Path, PathBuf};

    fn candidates_for(
        custom: Option<&str>,
        xdg_config: Option<&str>,
        home: Option<&str>,
        os: Os,
    ) -> Vec<PathBuf> {
        config_candidates_for(
            custom.map(Path::new),
            xdg_config.map(Path::new),
            home.map(Path::new),
            os,
        )
    }

    fn paths(expected: &[&str]) -> Vec<PathBuf> {
        expected.iter().map(PathBuf::from).collect()
    }

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
                candidates_for(custom, xdg_config, home, Os::Unix),
                paths(expected),
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
            candidates_for(None, xdg, Some("/h"), Os::Windows),
            paths(&[
                r"C:\Users\u\cfg/gpy/config.toml",
                "/h/.config/gpy/config.toml"
            ]),
        );
        assert_eq!(
            candidates_for(None, xdg, Some("/h"), Os::Unix),
            paths(&["/h/.config/gpy/config.toml"]),
        );
    }
}

#[cfg(test)]
mod resolve_active_config_tests {

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
