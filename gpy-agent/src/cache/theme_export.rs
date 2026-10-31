//! Theme-export cache files, sourced by shells at startup and on reload so they
//! never spawn the binary on the hot path.

use super::instant_prompt::files::{create_private_dir, get_gpy_cache_dir, write_atomic};
use crate::Result;
use std::path::PathBuf;

/// Export file per shell; the fish entry is the one `get_theme_export_cache_path` names.
const EXPORT_FILES: [(crate::shell::Shell, &str); 3] = [
    (crate::shell::Shell::Fish, "theme-export.fish"),
    (crate::shell::Shell::Bash, "theme-export.bash"),
    (crate::shell::Shell::Zsh, "theme-export.zsh"),
];

/// Return the directory the theme exports are cached in.
///
/// # Errors
///
/// Returns an error if neither `XDG_CACHE_HOME` nor `HOME` is set.
pub fn theme_export_dir() -> Result<PathBuf> {
    get_gpy_cache_dir()
}

/// Return the path used to cache the Fish-format theme export.
///
/// # Errors
///
/// Returns an error if neither `XDG_CACHE_HOME` nor `HOME` is set.
pub fn get_theme_export_cache_path() -> Result<PathBuf> {
    theme_export_dir().map(|d| d.join(EXPORT_FILES[0].1))
}

/// Write the theme exports atomically (temp-file + rename); returns whether any file changed.
///
/// # Errors
///
/// Returns an error if rendering or writing fails.
pub fn write_theme_export_cache(
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
    palette: &crate::template::Palette,
) -> Result<bool> {
    write_theme_export_to_dir(&theme_export_dir()?, theme_manager, config, palette)
}

/// Write fish/bash/zsh theme exports atomically into `cache_dir`.
///
/// Files with identical content are left untouched so their mtime only moves
/// on change (shells compare it). Returns `true` if any file was rewritten.
/// Takes an explicit dir so tests and benches avoid mutating env vars.
///
/// # Errors
///
/// Returns an error if rendering or writing fails.
pub fn write_theme_export_to_dir(
    cache_dir: &std::path::Path,
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
    palette: &crate::template::Palette,
) -> Result<bool> {
    create_private_dir(cache_dir)?;

    let mut changed = false;
    for (shell, file_name) in EXPORT_FILES {
        let content = theme_manager.export(shell, config, palette);
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
mod tests {
    use super::*;

    /// Bash and zsh get their own atomically written cache files beside `theme-export.fish`.
    #[test]
    fn writes_all_three_shell_files_without_temp_leftovers() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");

        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");
        let config = crate::config::Config::default();

        write_theme_export_to_dir(
            &cache_dir,
            &theme_manager,
            &config,
            &crate::template::Palette::default(),
        )
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

    /// An identical export reports "unchanged" and keeps mtimes; a different one lands on disk.
    #[test]
    fn reports_whether_the_export_changed() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");
        let config = crate::config::Config::default();
        let default_theme = crate::theme::ThemeManager::builtin("default").expect("default theme");
        let text_theme = crate::theme::ThemeManager::builtin("text").expect("text theme");
        let palette = crate::template::Palette::default();

        assert!(
            write_theme_export_to_dir(&cache_dir, &default_theme, &config, &palette)
                .expect("first write"),
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
            !write_theme_export_to_dir(&cache_dir, &default_theme, &config, &palette)
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
            write_theme_export_to_dir(&cache_dir, &text_theme, &config, &palette)
                .expect("different write"),
            "a different export must report changed"
        );
        let fish = std::fs::read_to_string(cache_dir.join("theme-export.fish")).expect("read fish");
        assert!(
            fish.contains("set -g __gpy_theme_name \"text\""),
            "the changed export must be on disk, got:\n{fish}"
        );
    }

    /// The directory shells source from is owner-only, and so are the files.
    #[cfg(unix)]
    #[test]
    fn theme_export_dir_and_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache_dir = temp_dir.path().join("gpy");
        std::fs::create_dir_all(&cache_dir).expect("create dir");
        std::fs::set_permissions(&cache_dir, std::fs::Permissions::from_mode(0o755))
            .expect("chmod dir");
        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");

        write_theme_export_to_dir(
            &cache_dir,
            &theme_manager,
            &crate::config::Config::default(),
            &crate::template::Palette::default(),
        )
        .expect("write theme export cache");

        let mode = |path: &std::path::Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&cache_dir), 0o700, "cache dir must be owner-only");
        for file_name in ["theme-export.fish", "theme-export.bash", "theme-export.zsh"] {
            assert_eq!(mode(&cache_dir.join(file_name)), 0o600, "{file_name}");
        }
    }

    /// A symlinked cache dir is written through and its target is not chmod-ed.
    #[cfg(unix)]
    #[test]
    fn theme_export_follows_a_symlinked_cache_dir_without_chmod() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let real = temp_dir.path().join("real");
        let link = temp_dir.path().join("gpy");
        std::fs::create_dir_all(&real).expect("create real");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755))
            .expect("chmod real");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let theme_manager = crate::theme::ThemeManager::builtin("default").expect("theme manager");

        write_theme_export_to_dir(
            &link,
            &theme_manager,
            &crate::config::Config::default(),
            &crate::template::Palette::default(),
        )
        .expect("write through symlink");

        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink(),
            "the symlink must stay a symlink"
        );
        assert_eq!(
            std::fs::metadata(&real)
                .expect("real metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "the symlink target must not be chmod-ed"
        );
        for file_name in ["theme-export.fish", "theme-export.bash", "theme-export.zsh"] {
            assert!(real.join(file_name).exists(), "{file_name} must exist");
        }
    }
}
