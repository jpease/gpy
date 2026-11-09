//! Cross-field validation for loaded configuration.

use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::config::skip_paths;
use crate::plugin::{SegmentName, first_token_collision};

/// Parse every enabled segment into its normalized [`SegmentName`].
///
/// # Errors
/// Returns an error if any segment name is invalid or two names share a shell
/// variable token (e.g. `a-b` and `a_b`).
fn parse_enabled_segments(segments: &[String]) -> Result<Vec<SegmentName>> {
    let mut names = Vec::with_capacity(segments.len());
    for (i, segment) in segments.iter().enumerate() {
        let name = SegmentName::new(segment)
            .ok()
            .ok_or_else(|| ValidationError {
                field_path: format!("ui.enabled_segments[{i}]"),
                kind: ValidationErrorKind::InvalidSegmentName {
                    value: segment.clone(),
                },
            })?;
        names.push(name);
    }
    if let Some(collision) = first_token_collision(&names) {
        return Err(ValidationError {
            field_path: "ui.enabled_segments".to_owned(),
            kind: ValidationErrorKind::SegmentTokenCollision { collision },
        }
        .into());
    }
    Ok(names)
}

/// Validate enabled segments list.
///
/// # Errors
/// Returns an error if any segment name is invalid or two names collide.
fn validate_enabled_segments(segments: &[String]) -> Result<()> {
    parse_enabled_segments(segments).map(drop)
}

