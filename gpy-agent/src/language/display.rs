//! Building [`LanguageInfo`] display entries from detected languages.
//!
//! Must not depend on `crate::agent` or `crate::cache`; both depend on this.

use crate::config::types::ColorSpec;
use crate::config::{LanguageSettings, LanguageTheme};
use crate::debug_log;
use crate::ipc::LanguageInfo;
use crate::language::detector::{DetectedLanguage, get_language_color_from_theme};
use crate::language::venv::PythonVenv;
use crate::language::version::{ColdVersion, ProbePolicy, ReleaseSource, get_version_detectors};
use crate::theme::ThemeConfig;
use std::collections::{HashMap, HashSet};
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

    match cfg.filter {
        crate::config::types::LanguageFilter::Primary => candidates.into_iter().take(1).collect(),
        crate::config::types::LanguageFilter::All => candidates,
        crate::config::types::LanguageFilter::Top(count) => {
            candidates.into_iter().take(count).collect()
        }
    }
}

/// What to display: detected languages, the theme and settings selecting
/// among them, and where to resolve versions.
#[derive(Clone, Copy)]
pub struct DisplayRequest<'a> {
    detected: &'a [DetectedLanguage],
    theme: &'a ThemeConfig,
    settings: &'a LanguageSettings,
    version_cwd: Option<&'a Path>,
    python_venv: Option<&'a PythonVenv>,
    sources: &'a [Box<dyn ReleaseSource + Sync + Send>],
}

impl<'a> DisplayRequest<'a> {
    /// A request resolving versions through the registered release sources
    /// with no project directory.
    #[must_use]
    pub fn new(
        detected: &'a [DetectedLanguage],
        theme: &'a ThemeConfig,
        settings: &'a LanguageSettings,
    ) -> Self {
        Self {
            detected,
            theme,
            settings,
            version_cwd: None,
            python_venv: None,
            sources: get_version_detectors(),
        }
    }

    /// Resolve versions from `version_cwd`; Python uses a project venv under it
    /// unless [`Self::with_venv`] supplies one.
    #[must_use]
    pub const fn with_cwd(mut self, version_cwd: Option<&'a Path>) -> Self {
        self.version_cwd = version_cwd;
        self
    }

    /// Use `python_venv`, already resolved by the caller, for Python's version.
    #[must_use]
    pub const fn with_venv(mut self, python_venv: Option<&'a PythonVenv>) -> Self {
        self.python_venv = python_venv;
        self
    }

    /// Probe through `sources` instead of the registered ones.
    #[cfg(test)]
    const fn with_sources(mut self, sources: &'a [Box<dyn ReleaseSource + Sync + Send>]) -> Self {
        self.sources = sources;
        self
    }
}

/// Where [`display_info`] gets versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versions {
    /// Probe cold versions, each bounded by its own timeout.
    Probe,
    /// Cache only; a cold version fails with [`ColdVersion`].
    Cached,
    /// Cache only; a cold version shows as none instead of failing. For
    /// after a probe job for these languages finished: it cached every result,
    /// success or failure, so a cold one means the entry is already gone again
    /// (expired, evicted, or a wall-clock step back made it read as stale).
    Settled,
}

/// Display entries for `request` with versions from `versions`.
///
/// # Errors
/// [`ColdVersion`] under [`Versions::Cached`] when a displayed language's
/// version is not cached.
pub fn display_info(
    request: &DisplayRequest<'_>,
    versions: Versions,
) -> Result<Vec<LanguageInfo>, ColdVersion> {
    let settings = request.settings;
    if !settings.enabled {
        return Ok(Vec::new());
    }

    let displayed =
        select_display_languages(request.detected, &request.theme.segments.language, settings);
    let version_slots = if settings.show_versions {
        resolve_versions(request, &displayed, versions)?
    } else {
        vec![None; displayed.len()]
    };

    Ok(displayed
        .into_iter()
        .zip(version_slots)
        .map(|(detected_lang, version)| LanguageInfo {
            name: detected_lang.name.clone(),
            version,
            color: language_color(&detected_lang.name, &request.theme.segments.language),
        })
        .collect())
}

