//! Building [`LanguageInfo`] display entries from detected languages.
//!
//! This module owns the pipeline that turns raw [`DetectedLanguage`] results
//! into what a prompt segment actually renders: confidence/filter/allow-list
//! selection ([`select_display_languages`], pure), version probing (gated on
//! [`LanguageSettings::show_versions`]), and theme color assignment.
//!
//! Deliberately has zero dependency on `crate::agent` or `crate::cache`: both
//! of those depend on this module (`cache::instant_prompt` renders the
//! language segment; `agent::oneshot` serves the oneshot CLI path), and a
//! dependency in the other direction would invert this crate's intended
//! layering (`language` sits below both).

use crate::config::{LanguageSettings, LanguageTheme};
use crate::ipc::LanguageInfo;
use crate::language::detector::{DetectedLanguage, get_language_color_from_theme};
use crate::theme::ThemeConfig;
use std::collections::HashMap;
use std::path::Path;

/// Test-only count of real version-probe attempts.
///
/// See [`build_language_display_info_at`]'s probing step. Incremented once
/// per language that actually reaches the probing logic -- never once per
/// candidate in `displayed_languages`, since the whole point of gating on
/// [`LanguageSettings::show_versions`] is that a hidden-versions request
/// makes zero probe attempts, not `N` attempts that each immediately return
/// `None`.
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

/// Select which detected languages should actually be displayed.
///
/// Applies the confidence threshold and the theme-or-config allow-list, drops
/// languages that cannot satisfy `show_versions`, and only then applies the
/// filter mode (`Primary`/`All`/`Top(n)`). The filter has to run last: it takes
/// from the front, so any narrowing done afterwards can empty a selection that
/// was supposed to name one language. Pure: no I/O, no version probing -- only
/// these selected languages warrant a version probe afterward.
#[must_use]
pub fn select_display_languages<'a>(
    detected: &'a [DetectedLanguage],
    theme_lang: &LanguageTheme,
    cfg: &LanguageSettings,
) -> Vec<&'a DetectedLanguage> {
    use std::collections::HashSet;

    // A theme may declare its own language allow-list (e.g. the starship preset
    // excludes fish to match Starship's module set). It takes precedence over
    // config.language.enabled_languages so a preset matches another tool's
    // module set without editing global config.
    let theme_languages = theme_lang
        .enabled_languages
        .as_deref()
        .filter(|list| !list.is_empty());
    let effective_enabled: Option<&[String]> = theme_languages.or_else(|| {
        (!cfg.enabled_languages.is_empty()).then_some(cfg.enabled_languages.as_slice())
    });
    let allowed_languages: Option<HashSet<String>> =
        effective_enabled.map(|list| list.iter().map(|name| name.to_lowercase()).collect());

    // Narrow to displayable languages BEFORE the filter mode picks a subset.
    // `Primary`/`Top(n)` take from the front, so anything that would be
    // discarded later has to be gone already or the selection comes back
    // empty -- the allow-list check used to run *after* the take, so a
    // `Primary` whose top language was not on the list rendered nothing.
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

    // Same trap, one layer down: when the renderer will drop languages that
    // have no version (`select_languages` in formatter::fish_ansi), selecting
    // only version-less ones renders an empty segment. A Node project whose
    // highest-confidence detection is JSON did exactly that under
    // `filter = "primary"` -- JSON has no version detector, so the one
    // selected language was then discarded and the segment vanished, while
    // `filter = "all"` still showed Node.
    //
    // Restrict the candidates to languages GPY can actually probe, unless that
    // would empty the list -- a project with no probeable language at all
    // keeps its previous behavior rather than silently changing which language
    // is named.
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

    // Apply filter mode. Only the survivors warrant a version probe.
    match cfg.filter {
        crate::config::types::LanguageFilter::Primary => candidates.into_iter().take(1).collect(),
        crate::config::types::LanguageFilter::All => candidates,
        crate::config::types::LanguageFilter::Top(count) => {
            candidates.into_iter().take(count).collect()
        }
    }
}

/// Build language display information from detected languages
///
/// Filters languages based on configuration and enriches with version and color data.
///
/// # Panics
///
/// Panics if internal color conversion fails for default fallback colors.
#[must_use]
pub fn build_language_display_info(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
) -> Vec<LanguageInfo> {
    build_language_display_info_at(detected_languages, theme, language_cfg, None, None)
}