/// Validate that every skip path is absolute, `~`, or `~/…`.
///
/// # Errors
/// Returns an error naming the first entry that could never match a repository,
/// including `~user/…`, which is never expanded.
fn validate_skip_paths(paths: &[String]) -> Result<()> {
    for (i, path) in paths.iter().enumerate() {
        if !skip_paths::is_valid_entry(path) {
            return Err(ValidationError {
                field_path: format!("git.skip_paths[{i}]"),
                kind: ValidationErrorKind::InvalidSkipPath {
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

/// Canonical spelling of a language identifier; unknown names pass through.
fn canonical_language(name: &str) -> String {
    crate::language::metadata::get_canonical_name(name)
        .unwrap_or(name)
        .to_owned()
}

/// Rewrite user-spelled names into the canonical form every consumer (export,
/// shells, doctor, `config get`) compares against, then validate.
///
/// `ui.enabled_segments` entries are trimmed and lowercased via [`SegmentName`],
/// so `"Directory"` and `" git "` load as `directory` and `git`. Repeats keep
/// only their first occurrence, so `["git", "Git"]` renders git once.
///
/// `language.enabled_languages` entries and `[language.icons]` keys become the
/// canonical language name (`rs` → `rust`, `golang` → `go`); unknown names are
/// kept as written. When several icon keys name one language, the key already
/// spelled canonically wins, else the alphabetically first alias.
///
/// # Errors
/// Returns an error if any configuration value is invalid.
pub fn normalize_config(config: &mut crate::config::Config) -> Result<()> {
    let names = parse_enabled_segments(&config.ui.enabled_segments)?;
    let mut canonical: Vec<String> = Vec::with_capacity(names.len());
    for parsed in names {
        let name = String::from(parsed);
        if !canonical.contains(&name) {
            canonical.push(name);
        }
    }
    config.ui.enabled_segments = canonical;

    let mut languages: Vec<String> = Vec::with_capacity(config.language.enabled_languages.len());
    for entry in &config.language.enabled_languages {
        let name = canonical_language(entry);
        if !languages.contains(&name) {
            languages.push(name);
        }
    }
    config.language.enabled_languages = languages;

    let mut icons: Vec<_> = std::mem::take(&mut config.language.icons.icons)
        .into_iter()
        .collect();
    icons.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, icon) in icons {
        let language = canonical_language(&key);
        if key == language || !config.language.icons.icons.contains_key(&language) {
            config.language.icons.icons.insert(language, icon);
        }
    }

    validate_config(config)
}

#[cfg(test)]
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

    #[test]
    fn normalize_config_stores_canonical_names_and_then_checks_collisions() {
        let mut config = crate::config::Config::default();
        config.ui.enabled_segments = vec![
            "Directory".to_owned(),
            " git ".to_owned(),
            "My-Plugin".to_owned(),
        ];
        normalize_config(&mut config).unwrap();
        assert_eq!(
            config.ui.enabled_segments,
            vec!["directory", "git", "my-plugin"]
        );

        config.ui.enabled_segments = vec!["K8S-Ctx".to_owned(), " k8s_ctx".to_owned()];
        assert!(normalize_config(&mut config).is_err());
    }

    #[test]
    fn normalize_config_canonicalizes_language_aliases() {
        let icon = |s: &str| crate::config::types::Icon::new(s).unwrap();
        let mut config = crate::config::Config::default();
        config.language.enabled_languages = vec![
            "golang".to_owned(),
            "TS".to_owned(),
            "rs".to_owned(),
            "rust".to_owned(),
        ];
        let input = &mut config.language.icons.icons;
        input.insert("rs".to_owned(), icon("A"));
        input.insert("rust".to_owned(), icon("B"));
        input.insert("js".to_owned(), icon("C"));
        input.insert("ts".to_owned(), icon("D"));
        normalize_config(&mut config).unwrap();
        assert_eq!(config.language.enabled_languages, ["go", "node", "rust"]);
        let icons = &config.language.icons.icons;
        assert_eq!(icons.len(), 2);
        assert_eq!(icons["rust"], icon("B"));
        assert_eq!(icons["node"], icon("C"));
    }

    #[test]
    fn normalize_config_keeps_unknown_language_names() {
        let mut config = crate::config::Config::default();
        config.language.enabled_languages = vec!["cobol".to_owned()];
        let icon = crate::config::types::Icon::new("C").unwrap();
        config.language.icons.icons.insert("cobol".to_owned(), icon);
        normalize_config(&mut config).unwrap();
        assert_eq!(config.language.enabled_languages, ["cobol"]);
        assert!(config.language.icons.icons.contains_key("cobol"));
    }

    #[test]
    fn normalize_config_keeps_the_first_of_repeated_segments() {
        let mut config = crate::config::Config::default();
        config.ui.enabled_segments = vec![
            "git".to_owned(),
            "Directory".to_owned(),
            "Git".to_owned(),
            " git ".to_owned(),
            "directory".to_owned(),
            "clock".to_owned(),
        ];
        normalize_config(&mut config).unwrap();
        assert_eq!(
            config.ui.enabled_segments,
            vec!["git", "directory", "clock"]
        );
    }

    /// Platform-neutral cases; absolute paths are tested per-`cfg` below.
    #[test]
    fn test_validate_skip_paths() {
        assert!(validate_skip_paths(&["~/test".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["~".to_owned()]).is_ok());
        assert!(validate_skip_paths(&["relative/path".to_owned()]).is_err());
    }

    /// `~user/…` is never expanded, so it could never match a repository.
    #[test]
    fn test_validate_skip_paths_rejects_other_users_home() {
        let absolute = std::env::temp_dir().to_string_lossy().into_owned();
        let err = validate_skip_paths(&[absolute, "~root/x".to_owned()])
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "Configuration error: git.skip_paths[1] must be an absolute path, '~', or start with '~/': ~root/x"
        );
    }

    #[test]
    fn test_enabled_segment_error_names_full_field_path() {
        let err = validate_enabled_segments(&["git".to_owned(), "bad seg".to_owned()]).unwrap_err();
        let crate::Error::Config {
            source: Some(source),
            ..
        } = &err
        else {
            panic!("expected a config error, got {err:?}");
        };
        let validation = source.downcast_ref::<ValidationError>().unwrap();
        assert_eq!(validation.field_path, "ui.enabled_segments[1]");
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