/// `name`'s themed colour, else white.
fn language_color(name: &str, theme_lang: &LanguageTheme) -> ColorSpec {
    ColorSpec::new(&get_language_color_from_theme(name, theme_lang))
        .unwrap_or_else(|_| ColorSpec::white())
}

/// One version per language in `displayed`, in order.
fn resolve_versions(
    request: &DisplayRequest<'_>,
    displayed: &[&DetectedLanguage],
    versions: Versions,
) -> Result<Vec<Option<String>>, ColdVersion> {
    let sources: HashMap<&str, &(dyn ReleaseSource + Sync + Send)> = request
        .sources
        .iter()
        .map(|source| (source.language_name(), source.as_ref()))
        .collect();
    let policy = match versions {
        Versions::Probe => ProbePolicy::Spawn,
        Versions::Cached | Versions::Settled => ProbePolicy::CachedOnly,
    };
    let lookup = |detected_lang: &DetectedLanguage| {
        language_version(
            detected_lang,
            sources.get(detected_lang.name.as_str()).copied(),
            request.version_cwd,
            request.python_venv,
            policy,
        )
    };

    match versions {
        Versions::Cached => displayed.iter().map(|lang| lookup(lang)).collect(),
        Versions::Settled => Ok(displayed
            .iter()
            .map(|lang| lookup(lang).unwrap_or(None))
            .collect()),
        Versions::Probe => Ok(probe_concurrently(displayed, &lookup)),
    }
}

/// Run `lookup` for every language on its own thread (each probe has its own
/// timeout); slots keep `displayed`'s order. A probe that panics logs and
/// shows no version.
fn probe_concurrently<F>(displayed: &[&DetectedLanguage], lookup: &F) -> Vec<Option<String>>
where
    F: Fn(&DetectedLanguage) -> Result<Option<String>, ColdVersion> + Sync,
{
    std::thread::scope(|scope| {
        // Spawn every probe before joining any; a lazy iterator would serialize.
        let handles: Vec<_> = displayed
            .iter()
            .map(|&detected_lang| scope.spawn(move || lookup(detected_lang).ok().flatten()))
            .collect();
        displayed
            .iter()
            .zip(handles)
            .map(|(detected_lang, handle)| {
                handle.join().unwrap_or_else(|_panic| {
                    debug_log!(
                        "language",
                        "version probe for {} panicked",
                        detected_lang.name
                    );
                    None
                })
            })
            .collect()
    })
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
    use super::{DisplayRequest, Versions, display_info, select_display_languages};
    use crate::config::LanguageSettings;
    use crate::language::detector::DetectedLanguage;
    use crate::language::version::ColdVersion;
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

        let theme = ThemeConfig::default();
        let request = DisplayRequest::new(&detected, &theme, &language_cfg)
            .with_cwd(Some(cwd.path()))
            .with_sources(&sources);
        let info =
            display_info(&request, Versions::Probe).expect("probing never reports a cold version");

        assert_eq!(
            info.len(),
            1,
            "the language is displayed with or without a version"
        );
        probes.load(Ordering::SeqCst)
    }

    #[test]
    fn fallback_white_is_the_canonical_white() {
        assert_eq!(
            crate::config::types::ColorSpec::white(),
            crate::config::types::ColorSpec::new("#ffffff").unwrap()
        );
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
    /// renders without a version instead of failing the request.
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

        let request =
            DisplayRequest::new(&detected, &theme, &language_cfg).with_cwd(Some(dir.path()));
        assert_eq!(
            display_info(&request, Versions::Cached).map(|_| ()),
            Err(ColdVersion),
            "never probed here, so the version is cold"
        );
        let settled = display_info(&request, Versions::Settled).expect("settled never fails");
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
