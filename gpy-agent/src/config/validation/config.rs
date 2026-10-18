//! Cross-field validation for loaded configuration.

use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::plugin::{SegmentName, first_token_collision};

/// Validate enabled segments list.
///
/// # Errors
/// Returns an error if any segment name is invalid or two names share a shell
/// variable token (e.g. `a-b` and `a_b`).
fn validate_enabled_segments(segments: &[String]) -> Result<()> {
    let mut names = Vec::with_capacity(segments.len());
    for (i, segment) in segments.iter().enumerate() {
        names.push(SegmentName::new(segment).map_err(|_| ValidationError {
            field_path: format!("segments[{i}]"),
            kind: ValidationErrorKind::InvalidSegmentName {
                value: segment.clone(),
            },
        })?);
    }
    if let Some(collision) = first_token_collision(&names) {
        return Err(ValidationError {
            field_path: "ui.enabled_segments".to_owned(),
            kind: ValidationErrorKind::SegmentTokenCollision { collision },
        }
        .into());
    }
    Ok(())
}

/// Validate that skip paths are absolute paths.
/// `Path::is_absolute` is platform-aware; `~` is special-cased since it isn't expanded.
/// # Errors
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
/// Returns an error if any configuration value is invalid.
pub fn validate_config(config: &crate::config::Config) -> Result<()> {
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

    #[test]
    fn enabled_segments_with_colliding_tokens_are_rejected() {
        let err = validate_enabled_segments(&["k8s-ctx".to_owned(), "k8s_ctx".to_owned()])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("k8s-ctx") && err.contains("k8s_ctx"),
            "error must name both segments: {err}"
        );
        assert!(validate_enabled_segments(&["my--seg".to_owned(), "my-seg".to_owned()]).is_ok());
    }

    /// Platform-neutral cases; absolute paths are tested per-`cfg` below.
    #[test]
    fn test_validate_skip_paths() {
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative/path".to_owned()]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_skip_paths_unix_absolute_and_tilde() {
        assert!(validate_skip_paths(&["/tmp".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative/path".to_owned()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_skip_paths_windows_drive_letter_path_is_absolute() {
        assert!(validate_skip_paths(&[r"C:\Users\foo".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative\\path".to_owned()]).is_err());
    }
}
