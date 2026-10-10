//! Cross-field validation for loaded configuration.
//!
//! This module checks relationships that individual serde fields and newtypes
//! cannot prove alone, such as enabled segment names, timing ranges, and display
//! modes that depend on the surrounding [`crate::config::Config`]. It runs after
//! parsing and before config snapshots are accepted by the loader or manager.

use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::plugin::SegmentName;

/// Validate enabled segments list.
///
/// # Errors
///
/// Returns an error if any segment name is invalid.
fn validate_enabled_segments(segments: &[String]) -> Result<()> {
    for (i, segment) in segments.iter().enumerate() {
        SegmentName::new(segment).map_err(|_| ValidationError {
            field_path: format!("segments[{i}]"),
            kind: ValidationErrorKind::InvalidSegmentName {
                value: segment.clone(),
            },
        })?;
    }
    Ok(())
}

/// Validate that skip paths are absolute paths.
///
/// Uses [`std::path::Path::is_absolute`], which is platform-aware: it
/// recognizes drive-letter-prefixed paths (`C:\Users\...`) as absolute on a
/// Windows-target build, and `/`-prefixed paths as absolute on Unix. The
/// previous string-based check (`starts_with('/')`) only ever recognized
/// Unix-style paths, so a Windows absolute path was unconditionally rejected
/// even when the binary was actually running on Windows (#597) — Windows
/// users could never successfully configure a `skip_paths` entry. The tilde
/// convention is special-cased separately since `Path::is_absolute` does not
/// expand `~` (it is just a literal path component to the OS).
///
/// # Errors
///
/// Returns an error if any path is not absolute.
fn validate_skip_paths(paths: &[String]) -> Result<()> {
    for (i, path) in paths.iter().enumerate() {
        if !path.starts_with('~') && !std::path::Path::new(path).is_absolute() {
            return Err(ValidationError {
                field_path: format!("skip_paths[{i}]"),
                kind: ValidationErrorKind::RelativeSkipPath {
                    value: path.clone(),
                },
            }
            .into());
        }
    }
    Ok(())
}

/// Validate complete configuration.
///
/// # Errors
///
/// Returns an error if any configuration value is invalid.
pub fn validate_config(config: &crate::config::Config) -> Result<()> {
    // Note: supervisor check_interval_seconds/max_restart_attempts validate
    // themselves at deserialize time via `types::SupervisorCheckInterval`/
    // `types::SupervisorMaxRestartAttempts` (#597), consistent with how every
    // other bounded field (AgentTimeout, GitTimeout, etc.) is handled — none
    // of those are re-validated here either.
    validate_skip_paths(&config.git.skip_paths)?;

    validate_enabled_segments(&config.ui.enabled_segments)?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_enabled_segments() {
        assert!(
            validate_enabled_segments(&[
                "clock".to_owned(),
                "duration".to_owned(),
                "language".to_owned(),
                "k8s-context".to_owned(),
            ])
            .is_ok()
        );
        assert!(validate_enabled_segments(&["bad segment".to_owned()]).is_err());
        assert!(validate_enabled_segments(&["clock".to_owned(), "bad@seg".to_owned()]).is_err());
    }

    /// Platform-neutral cases only.
    ///
    /// Whether a rooted path such as `/tmp` is absolute is platform-specific
    /// (`/tmp` has no drive on Windows), so the absolute-path cases live in the
    /// `cfg(unix)` / `cfg(windows)` tests below.
    #[test]
    fn test_validate_skip_paths() {
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative/path".to_owned()]).is_err());
    }

    /// Unix absolute paths and the `~` convention must keep working (#597
    /// switched the absolute-path check from a `starts_with('/')` string test
    /// to the platform-aware `Path::is_absolute`).
    #[cfg(unix)]
    #[test]
    fn test_validate_skip_paths_unix_absolute_and_tilde() {
        assert!(validate_skip_paths(&["/tmp".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative/path".to_owned()]).is_err());
    }

    /// Regression test for #597: a Windows drive-letter path like `C:\Users\foo`
    /// starts with neither `/` nor `~`, so the old string-based check rejected
    /// it unconditionally -- even when actually running on Windows, where such
    /// a path genuinely is absolute. `Path::is_absolute()` is platform-aware,
    /// so this can only be verified meaningfully in a Windows-target build.
    #[cfg(windows)]
    #[test]
    fn test_validate_skip_paths_windows_drive_letter_path_is_absolute() {
        assert!(validate_skip_paths(&[r"C:\Users\foo".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative\\path".to_owned()]).is_err());
    }
}
