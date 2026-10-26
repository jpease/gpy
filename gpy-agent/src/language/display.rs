//! Building [`LanguageInfo`] display entries from detected languages.
//!
//! Must not depend on `crate::agent` or `crate::cache`; both depend on this.

use crate::config::{LanguageSettings, LanguageTheme};
use crate::ipc::LanguageInfo;
use crate::language::detector::{DetectedLanguage, get_language_color_from_theme};
use crate::language::venv::PythonVenv;
use crate::language::version::{ColdVersion, ProbePolicy, ReleaseSource};
use crate::theme::ThemeConfig;
use std::collections::HashMap;
use std::path::Path;

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
/// project venv under `version_cwd`. Cold versions are probed, each bounded by
/// its own timeout.
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
    display_info(
        detected_languages,
        theme,
        language_cfg,
        version_cwd,
        python_venv,
        Versions::Probe,
        crate::language::version::get_version_detectors(),
    )
    .unwrap_or_default()
}

/// [`build_language_display_info_at`] from cached versions only: never runs a
/// probe, and is `None` when any displayed language's version is cold.
///
/// # Panics
/// If the constant fallback color fails to parse.
#[must_use]
pub fn cached_language_display_info_at(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
    version_cwd: Option<&Path>,
    python_venv: Option<&PythonVenv>,
) -> Option<Vec<LanguageInfo>> {
    display_info(
        detected_languages,
        theme,
        language_cfg,
        version_cwd,
        python_venv,
        Versions::Cached,
        crate::language::version::get_version_detectors(),
    )
    .ok()
}

/// [`cached_language_display_info_at`] once a probe job for these languages
/// has finished: a version still cold shows as none instead of failing the
/// render. The job cached every result, success or failure, so a cold one
/// means the entry is already gone again (expired, evicted, or a wall-clock
/// step back made it read as stale).
///
/// # Panics
/// If the constant fallback color fails to parse.
#[must_use]
pub fn settled_language_display_info_at(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
    version_cwd: Option<&Path>,
    python_venv: Option<&PythonVenv>,
) -> Vec<LanguageInfo> {
    display_info(
        detected_languages,
        theme,
        language_cfg,
        version_cwd,
        python_venv,
        Versions::Settled,
        crate::language::version::get_version_detectors(),
    )
    .unwrap_or_default()
}

/// Where [`display_info`] gets versions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Versions {
    /// Probe cold versions.
    Probe,
    /// Cache only; a cold version fails with [`ColdVersion`].
    Cached,
    /// Cache only; a cold version shows as none.
    Settled,
}

