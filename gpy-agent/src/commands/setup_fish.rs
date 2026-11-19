//! Fish-specific wiring for `gpy setup`: the `fish_prompt` link, the loader
//! an older layout left in `conf.d`, and the old directories to report.
//!
//! Releases before 0.9 kept the Fish tree in `~/.config/fish/gpy` and the Bash,
//! Zsh and bundled-plugin trees in `~/.config/gpy`; they autoloaded Fish through
//! `~/.config/fish/conf.d/gpy_init.fish`.

use crate::commands::output;
use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// The loader file name, both in the old `conf.d` and in the current tree.
const LOADER: &str = "gpy_init.fish";

/// Wrap an I/O failure on `path` with what was being done to it.
fn io_error(action: &str, path: &Path) -> impl Fn(std::io::Error) -> Error {
    let context = format!("{action} {}", path.display());
    move |err| Error::io_context(&context, &err)
}

/// Directories a release before 0.9 installed that `gpy setup` no longer
/// uses, minus anything inside the current data root.
pub(super) fn legacy_trees(config_home: &Path, data_root: &Path) -> Vec<PathBuf> {
    let config_gpy = config_home.join("gpy");
    [
        config_home.join("fish").join("gpy"),
        config_gpy.join("bash"),
        config_gpy.join("zsh"),
        config_gpy.join("bundled-plugins"),
    ]
    .into_iter()
    .filter(|dir| !dir.starts_with(data_root) && !data_root.starts_with(dir))
    .collect()
}

/// The [`legacy_trees`] that exist on disk.
pub(super) fn leftover_trees(config_home: &Path, data_root: &Path) -> Vec<PathBuf> {
    legacy_trees(config_home, data_root)
        .into_iter()
        .filter(|dir| dir.symlink_metadata().is_ok())
        .collect()
}

/// Tell the user which old directories are left and how to delete them.
pub(super) fn report_leftovers(leftovers: &[PathBuf]) {
    if leftovers.is_empty() {
        return;
    }
    output::blank();
    output::plain(
        "These directories belong to an older GPY layout and are no longer used. \
         After restarting your shells, remove them:",
    );
    for dir in leftovers {
        output::plain(format_args!("  rm -rf {}", super::utils::shell_quote(dir)));
    }
}

/// Move `path` to `<path>.backup.<stamp>` and return the new name.
fn move_aside(path: &Path) -> Result<PathBuf> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_default(|elapsed| elapsed.as_secs());
    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".backup.{}",
        super::backup_stamp::backup_stamp(now)
    ));
    let backup = PathBuf::from(name);
    fs::rename(path, &backup).map_err(io_error("moving", path))?;
    Ok(backup)
}

/// Retire the autoloaded `conf.d/gpy_init.fish` of the old layout. Fish runs
/// `conf.d` before `config.fish`, so it would keep loading the old tree next
/// to the new rc block, and stop GPY once that tree is deleted. A link to a
/// `gpy_init.fish` is GPY's and is removed; anything else is moved aside.
pub(super) fn retire_fish_loader(config_home: &Path) -> Result<()> {
    let loader = config_home.join("fish").join("conf.d").join(LOADER);
    if loader.symlink_metadata().is_err() {
        return Ok(());
    }
    let gpy_link = fs::read_link(&loader).is_ok_and(|target| {
        target
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new(LOADER))
    });
    if gpy_link {
        fs::remove_file(&loader).map_err(io_error("removing", &loader))?;
        output::plain(format_args!(
            "Removed the old Fish loader {}",
            loader.display()
        ));
    } else {
        let backup = move_aside(&loader)?;
        output::plain(format_args!(
            "Moved the old Fish loader {} to {}",
            loader.display(),
            backup.display()
        ));
    }
    Ok(())
}

