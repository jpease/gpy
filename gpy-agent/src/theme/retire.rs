//! Retire user copies of builtin themes that carry no edits.
//!
//! A `<themes dir>/<name>.toml` byte-identical to a released version of the
//! builtin `<name>` up to v0.7.1 (which is the same as 0.8.0) holds no user
//! intent but shadows every later fix to the builtin. `gpy setup` moves such
//! a copy aside to `<name>.toml.bak` so the builtin takes over. Any other file
//! is left alone.

use super::manager::BUILTIN_THEMES;
use super::released_hashes::RELEASED_THEME_HASHES;
use super::user_copy::move_aside;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// A file in the themes directory named after a builtin theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCopy {
    /// The builtin it shadows.
    pub name: &'static str,
    /// The file, `<themes dir>/<name>.toml`.
    pub path: PathBuf,
    /// What retirement did with it.
    pub outcome: Outcome,
}

/// What [`retire_unmodified_copies`] did with one [`UserCopy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Matched a released version and was moved to this backup path.
    Retired(PathBuf),
    /// Differs from every released version, so it is the user's: kept.
    Modified,
    /// A symlink: never followed or moved.
    Symlink,
    /// Could not be read or moved; the file is unchanged.
    Failed(String),
}

/// Whether `bytes` are exactly a released version of builtin `name`.
#[must_use]
pub fn matches_release(name: &str, bytes: &[u8]) -> bool {
    let digest = format!("{:x}", Sha256::digest(bytes));
    RELEASED_THEME_HASHES
        .iter()
        .any(|(released, hash)| *released == name && *hash == digest)
}

/// Move every builtin-named theme file in `themes_dir` that matches a released
/// version of its builtin to `<name>.toml.bak`, and report each such file.
/// Builtins without a file there are not reported; non-builtin names are never
/// looked at. Errors are reported per file, never raised.
#[must_use]
pub fn retire_unmodified_copies(themes_dir: &Path) -> Vec<UserCopy> {
    BUILTIN_THEMES
        .iter()
        .filter_map(|(name, _)| {
            let path = themes_dir.join(format!("{name}.toml"));
            let outcome = retire(name, &path)?;
            Some(UserCopy {
                name,
                path,
                outcome,
            })
        })
        .collect()
}

/// Retire `path` when it matches a release of `name`; `None` when there is no
/// regular file or symlink there.
fn retire(name: &str, path: &Path) -> Option<Outcome> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return None,
        Err(err) => return Some(Outcome::Failed(err.to_string())),
    };
    if metadata.file_type().is_symlink() {
        return Some(Outcome::Symlink);
    }
    if !metadata.is_file() {
        return None;
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return Some(Outcome::Failed(err.to_string())),
    };
    if !matches_release(name, &bytes) {
        return Some(Outcome::Modified);
    }
    Some(match move_aside(path) {
        Ok(backup) => Outcome::Retired(backup),
        Err(err) => Outcome::Failed(err.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::{Outcome, matches_release, retire_unmodified_copies};
    use std::fs;

    /// `text.toml` as v0.1.0 through v0.6.0 shipped it; not the current builtin.
    const TEXT_V0_6_0: &str = include_str!("../../tests/fixtures/released_themes/text-v0.6.0.toml");
    const STARSHIP_V0_7_1: &str =
        include_str!("../../tests/fixtures/released_themes/starship-v0.7.1.toml");

    #[test]
    fn an_old_release_matches_but_an_edited_copy_does_not() {
        let current = super::super::user_copy::builtin_content("text").expect("text is a builtin");
        assert_ne!(TEXT_V0_6_0, current, "fixture must be an older release");

        assert!(matches_release("text", TEXT_V0_6_0.as_bytes()));
        assert!(!matches_release("default", TEXT_V0_6_0.as_bytes()));
        assert!(!matches_release(
            "text",
            format!("{TEXT_V0_6_0}\n").as_bytes()
        ));
        assert!(!matches_release(
            "text",
            TEXT_V0_6_0.replace('\n', "\r\n").as_bytes()
        ));
    }

    #[test]
    fn retires_released_copies_and_keeps_everything_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let themes = dir.path();
        let starship = super::super::user_copy::builtin_content("starship").expect("builtin");
        let default = super::super::user_copy::builtin_content("default").expect("builtin");
        fs::write(themes.join("text.toml"), TEXT_V0_6_0).expect("seed text");
        fs::write(themes.join("starship.toml"), STARSHIP_V0_7_1).expect("seed starship");
        let edited = default.replacen('\n', " \n", 1);
        fs::write(themes.join("default.toml"), &edited).expect("seed default");
        fs::write(themes.join("mine.toml"), starship).expect("seed non-builtin");

        let report = retire_unmodified_copies(themes);

        let outcome = |name: &str| {
            report
                .iter()
                .find(|copy| copy.name == name)
                .map(|copy| copy.outcome.clone())
        };
        assert_eq!(
            outcome("text"),
            Some(Outcome::Retired(themes.join("text.toml.bak")))
        );
        assert_eq!(
            outcome("starship"),
            Some(Outcome::Retired(themes.join("starship.toml.bak")))
        );
        assert_eq!(outcome("default"), Some(Outcome::Modified));
        assert_eq!(
            report.len(),
            3,
            "only builtin names are reported: {report:?}"
        );
        assert!(!themes.join("text.toml").exists());
        assert_eq!(
            fs::read_to_string(themes.join("text.toml.bak")).expect("backup"),
            TEXT_V0_6_0
        );
        assert_eq!(
            fs::read_to_string(themes.join("default.toml")).expect("kept"),
            edited
        );
        assert!(
            themes.join("mine.toml").is_file(),
            "non-builtin names are never touched"
        );

        let again = retire_unmodified_copies(themes);
        assert_eq!(
            again.iter().map(|copy| &copy.outcome).collect::<Vec<_>>(),
            vec![&Outcome::Modified],
            "a second run only sees the kept copy"
        );
        assert!(themes.join("text.toml.bak").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_copy_is_reported_and_left_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("elsewhere.toml");
        fs::write(&target, TEXT_V0_6_0).expect("seed target");
        let themes = dir.path().join("themes");
        fs::create_dir_all(&themes).expect("themes dir");
        std::os::unix::fs::symlink(&target, themes.join("text.toml")).expect("symlink");

        let report = retire_unmodified_copies(&themes);

        assert_eq!(report.len(), 1);
        assert_eq!(
            report.first().map(|copy| &copy.outcome),
            Some(&Outcome::Symlink)
        );
        assert!(themes.join("text.toml").is_symlink());
        assert!(!themes.join("text.toml.bak").exists());
        assert!(target.is_file());
    }
}