/// Display entries with versions from `versions`, probing through `sources`;
/// [`ColdVersion`] only under [`Versions::Cached`].
fn display_info(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
    version_cwd: Option<&Path>,
    python_venv: Option<&PythonVenv>,
    versions: Versions,
    sources: &[Box<dyn ReleaseSource + Sync + Send>],
) -> Result<Vec<LanguageInfo>, ColdVersion> {
    if !language_cfg.enabled {
        return Ok(Vec::new());
    }

    let displayed_languages: Vec<&DetectedLanguage> =
        select_display_languages(detected_languages, &theme.segments.language, language_cfg);

    let policy = if versions == Versions::Probe {
        ProbePolicy::Spawn
    } else {
        ProbePolicy::CachedOnly
    };
    let version_slots: Vec<Option<String>> = if language_cfg.show_versions {
        let version_detectors: HashMap<_, _> = sources
            .iter()
            .map(|probe| (probe.language_name().to_owned(), probe.as_ref()))
            .collect();
        let lookup = |detected_lang: &DetectedLanguage| {
            language_version(
                detected_lang,
                version_detectors.get(&detected_lang.name).copied(),
                version_cwd,
                python_venv,
                policy,
            )
        };

        match versions {
            // Cache reads only: nothing to overlap.
            Versions::Cached => displayed_languages
                .iter()
                .map(|detected_lang| lookup(detected_lang))
                .collect::<Result<Vec<_>, _>>()?,
            Versions::Settled => displayed_languages
                .iter()
                .map(|detected_lang| lookup(detected_lang).unwrap_or(None))
                .collect(),
            // Probe concurrently (each probe has its own timeout); slots keep order.
            Versions::Probe => std::thread::scope(|scope| {
                // Spawn eagerly before joining; a lazy iterator would serialize.
                let mut probe_handles = Vec::with_capacity(displayed_languages.len());
                for detected_lang in &displayed_languages {
                    let lookup_ref = &lookup;
                    probe_handles
                        .push(scope.spawn(move || lookup_ref(detected_lang).ok().flatten()));
                }

                probe_handles
                    .into_iter()
                    // A panicked probe degrades to "no version".
                    .map(|handle| handle.join().ok().flatten())
                    .collect()
            }),
        }
    } else {
        vec![None; displayed_languages.len()]
    };

    let mut languages_info: Vec<LanguageInfo> = Vec::with_capacity(displayed_languages.len());
    for (detected_lang, version) in displayed_languages.iter().zip(version_slots) {
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

    Ok(languages_info)
}

/// One displayed language's version under `policy`; [`ColdVersion`] when only
/// a probe `policy` forbids could answer.
fn language_version(
    detected_lang: &DetectedLanguage,
    probe: Option<&(dyn ReleaseSource + Sync + Send)>,
    version_cwd: Option<&Path>,
    python_venv: Option<&PythonVenv>,
    policy: ProbePolicy,
) -> Result<Option<String>, ColdVersion> {
    // The daemon never inherits an activated venv; prefer the project one.
    if detected_lang.name == "python"
        && let Some(cwd) = version_cwd
    {
        let discovered;
        let candidate = if python_venv.is_some() {
            python_venv
        } else {
            discovered = crate::language::venv::resolve_python_venv(cwd, None);
            discovered.as_ref()
        };
        if let Some(venv) = candidate {
            let version = venv.lookup_version(policy)?;
            // Without a usable `pyvenv.cfg` a project venv shows no
            // version rather than the unrelated PATH interpreter's.
            if version.is_some() || matches!(venv, PythonVenv::Discovered(_)) {
                return Ok(version);
            }
        }
    }
    // Prefer `.ruby-version` when no mise config governs the dir: mise often
    // ignores idiomatic version files for ruby and reports the global one.
    if detected_lang.name == "ruby"
        && let Some(cwd) = version_cwd
        && let Some(version) = crate::language::version::resolve_ruby_version_file(cwd)
    {
        return Ok(Some(version));
    }
    probe.map_or(Ok(None), |source| {
        crate::language::version::lookup_language_release_at(source, version_cwd, policy)
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]
mod tests {
    use super::{Versions, display_info, select_display_languages};
    use crate::config::LanguageSettings;
    use crate::language::detector::DetectedLanguage;
    use crate::language::version::ReleaseSource;
    use crate::theme::ThemeConfig;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `rust` release source counting how often its version command is read.
    struct CountingSource(Arc<AtomicUsize>);

    impl ReleaseSource for CountingSource {
        fn language_name(&self) -> &'static str {
            "rust"
        }

        fn version_command(&self) -> &[&str] {
            self.0.fetch_add(1, Ordering::SeqCst);
            &["gpy-test-no-such-version-command"]
        }

        fn parse_version(&self, _output: &str) -> Option<String> {
            None
        }
    }

    /// Version-command reads while displaying one `rust` language with versions
    /// shown or hidden, probing from a fresh directory so no cached version answers.
    fn probes_for(show_versions: bool) -> usize {
        let probes = Arc::new(AtomicUsize::new(0));
        let sources: Vec<Box<dyn ReleaseSource + Sync + Send>> =
            vec![Box::new(CountingSource(Arc::clone(&probes)))];
        let detected = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 42,
        }];
        let language_cfg = LanguageSettings {
            show_versions,
            ..LanguageSettings::default()
        };
        let cwd = tempfile::TempDir::new().expect("temp dir");

        let info = display_info(
            &detected,
            &ThemeConfig::default(),
            &language_cfg,
            Some(cwd.path()),
            None,
            Versions::Probe,
            &sources,
        )
        .expect("probing never reports a cold version");

        assert_eq!(
            info.len(),
            1,
            "the language is displayed with or without a version"
        );
        probes.load(Ordering::SeqCst)
    }

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

    /// After a probe job, a version whose cache entry is already gone again
    /// renders without a version instead of failing the request (#951).
    #[test]
    fn settled_read_shows_a_cold_version_as_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let detected = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 84,
        }];
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: true,
            ..LanguageSettings::default()
        };

        assert!(
            super::cached_language_display_info_at(
                &detected,
                &theme,
                &language_cfg,
                Some(dir.path()),
                None
            )
            .is_none(),
            "never probed here, so the version is cold"
        );
        let settled = super::settled_language_display_info_at(
            &detected,
            &theme,
            &language_cfg,
            Some(dir.path()),
            None,
        );
        let shown: Vec<(&str, Option<&str>)> = settled
            .iter()
            .map(|lang| (lang.name.as_str(), lang.version.as_deref()))
            .collect();
        assert_eq!(shown, [("rust", None)]);
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
    fn probe_skipped_when_show_versions_false() {
        assert_eq!(
            probes_for(false),
            0,
            "show_versions=false must attempt zero probes"
        );
    }

    #[test]
    fn probe_runs_when_show_versions_true() {
        assert!(
            probes_for(true) > 0,
            "show_versions=true must attempt at least one probe"
        );
    }
}
