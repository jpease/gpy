//! One discovery implementation for the named TOML artifacts GPY resolves by
//! name: themes and palettes (#588).
//!
//! Both were the same code under different names — scan a directory, keep the
//! `.toml` files, take each file stem as the artifact's name, tag it with where
//! it came from, and merge it into a map where the highest-precedence source
//! for a given name wins. `theme::manager` and `palette::manager` now call
//! these functions instead of each keeping a copy.
//!
//! Precedence is unchanged from what each manager implemented separately:
//! **User > Plugin > Builtin**.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a discovered artifact came from, for precedence and UX display.
///
/// Re-exported as `theme::ThemeSource` and `palette::PaletteSource`; the two
/// were structurally identical enums with identical precedence, so they are now
/// one type under those names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Built into the GPY distribution.
    Builtin,
    /// User-provided, from `~/.config/gpy/<themes|palettes>`.
    User,
    /// Provided by a discovered plugin.
    ///
    /// Palettes never construct this variant: plugin manifests carry no
    /// `palettes` field yet, so plugin palette discovery stays deferred to a
    /// later sub-project. The variant exists for themes, which do wire it up,
    /// and for palettes' forward compatibility.
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

/// A discovered artifact, exposed to CLI/runtime discovery.
///
/// Re-exported as `theme::DiscoveredTheme` and `palette::DiscoveredPalette`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Artifact name (the file stem, or the builtin's registered name).
    pub name: String,
    /// Where it came from, for precedence and UX display.
    pub source: Source,
    /// Concrete file path when file-backed; `None` for built-ins.
    pub path: Option<PathBuf>,
}

/// Every `.toml` file directly in `dir`, as `(file stem, full path)` pairs.
///
/// `dir` not existing, or being unreadable, or not being a directory at all,
/// yields an empty iterator rather than an error: discovery is best-effort by
/// design — a missing user themes directory means "no user themes", not a
/// failure.
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

/// Merge one candidate into `merged`, highest-precedence source winning.
///
/// Ties go to the candidate, so a later scan of an equally-ranked source
/// replaces an earlier one — the behavior both managers had.
pub(crate) fn merge_candidate(merged: &mut BTreeMap<String, Discovered>, candidate: Discovered) {
    let should_replace = merged
        .get(&candidate.name)
        .is_none_or(|existing| candidate.source.precedence() >= existing.source.precedence());

    if should_replace {
        merged.insert(candidate.name.clone(), candidate);
    }
}

/// Seed `merged` with a builtin table's names (`name -> embedded TOML`).
///
/// Builtins are inserted unconditionally rather than merged: nothing can
/// outrank them yet, and they are the base every other source shadows.
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

/// Describe the builtin or plugin artifact that `name` resolves to in
/// `discovered`, e.g. `"builtin"` or `"plugin 'acme'"`.
///
/// Returns `None` when nothing named `name` exists or the winning entry is a
/// user file. Imports use this to refuse silently shadowing a shipped
/// artifact with a new user file (#691).
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

        // A *directory* named `nested.toml` still matches the extension check,
        // exactly as both managers' original loops did; reading it later is
        // what fails, and that failure is already handled per-artifact.
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

    /// The precedence contract both managers depend on, exercised once through
    /// the shared merge: a user file shadows a plugin file, which shadows a
    /// builtin, whatever order they are merged in.
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

    /// A builtin with no file-backed override keeps `path: None`, and a
    /// discovered file keeps the path it was found at — the expression
    /// `ThemeManager::resolve_path` runs over this map.
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