/// Build language display information using a project directory for version resolution.
///
/// Version managers often resolve tools from project-local files, so prompt
/// rendering should pass the detected repository root when it is available.
///
/// # Panics
///
/// Panics if internal color conversion fails for default fallback colors.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "builds language display structs (name/version/color) by filtering, probing, and resolving version+color for each detected language in one linear pass"
)]
pub fn build_language_display_info_at(
    detected_languages: &[DetectedLanguage],
    theme: &ThemeConfig,
    language_cfg: &LanguageSettings,
    version_cwd: Option<&Path>,
    virtual_env: Option<&Path>,
) -> Vec<LanguageInfo> {
    if !language_cfg.enabled {
        return Vec::new();
    }

    let displayed_languages: Vec<&DetectedLanguage> =
        select_display_languages(detected_languages, &theme.segments.language, language_cfg);

    // Version probing is I/O (subprocess spawns) and pointless work when the
    // config/theme says not to show versions at all -- previously this ran
    // unconditionally regardless of `show_versions`.
    let probed_versions: Vec<Option<String>> = if language_cfg.show_versions {
        let version_detectors: HashMap<_, _> = crate::language::version::get_version_detectors()
            .iter()
            .map(|probe| (probe.language_name().to_owned(), probe.as_ref()))
            .collect();

        // Probe each displayed language's version concurrently. Every probe already
        // spawns its own subprocess with an internal 5s timeout and reader-thread
        // cleanup, so fanning out at this call makes cold-directory wall latency
        // ~= max(probe) instead of sum(probe) while preserving each probe's timeout
        // and cleanup semantics for free. Threads write into position-indexed slots,
        // so the resulting order is identical to the serial path.
        std::thread::scope(|scope| {
            // Spawn every probe eagerly, before joining any of them, so the fan-out
            // stays concurrent (`for` + explicit `Vec` here, not a lazy iterator
            // chain, which would spawn-then-immediately-join one probe at a time).
            let mut probe_handles = Vec::with_capacity(displayed_languages.len());
            for detected_lang in &displayed_languages {
                let probe = version_detectors.get(&detected_lang.name).copied();
                let is_python = detected_lang.name == "python";
                let is_ruby = detected_lang.name == "ruby";
                probe_handles.push(scope.spawn(move || {
                    #[cfg(test)]
                    PROBE_CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    // Python prefers a project venv (forwarded VIRTUAL_ENV, then
                    // .venv/venv) over the daemon's global interpreter. The daemon
                    // never inherits an activated venv, so bare `python` would
                    // otherwise report the global version.
                    if is_python
                        && let Some(cwd) = version_cwd
                        && let Some(venv) =
                            crate::language::venv::resolve_python_venv(cwd, virtual_env)
                        && let Some(version) = crate::language::venv::read_venv_version(&venv)
                    {
                        return Some(version);
                    }
                    // Ruby prefers a project `.ruby-version` pin over `mise exec`/PATH
                    // resolution when no mise/tool-versions config already governs the
                    // directory: mise only honors idiomatic version files for tools
                    // opted into `idiomatic_version_file_enable_tools`, which commonly
                    // excludes ruby, so `mise exec -- ruby --version` would otherwise
                    // silently report the globally pinned Ruby instead of the project's.
                    if is_ruby
                        && let Some(cwd) = version_cwd
                        && let Some(version) =
                            crate::language::version::resolve_ruby_version_file(cwd)
                    {
                        return Some(version);
                    }
                    probe.and_then(|source| {
                        crate::language::version::detect_language_release_at(source, version_cwd)
                            .ok()
                            .flatten()
                    })
                }));
            }

            probe_handles
                .into_iter()
                // A panicked probe thread degrades to "no version", matching the
                // serial path's `.ok().flatten()` swallow rather than propagating.
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
            reason = "the fallback only runs when the theme-derived color string fails to parse; \"#ffffff\" is a compile-time-constant literal, so parsing it is infallible in practice"
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
        // The reported bug: a Node project whose highest-confidence detection
        // is JSON. JSON has no version detector, so under `primary` it was
        // selected and then discarded by the renderer's version filter, and
        // the language segment disappeared entirely -- while `all` still
        // showed Node.
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
        // With show_versions off nothing is discarded downstream, so the
        // version-detector narrowing must not apply and JSON stays primary.
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
        // A project with no probeable language keeps naming its top detection
        // rather than selecting nothing at all.
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
        // The allow-list used to be applied after the take, so a primary whose
        // top language was not allowed selected nothing.
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

    // Shares the process-global `PROBE_CALL_COUNT` with the
    // show_versions=false test above: both reset it and then assert on it, so
    // running them concurrently lets one test's probes land inside the other's
    // measurement window. Serialized on a shared key rather than left to
    // scheduling luck (this surfaced when an unrelated module added tests and
    // shifted the parallel interleaving).
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
