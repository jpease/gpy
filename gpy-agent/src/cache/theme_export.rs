//! Fish theme-export cache file I/O.
//!
//! This is a distinct output from the instant-prompt cache (see
//! [`crate::cache::instant_prompt`]): it caches the Fish-format theme export
//! consumed by the shell's reload handler (doorbell + `.reload` flag), not the instant-prompt hot path.
//!
//! The agent writes this file atomically on startup and after every
//! config/theme change (before ringing the reload doorbell), so shells can source it
//! without spawning the binary on the hot path.

use super::instant_prompt::{get_gpy_cache_dir, write_atomic};
use crate::Result;
use std::path::PathBuf;

/// Return the path used to cache the Fish-format theme export.
///
/// The agent writes this file atomically on startup and after every config/theme
/// change (before ringing the reload doorbell), so shells can source it without spawning the
/// binary on the hot path.
///
/// # Errors
///
/// Returns an error if neither `XDG_CACHE_HOME` nor `HOME` is set.
pub fn get_theme_export_cache_path() -> Result<PathBuf> {
    get_gpy_cache_dir().map(|d| d.join("theme-export.fish"))
}

/// Write the Fish-format theme export to the cache file atomically.
///
/// Uses temp-file + rename so readers always see either the complete old or
/// complete new content — never a partial write.
///
/// Returns whether any export file changed; see [`write_theme_export_to_dir`].
///
/// # Errors
///
/// Returns an error if rendering or writing fails.
pub fn write_theme_export_cache(
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
) -> Result<bool> {
    let cache_dir = get_gpy_cache_dir()?;
    write_theme_export_to_dir(&cache_dir, theme_manager, config)
}

/// Inner implementation: write theme export atomically to an explicit directory.
///
/// Writes all three shells' exports on every call (`theme-export.fish`,
/// `theme-export.bash`, `theme-export.zsh`) -- Bash and Zsh's
/// `__gpy_load_theme` (bash/core/init.bash, zsh/core/init.zsh) source their
/// cache file the same way Fish already does, instead of forking `gpy-agent
/// theme export` on every shell start (#614). No call-site change needed:
/// startup and config reload already call this function, and now cover all
/// three shells for free.
///
/// A file whose bytes already match the new export is left untouched, so its
/// mtime only moves when its content does: shells compare it with the mtime
/// they sourced to spot an export rewritten while they were not told (#701).
/// Returns `true` if any of the three files was (re)written.
///
/// Separated from `write_theme_export_cache` so tests (and hermetic Criterion
/// benches, which compile as separate crates and cannot see `#[cfg(test)]`
/// helpers) can supply a temp dir without mutating env vars (which is
/// forbidden under `#![forbid(unsafe_code)]`).
///
/// # Errors
///
/// Returns an error if rendering or writing fails.
pub fn write_theme_export_to_dir(
    cache_dir: &std::path::Path,
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
) -> Result<bool> {
    std::fs::create_dir_all(cache_dir)?;

    let mut changed = false;
    for (shell, file_name) in [
        (crate::shell::Shell::Fish, "theme-export.fish"),
        (crate::shell::Shell::Bash, "theme-export.bash"),
        (crate::shell::Shell::Zsh, "theme-export.zsh"),
    ] {
        let content = theme_manager.export(shell, config);
        // Any read failure (typically: no file yet) means "write it".
        if std::fs::read(cache_dir.join(file_name))
            .is_ok_and(|current| current == content.as_bytes())
        {
            continue;
        }
        write_atomic(cache_dir, file_name, &content)?;
        changed = true;
    }

    Ok(changed)
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn test_write_theme_export_cache_creates_file() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");

        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");
        let config = crate::config::Config::default();

        write_theme_export_to_dir(&cache_dir, &theme_manager, &config)
            .expect("write theme export cache");

        let cache_path = cache_dir.join("theme-export.fish");
        assert!(cache_path.exists(), "theme-export.fish must be created");

        let content = std::fs::read_to_string(&cache_path).expect("read cache");
        assert!(
            content.contains("set -g"),
            "cache must contain Fish set assignments"
        );
    }

    #[test]
    fn test_write_theme_export_cache_is_atomic_no_temp_leftovers() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");

        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");
        let config = crate::config::Config::default();

        write_theme_export_to_dir(&cache_dir, &theme_manager, &config).expect("write");

        let leftovers: Vec<_> = std::fs::read_dir(&cache_dir)
            .expect("read dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp- files should be left behind");
    }

    /// Bash and zsh get their own cache files beside `theme-export.fish`.
    ///
    /// #614: written on every call, atomically (temp+rename, no `.tmp-`
    /// leftovers) -- Bash and Zsh `__gpy_load_theme` source these instead of
    /// forking `gpy-agent theme export` on every shell start.
    #[test]
    fn test_write_theme_export_to_dir_writes_all_three_shell_files() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");

        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");
        let config = crate::config::Config::default();

        write_theme_export_to_dir(&cache_dir, &theme_manager, &config)
            .expect("write theme export cache");

        for (file_name, needle) in [
            ("theme-export.fish", "set -g"),
            ("theme-export.bash", "export"),
            ("theme-export.zsh", "export"),
        ] {
            let cache_path = cache_dir.join(file_name);
            assert!(cache_path.exists(), "{file_name} must be created");
            let content = std::fs::read_to_string(&cache_path)
                .unwrap_or_else(|e| panic!("read {file_name}: {e}"));
            assert!(
                content.contains(needle),
                "{file_name} must contain a `{needle}` assignment, got:\n{content}"
            );
        }

        let leftovers: Vec<_> = std::fs::read_dir(&cache_dir)
            .expect("read dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no .tmp- files should be left behind for any of the three shell exports"
        );
    }

    /// #701: an identical export reports "unchanged", a different one "changed".
    ///
    /// The startup warm-up rings shells only on a change, and shells spot a
    /// rewrite by its mtime, so an identical export must also leave every file
    /// (and its mtime) alone, while a different one must land on disk.
    #[test]
    fn test_write_theme_export_to_dir_reports_whether_the_export_changed() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");
        let config = crate::config::Config::default();
        let default_theme = crate::theme::ThemeManager::builtin("default").expect("default theme");
        let text_theme = crate::theme::ThemeManager::builtin("text").expect("text theme");

        assert!(
            write_theme_export_to_dir(&cache_dir, &default_theme, &config).expect("first write"),
            "a first write (no files yet) is a change"
        );

        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_u64);
        for file_name in ["theme-export.fish", "theme-export.bash", "theme-export.zsh"] {
            std::fs::File::options()
                .write(true)
                .open(cache_dir.join(file_name))
                .and_then(|file| file.set_modified(old))
                .expect("backdate export");
        }

        assert!(
            !write_theme_export_to_dir(&cache_dir, &default_theme, &config)
                .expect("identical write"),
            "rewriting identical export bytes must report unchanged"
        );
        for file_name in ["theme-export.fish", "theme-export.bash", "theme-export.zsh"] {
            let modified = std::fs::metadata(cache_dir.join(file_name))
                .and_then(|meta| meta.modified())
                .expect("export mtime");
            assert_eq!(
                modified, old,
                "{file_name} must not be rewritten when unchanged"
            );
        }

        assert!(
            write_theme_export_to_dir(&cache_dir, &text_theme, &config).expect("different write"),
            "a different export must report changed"
        );
        let fish = std::fs::read_to_string(cache_dir.join("theme-export.fish")).expect("read fish");
        assert!(
            fish.contains("set -g __gpy_theme_name \"text\""),
            "the changed export must be on disk, got:\n{fish}"
        );
    }
}