/// Whether a symlink to `target` is GPY's own `fish_prompt` link: a
/// `functions/fish_prompt.fish` inside a `gpy` directory (the pre-0.9
/// `~/.config/fish/gpy` tree) or inside a `gpy/fish` tree (any data root).
pub(super) fn is_gpy_prompt_link(target: &Path) -> bool {
    let names: Vec<String> = target
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let parts: Vec<&str> = names.iter().map(String::as_str).collect();
    matches!(
        parts.as_slice(),
        [.., "gpy", "functions", "fish_prompt.fish"]
            | [.., "gpy", "fish", "functions", "fish_prompt.fish"]
    )
}

/// Make `target` a symlink to `source`. GPY's own link, and a copy of the
/// shipped prompt, are replaced; anything else (including an edited or older
/// copy of GPY's prompt) is moved aside first.
#[cfg(unix)]
pub(super) fn link_fish_prompt(target: &Path, source: &Path) -> Result<()> {
    if fs::read_link(target).is_ok_and(|current| current == source) {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(io_error("creating", parent))?;
    }
    if let Ok(meta) = target.symlink_metadata() {
        if replaceable_without_backup(target, &meta, source) {
            fs::remove_file(target).map_err(io_error("removing", target))?;
        } else {
            let backup = move_aside(target)?;
            output::plain(format_args!(
                "Moved your existing fish_prompt to {}",
                backup.display()
            ));
        }
    }
    std::os::unix::fs::symlink(source, target).map_err(io_error("linking", target))?;
    Ok(())
}

/// Whether `target` holds nothing of the user's: a link into a GPY tree, or a
/// regular file with exactly the shipped prompt's bytes.
#[cfg(unix)]
fn replaceable_without_backup(target: &Path, meta: &fs::Metadata, source: &Path) -> bool {
    if meta.file_type().is_symlink() {
        return fs::read_link(target).is_ok_and(|link| is_gpy_prompt_link(&link));
    }
    meta.is_file()
        && matches!(
            (fs::read(target), fs::read(source)),
            (Ok(have), Ok(shipped)) if have == shipped
        )
}

/// A regular-file copy of GPY's own prompt (the rule `gpy uninstall` removes
/// a copy by): the `gpy prompt fish` call of the current prompt, or both
/// segment-dispatch lines every earlier prompt shipped with.
#[cfg(unix)]
pub(super) fn is_gpy_prompt_copy(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| {
        let has = |line: &str| text.lines().any(|l| l.trim_ascii() == line);
        has("set -l args prompt fish --status $last_status --pid $fish_pid --shell-version $FISH_VERSION")
            || (has("for segment in $__enabled_segments")
                && has("if functions -q segment_{$segment}_render"))
    })
}

