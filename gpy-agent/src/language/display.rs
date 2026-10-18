//! Building [`LanguageInfo`] display entries from detected languages.
//!
//! Must not depend on `crate::agent` or `crate::cache`; both depend on this.

use crate::config::{LanguageSettings, LanguageTheme};
use crate::ipc::LanguageInfo;
use crate::language::detector::{DetectedLanguage, get_language_color_from_theme};
use crate::language::venv::PythonVenv;
use crate::theme::ThemeConfig;
use std::collections::HashMap;
use std::path::Path;

/// Test-only count of version-probe attempts; stays zero when versions are hidden.
#[cfg(test)]
static PROBE_CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reset [`PROBE_CALL_COUNT`] to zero (test-only).
#[cfg(test)]
pub(crate) fn reset_probe_call_count_for_test() {
    PROBE_CALL_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Read [`PROBE_CALL_COUNT`] (test-only).
#[cfg(test)]
pub(crate) fn probe_call_count_for_test() -> u64 {
    PROBE_CALL_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Select which detected languages to display (pure, no I/O).
///
/// The filter mode runs last: it takes from the front, so narrowing after it
/// could empty a selection meant to name one language.
#[must_use]
pub fn select_display_languages<'a>(
    detected: &'a [DetectedLanguage],
    theme_lang: &LanguageTheme,
    cfg: &LanguageSettings,
) -> Vec<&'a DetectedLanguage> {
    use std::collections::HashSet;

    // A theme allow-list overrides config.language.enabled_languages.
    let theme_languages = theme_lang
        .enabled_languages
        .as_deref()
        .filter(|list| !list.is_empty());
    let effective_enabled: Option<&[String]> = theme_languages.or_else(|| {
        (!cfg.enabled_languages.is_empty()).then_some(cfg.enabled_languages.as_slice())
    });
    let allowed_languages: Option<HashSet<String>> =
        effective_enabled.map(|list| list.iter().map(|name| name.to_lowercase()).collect());

    let threshold = cfg.confidence_threshold.get();
    let mut candidates: Vec<_> = detected
        .iter()
        .filter(|lang| lang.confidence >= threshold)
        .filter(|detected_lang| {
            let name_lower = detected_lang.name.to_lowercase();
            allowed_languages
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&name_lower))
        })
        .collect();

    // The renderer drops version-less languages, so prefer probeable ones
    // (else e.g. JSON wins `primary` and the segment vanishes) unless none are.
    let versions_required =
        cfg.show_versions && !theme_lang.show_symbol_without_version.unwrap_or(false);
    if versions_required {
        let probeable: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|detected_lang| {
                crate::language::version::has_version_detector(&detected_lang.name)
            })
            .collect();
        if !probeable.is_empty() {
            candidates = probeable;
        }
    }

    // Only the survivors warrant a version probe.
    match cfg.filter {
        crate::config::types::LanguageFilter::Primary => candidates.into_iter().take(1).collect(),
        crate::config::types::LanguageFilter::All => candidates,
        crate::config::types::LanguageFilter::Top(count) => {
            candidates.into_iter().take(count).collect()
        }
    }
}

/// Build language display information from detected languages.
///
/// # Panics
/// If the constant fallback color fails to parse.
#[must_use]
pub fn build_language_display_info(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
) -> Vec<LanguageInfo> {
    build_language_display_info_at(detected_languages, theme, language_cfg, None, None)
}

/// Like [`build_language_display_info`], resolving versions from `version_cwd`.
/// Python uses `python_venv` when the caller already resolved one, else a
/// project venv under `version_cwd`.
///
/// # Panics
/// If the constant fallback color fails to parse.
#[must_use]
pub fn build_language_display_info_at(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
    version_cwd: Option<&Path>,
    python_venv: Option<&PythonVenv>,
) -> Vec<LanguageInfo> {
    if !language_cfg.enabled {
        return Vec::new();
    }

    let displayed_languages: Vec<&DetectedLanguage> =
        select_display_languages(detected_languages, &theme.segments.language, language_cfg);

    let probed_versions: Vec<Option<String>> = if language_cfg.show_versions {
        let version_detectors: HashMap<_, _> = crate::language::version::get_version_detectors()
            .iter()
            .map(|probe| (probe.language_name().to_owned(), probe.as_ref()))
            .collect();

        // Probe concurrently (each probe has its own timeout); slots keep order.
        std::thread::scope(|scope| {
            // Spawn eagerly before joining; a lazy iterator would serialize.
            let mut probe_handles = Vec::with_capacity(displayed_languages.len());
            for detected_lang in &displayed_languages {
                let probe = version_detectors.get(&detected_lang.name).copied();
                let is_python = detected_lang.name == "python";
                let is_ruby = detected_lang.name == "ruby";
                probe_handles.push(scope.spawn(move || {
                    #[cfg(test)]
                    PROBE_CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    // The daemon never inherits an activated venv; prefer the project one.
                    if is_python && let Some(cwd) = version_cwd {
                        let discovered;
                        let venv = if python_venv.is_some() {
                            python_venv
                        } else {
                            discovered = crate::language::venv::resolve_python_venv(cwd, None);
                            discovered.as_ref()
                        };
                        if let Some(venv) = venv {
                            let version = venv.version();
                            // Without a usable `pyvenv.cfg` a project venv shows no
                            // version rather than the unrelated PATH interpreter's.
                            if version.is_some() || matches!(venv, PythonVenv::Discovered(_)) {
                                return version;
                            }
                        }
                    }
                    // Prefer `.ruby-version` when no mise config governs the dir: mise often
                    // ignores idiomatic version files for ruby and reports the global one.
                    if is_ruby
                        && let Some(cwd) = version_cwd
                        && let Some(version) =
                            crate::language::version::resolve_ruby_version_file(cwd)
                    {
                        return Some(version);
                    }
                    probe.and_then(|source| {
                        crate::language::version::detect_language_release_at(source, version_cwd)
                    })
                }));
            }

            probe_handles
                .into_iter()
                // A panicked probe degrades to "no version".
                .map(|handle| handle.join().ok().flatten())
                .collect()
        })
    } else {
        vec![None; displayed_languages.len()]
    };

    let mut languages_info: Vec<LanguageInfo> = Vec::with_capacity(displayed_languages.len());
    for (detected_lang, version) in displayed_languages.iter().zip(probed_versions) {
        let color_str =
            get_language_color_from_theme(&detected_lang.name, &theme.segments.language);
        #[expect(
            clippy::expect_used,
            reason = "constant fallback literal always parses"
        )]
        let color = crate::config::types::ColorSpec::new(&color_str).unwrap_or_else(|_| {
            crate::config::types::ColorSpec::new("#ffffff")
                .expect("Hardcoded default color must be valid")
        });

        languages_info.push(LanguageInfo {
            name: detected_lang.name.clone(),
            version,
            color,
        });
    }

    languages_info
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]
mod tests {
    use super::{
        build_language_display_info_at, probe_call_count_for_test, reset_probe_call_count_for_test,
        select_display_languages,
    };
    use crate::config::LanguageSettings;
    use crate::language::detector::DetectedLanguage;
    use crate::theme::ThemeConfig;

