//! Discovery of named TOML artifacts (themes, palettes); precedence
//! User > Plugin > Builtin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a discovered artifact came from (re-exported as `ThemeSource`/`PaletteSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Built into the GPY distribution.
    Builtin,
    /// User-provided, from `~/.config/gpy/<themes|palettes>`.
    User,
    /// Provided by a discovered plugin (themes only).
    Plugin {
        /// Plugin identifier from the provider manifest.
        plugin_id: String,
    },
}

impl Source {
    /// Higher wins when the same name is discovered from several sources.
    pub(crate) const fn precedence(&self) -> u8 {
        match self {
            Self::Builtin => 0,
            Self::Plugin { .. } => 1,
            Self::User => 2,
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => f.write_str("builtin"),
            Self::User => f.write_str("user"),
            Self::Plugin { plugin_id } => write!(f, "plugin:{plugin_id}"),
        }
    }
}

/// The error for a theme or palette `name` that fails
/// [`is_safe_config_name`](super::types::is_safe_config_name).
#[must_use]
pub fn invalid_name_error(kind: &str, name: &str) -> crate::Error {
    crate::Error::config(format!(
        "invalid {kind} name '{name}': must not contain path separators, '..', or control characters"
    ))
}

/// Reject a theme or palette `name` that could escape its directory.
///
/// # Errors
/// Returns [`invalid_name_error`] unless `name` is [`is_safe_config_name`](super::types::is_safe_config_name).
pub fn ensure_safe_name(kind: &str, name: &str) -> crate::Result<()> {
    if super::types::is_safe_config_name(name) {
        Ok(())
    } else {
        Err(invalid_name_error(kind, name))
    }
}

/// A discovered artifact (re-exported as `DiscoveredTheme`/`DiscoveredPalette`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Artifact name (the file stem, or the builtin's registered name).
    pub name: String,
    /// Where it came from, for precedence and UX display.
    pub source: Source,
    /// Concrete file path when file-backed; `None` for built-ins.
    pub path: Option<PathBuf>,
}

/// `.toml` files directly in `dir` as `(stem, path)`; best-effort, a missing
/// or unreadable dir yields nothing.
pub(crate) fn toml_stems(dir: &Path) -> impl Iterator<Item = (String, PathBuf)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("toml") {
                return None;
            }
            let name = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .map(ToOwned::to_owned)?;
            Some((name, path))
        })
}

/// Merge one candidate, highest precedence winning; ties go to the candidate.
pub(crate) fn merge_candidate(merged: &mut BTreeMap<String, Discovered>, candidate: Discovered) {
    let should_replace = merged
        .get(&candidate.name)
        .is_none_or(|existing| candidate.source.precedence() >= existing.source.precedence());

    if should_replace {
        merged.insert(candidate.name.clone(), candidate);
    }
}

/// Seed `merged` with builtin names unconditionally (nothing ranks below them).
pub(crate) fn insert_builtins(
    merged: &mut BTreeMap<String, Discovered>,
    builtins: &[(&str, &str)],
) {
    for (name, _) in builtins {
        let key = (*name).to_owned();
        merged.insert(
            key.clone(),
            Discovered {
                name: key,
                source: Source::Builtin,
                path: None,
            },
        );
    }
}

/// Merge every `.toml` file in `dir` into `merged`, tagged with `source`.
pub(crate) fn insert_toml_dir(
    merged: &mut BTreeMap<String, Discovered>,
    dir: &Path,
    source: &Source,
) {
    for (name, path) in toml_stems(dir) {
        merge_candidate(
            merged,
            Discovered {
                name,
                source: source.clone(),
                path: Some(path),
            },
        );
    }
}

/// `"builtin"`/`"plugin 'id'"` for the shipped artifact `name` resolves to, or
/// `None` if absent or user-owned; imports refuse to silently shadow these.
pub(crate) fn shadowed_provider(discovered: &[Discovered], name: &str) -> Option<String> {
    discovered
        .iter()
        .find(|entry| entry.name == name)
        .and_then(|entry| match &entry.source {
            Source::Builtin => Some("builtin".to_owned()),
            Source::Plugin { plugin_id } => Some(format!("plugin '{plugin_id}'")),
            Source::User => None,
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{
        Discovered, Source, insert_builtins, insert_toml_dir, merge_candidate, toml_stems,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn write(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "# fixture\n").expect("write fixture");
        path
    }

    #[test]
    fn toml_stems_keeps_only_toml_files_and_names_them_by_stem() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        write(temp.path(), "alpha.toml");
        write(temp.path(), "beta.toml");
        write(temp.path(), "notes.txt");
        write(temp.path(), "no-extension");
        std::fs::create_dir_all(temp.path().join("nested.toml")).expect("dir named like a toml");

        let mut found: Vec<String> = toml_stems(temp.path()).map(|(name, _)| name).collect();
        found.sort();

        // A directory named `*.toml` matches; reading it fails later, per artifact.
        assert_eq!(found, ["alpha", "beta", "nested"]);
    }

    #[test]
    fn toml_stems_treats_a_missing_directory_as_empty() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let absent = temp.path().join("definitely-absent");
        assert_eq!(toml_stems(&absent).count(), 0);
    }

    #[test]
    fn user_outranks_plugin_outranks_builtin() {
        assert!(
            Source::User.precedence()
                > Source::Plugin {
                    plugin_id: String::new()
                }
                .precedence()
        );
        assert!(
            Source::Plugin {
                plugin_id: String::new()
            }
            .precedence()
                > Source::Builtin.precedence()
        );
    }

    #[test]
    fn merge_keeps_the_highest_precedence_source_regardless_of_order() {
        let plugin = Discovered {
            name: "shared".to_owned(),
            source: Source::Plugin {
                plugin_id: "acme".to_owned(),
            },
            path: Some(PathBuf::from("/plugins/acme/shared.toml")),
        };
        let user = Discovered {
            name: "shared".to_owned(),
            source: Source::User,
            path: Some(PathBuf::from("/home/user/shared.toml")),
        };

        for candidates in [
            vec![plugin.clone(), user.clone()],
            vec![user.clone(), plugin],
        ] {
            let mut merged = BTreeMap::new();
            insert_builtins(&mut merged, &[("shared", "")]);
            for candidate in candidates {
                merge_candidate(&mut merged, candidate);
            }
            assert_eq!(
                merged.get("shared"),
                Some(&user),
                "the user copy must win over plugin and builtin, in any merge order"
            );
        }
    }

    #[test]
    fn insert_toml_dir_tags_source_and_retains_paths() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let alpha = write(temp.path(), "alpha.toml");

        let mut merged = BTreeMap::new();
        insert_builtins(&mut merged, &[("builtin-only", ""), ("alpha", "")]);
        insert_toml_dir(&mut merged, temp.path(), &Source::User);

        assert_eq!(
            merged.get("alpha"),
            Some(&Discovered {
                name: "alpha".to_owned(),
                source: Source::User,
                path: Some(alpha),
            }),
            "a user file must shadow the builtin and carry its own path"
        );
        assert_eq!(
            merged
                .get("builtin-only")
                .and_then(|found| found.path.clone()),
            None,
            "a builtin with no file behind it stays path-less"
        );
    }
}