#[cfg(not(unix))]
pub(super) fn link_fish_prompt(_target: &Path, _source: &Path) -> Result<()> {
    Err(Error::invalid("shell integration needs a Unix shell"))
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn prompt_fixture(dir: &Path) -> (PathBuf, PathBuf) {
        let target = dir.join("fish/functions/fish_prompt.fish");
        let source = dir.join("gpy/functions/fish_prompt.fish");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "function fish_prompt; end\n").unwrap();
        (target, source)
    }

    fn prompt_backups(target: &Path) -> Vec<PathBuf> {
        fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fish_prompt.fish.backup")
            })
            .collect()
    }

    #[test]
    fn link_fish_prompt_backup_uses_timestamped_name() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let original = "function fish_prompt; echo mine; end\n";
        fs::write(&target, original).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        let name = backups
            .first()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let suffix = name.split(".backup.").nth(1).unwrap();
        assert_eq!(suffix.len(), 15);
        for (i, c) in suffix.chars().enumerate() {
            if i == 8 {
                assert_eq!(c, '_');
            } else {
                assert!(c.is_ascii_digit(), "{suffix}");
            }
        }
        assert_eq!(
            fs::read_to_string(backups.first().unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn link_fish_prompt_moves_foreign_symlink_aside() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let dotfiles = dir.path().join("dotfiles/fish_prompt.fish");
        fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
        fs::write(&dotfiles, "function fish_prompt; end\n").unwrap();
        std::os::unix::fs::symlink(&dotfiles, &target).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_link(backups.first().unwrap()).unwrap(), dotfiles);
        assert!(dotfiles.exists());
    }

    #[test]
    fn link_fish_prompt_backs_up_prompt_that_only_mentions_gpy() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let original = "# Tried GPY once; kept my own. See __enabled_segments.\nfunction fish_prompt\n    printf \"mine> \"\nend\n";
        fs::write(&target, original).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups.first().unwrap()).unwrap(),
            original.as_bytes()
        );
    }

    #[test]
    fn link_fish_prompt_replaces_an_unedited_copy_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        fs::copy(&source, &target).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        assert_eq!(prompt_backups(&target), Vec::<PathBuf>::new());
    }

    #[test]
    fn link_fish_prompt_backs_up_an_edited_copy_of_gpys_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let edited = "function fish_prompt\n    echo mine\n    for segment in $__enabled_segments\n        if functions -q segment_{$segment}_render\n            segment_{$segment}_render\n        end\n    end\nend\n";
        fs::write(&target, edited).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups.first().unwrap()).unwrap(),
            edited.as_bytes()
        );
    }

    #[test]
    fn link_fish_prompt_replaces_the_legacy_gpy_link_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let legacy = dir.path().join("fish/gpy/functions/fish_prompt.fish");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "function fish_prompt; end\n").unwrap();
        std::os::unix::fs::symlink(&legacy, &target).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        assert_eq!(prompt_backups(&target), Vec::<PathBuf>::new());
    }

    #[test]
    fn gpy_prompt_links_are_recognised_by_their_tree() {
        for gpy in [
            "/h/.config/fish/gpy/functions/fish_prompt.fish",
            "/h/.local/share/gpy/fish/functions/fish_prompt.fish",
            "gpy/fish/functions/fish_prompt.fish",
        ] {
            assert!(is_gpy_prompt_link(Path::new(gpy)), "{gpy}");
        }
        for other in [
            "/h/dotfiles/fish_prompt.fish",
            "/h/.config/fish/functions/fish_prompt.fish",
            "/h/gpy/prompts/fish_prompt.fish",
        ] {
            assert!(!is_gpy_prompt_link(Path::new(other)), "{other}");
        }
    }

    #[test]
    fn retire_fish_loader_moves_a_regular_loader_aside_and_removes_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let conf_d = dir.path().join("fish/conf.d");
        fs::create_dir_all(&conf_d).unwrap();
        let loader = conf_d.join(LOADER);

        fs::write(&loader, "source old\n").unwrap();
        retire_fish_loader(dir.path()).unwrap();
        assert!(!loader.exists());
        let moved: Vec<_> = fs::read_dir(&conf_d)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert!(
            moved
                .iter()
                .all(|name| name.starts_with("gpy_init.fish.backup.")),
            "{moved:?}"
        );

        let old_tree = dir.path().join("fish/gpy/conf.d");
        fs::create_dir_all(&old_tree).unwrap();
        fs::write(old_tree.join(LOADER), "source old\n").unwrap();
        std::os::unix::fs::symlink(old_tree.join(LOADER), &loader).unwrap();
        retire_fish_loader(dir.path()).unwrap();
        assert!(loader.symlink_metadata().is_err());
        assert!(old_tree.join(LOADER).exists());
    }

    #[test]
    fn leftover_trees_lists_only_existing_old_directories_outside_the_data_root() {
        let dir = tempfile::tempdir().unwrap();
        let config_home = dir.path().join("config");
        let data_root = dir.path().join("data/gpy");
        fs::create_dir_all(config_home.join("fish/gpy")).unwrap();
        fs::create_dir_all(config_home.join("gpy/zsh")).unwrap();
        assert_eq!(
            leftover_trees(&config_home, &data_root),
            vec![config_home.join("fish/gpy"), config_home.join("gpy/zsh")]
        );
        // A data root that is the old config directory owns those trees.
        assert_eq!(
            leftover_trees(&config_home, &config_home.join("gpy")),
            vec![config_home.join("fish/gpy")]
        );
    }
}