    fn sample_detected_languages() -> Vec<DetectedLanguage> {
        vec![
            DetectedLanguage {
                name: "alpha".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 42,
            },
            DetectedLanguage {
                name: "beta".to_owned(),
                confidence: 0.8,
                file_count: 2,
                total_bytes: 256,
            },
        ]
    }

    #[test]
    fn primary_skips_a_language_that_can_never_have_a_version() {
        // Node project whose top detection is JSON (no version detector).
        let detected = vec![
            DetectedLanguage {
                name: "json".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 84,
            },
            DetectedLanguage {
                name: "node".to_owned(),
                confidence: 0.6,
                file_count: 1,
                total_bytes: 29,
            },
        ];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            show_versions: true,
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["node"],
            "primary must name the top language that can actually show a version"
        );
    }

    #[test]
    fn primary_keeps_the_top_language_when_versions_are_not_shown() {
        // Without show_versions nothing is dropped downstream; JSON stays primary.
        let detected = vec![
            DetectedLanguage {
                name: "json".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 84,
            },
            DetectedLanguage {
                name: "node".to_owned(),
                confidence: 0.6,
                file_count: 1,
                total_bytes: 29,
            },
        ];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            show_versions: false,
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["json"]);
    }

    #[test]
    fn primary_falls_back_when_nothing_can_be_probed() {
        let detected = vec![DetectedLanguage {
            name: "json".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 84,
        }];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            show_versions: true,
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["json"]);
    }

    #[test]
    fn primary_applies_the_allowlist_before_taking_one() {
        let detected = vec![
            DetectedLanguage {
                name: "rust".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 84,
            },
            DetectedLanguage {
                name: "node".to_owned(),
                confidence: 0.6,
                file_count: 1,
                total_bytes: 29,
            },
        ];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            enabled_languages: vec!["node".to_owned()],
            show_versions: true,
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["node"],
            "an allow-list that excludes the top language must not empty the selection"
        );
    }

    #[test]
    fn select_display_languages_filters_by_enabled_allowlist() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            enabled_languages: vec!["alpha".to_owned()],
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    fn select_display_languages_respects_confidence_threshold() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            confidence_threshold: crate::config::types::ConfidenceThreshold::new(0.9)
                .expect("valid threshold"),
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["alpha"],
            "only alpha (confidence 1.0) clears a 0.9 threshold"
        );
    }

    #[test]
    fn select_display_languages_applies_primary_filter_mode() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            ..LanguageSettings::default()
        };

        let selected = select_display_languages(&detected, &theme.segments.language, &language_cfg);
        let names: Vec<&str> = selected.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    #[serial_test::serial(probe_call_count)]
    fn probe_skipped_when_show_versions_false() {
        reset_probe_call_count_for_test();
        let detected = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 42,
        }];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: false,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info_at(&detected, &theme, &language_cfg, None, None);

        assert_eq!(
            info.len(),
            1,
            "the language is still displayed, just without a version probe"
        );
        assert_eq!(
            probe_call_count_for_test(),
            0,
            "show_versions=false must attempt zero probes"
        );
    }

    // Serialized: shares the global `PROBE_CALL_COUNT` with the test above.
    #[test]
    #[serial_test::serial(probe_call_count)]
    fn probe_runs_when_show_versions_true() {
        reset_probe_call_count_for_test();
        let detected = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 42,
        }];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: true,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info_at(&detected, &theme, &language_cfg, None, None);

        assert_eq!(info.len(), 1);
        assert!(
            probe_call_count_for_test() > 0,
            "show_versions=true must attempt at least one probe, unlike the false case above"
        );
    }
}
