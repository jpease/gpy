//! IPC handler for language detection requests; detection lives in [`crate::language`].

use super::{
    HandlerError, JobGuard, RenderDeps, RequestHandler, SharedJob, SingleFlight, notify_if_changed,
    spawn_detached,
};
use crate::ipc::{LanguageInfo, Message, Response};
use crate::language::DetectionCache;
use crate::language::detection_cache::LANGUAGE_REFRESH_INTERVAL;
use crate::language::display::{build_language_display_info_at, cached_language_display_info_at};
use crate::language::venv::PythonVenv;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Handler for programming language detection queries.
pub struct LanguageHandler {
    /// Publishing handles; see [`LanguageHandler::publish_language_status`].
    render: RenderDeps,
    language_cache: DetectionCache,
    in_flight: SingleFlight<PathBuf, Vec<crate::language::DetectedLanguage>>,
    /// Version probes running off the request path, one per project root.
    probes_in_flight: SingleFlight<PathBuf, ()>,
}

type LanguageJob = SharedJob<Vec<crate::language::DetectedLanguage>>;
type LanguageJobGuard = JobGuard<PathBuf, Vec<crate::language::DetectedLanguage>>;

impl LanguageHandler {
    /// Create a new `LanguageHandler`.
    #[must_use]
    pub fn new(render: RenderDeps, language_cache: DetectionCache) -> Self {
        Self {
            render,
            language_cache,
            in_flight: SingleFlight::new(),
            probes_in_flight: SingleFlight::new(),
        }
    }

    /// Handle language detection for a given path.
    ///
    /// The whole request, detection and version probes included, waits at most
    /// `agent.timeout_seconds`. A version probe that outlasts it keeps running,
    /// then writes the instant cache and rings the shells itself.
    ///
    /// # Errors
    /// Fails if a cold detection or version probe exceeds the timeout budget;
    /// the work keeps running and its result is served from cache next time.
    fn handle_language_detection(
        &self,
        path: &crate::security::SafePath,
        _is_last: bool,
        prev_bg: Option<&str>,
        virtual_env: Option<&str>,
    ) -> Result<Response, HandlerError> {
        let config = self.render.config_manager.get();
        if !config.language.enabled {
            return Ok(Response::Language { languages: vec![] });
        }
        let deadline = Instant::now() + Duration::from_secs(config.agent.timeout_seconds.get());

        let theme = self.render.theme_manager.get();
        let request_path = path.as_path();
        // Detect at the git root, else the nearest marker ancestor. Outside git
        // the instant cache stays keyed by the request path (`realpath $PWD`).
        let project_root = crate::language::project_root(request_path);
        let in_git_repo = project_root.is_git();
        let repo_root = project_root.into_path();
        let cache_path = if in_git_repo {
            repo_root.as_path()
        } else {
            request_path
        };

        // Stash the forwarded venv so background refreshes render the same
        // interpreter; a request without one clears it. Instant-cache files are
        // keyed by repo, not venv, so the most recent request wins. Resolved
        // against the project root: outside git, `cache_path` may be a
        // subdirectory without the project's `.venv`.
        let python_venv =
            crate::language::venv::resolve_python_venv(&repo_root, virtual_env.map(Path::new));
        match &python_venv {
            Some(PythonVenv::Forwarded(venv)) => {
                crate::language::venv::stash_project_venv(&repo_root, venv);
            }
            _ => crate::language::venv::clear_project_venv(&repo_root),
        }

        // On a cold miss, wait on the single-flight job only up to a bounded budget.
        let detected_languages =
            if let Some((cached, age)) = self.language_cache.get_with_age(&repo_root) {
                // Never wait here: the stale value is served now and the job's
                // result reaches the shell through the repaint doorbell.
                drop(self.revalidate_stale_hit(&repo_root, in_git_repo, age));
                cached
            } else {
                let job = self.language_detection_job(&repo_root);
                let Some(detected) = job.wait_timeout(remaining(deadline)) else {
                    // Error rather than empty success, so a slow scan never
                    // clobbers a good prompt with a blank one.
                    return Err(HandlerError::TimedOut("Language detection"));
                };
                detected
            };

        // Versions come from the caches; a cold one is probed by a background
        // job, waited on only for what is left of the budget.
        let cached = || {
            cached_language_display_info_at(
                &detected_languages,
                &theme,
                &config.language,
                Some(&repo_root),
                python_venv.as_ref(),
            )
        };
        let (languages, published) = if let Some(languages) = cached() {
            (languages, false)
        } else {
            let (probe, started_here) = self.version_probe_job(
                cache_path,
                &repo_root,
                &detected_languages,
                python_venv.as_ref(),
                prev_bg,
            );
            // Error rather than a version-less render, for the same reason as
            // a slow detection. The job publishes when it finishes.
            let languages = probe
                .wait_timeout(remaining(deadline))
                .and_then(|()| cached())
                .ok_or(HandlerError::TimedOut("Language version probe"))?;
            // A job this request started already published its inputs.
            (languages, started_here)
        };

        // Refresh the instant cache and repaint other live shells on change.
        if !published {
            Self::publish_languages(&self.render, cache_path, &languages, prev_bg);
        }

        Ok(Response::Language { languages })
    }

    /// Probe `detected`'s cold versions off the request path, publish them for
    /// `cache_path` (and `prev_bg`), repainting on change, then complete. One
    /// job per project root; a request that finds another request's job
    /// running waits for it, and is `false` in the returned pair.
    fn version_probe_job(
        &self,
        cache_path: &Path,
        repo_root: &Path,
        detected: &[crate::language::DetectedLanguage],
        python_venv: Option<&PythonVenv>,
        prev_bg: Option<&str>,
    ) -> (Arc<SharedJob<()>>, bool) {
        let render = self.render.clone();
        let cache_path = cache_path.to_path_buf();
        let repo_root_owned = repo_root.to_path_buf();
        let detected = detected.to_vec();
        let python_venv = python_venv.cloned();
        let prev_bg = prev_bg.map(str::to_owned);
        let mut started = false;
        let job = self
            .probes_in_flight
            .get_or_start(repo_root.to_path_buf(), (), |guard| {
                started = true;
                spawn_detached(move || {
                    let config = render.config_manager.get();
                    let theme = render.theme_manager.get();
                    let languages = build_language_display_info_at(
                        &detected,
                        &theme,
                        &config.language,
                        Some(&repo_root_owned),
                        python_venv.as_ref(),
                    );
                    Self::publish_languages(&render, &cache_path, &languages, prev_bg.as_deref());
                    guard.finish(());
                });
            });
        (job, started)
    }

    /// Start a background re-detection for a non-git cache hit older than
    /// [`LANGUAGE_REFRESH_INTERVAL`]; non-git directories aren't watched.
    /// Returns the job handle (for tests), or `None`; never awaited.
    #[must_use]
    fn revalidate_stale_hit(
        &self,
        repo_root: &Path,
        in_git_repo: bool,
        age: Duration,
    ) -> Option<Arc<LanguageJob>> {
        if in_git_repo || age < LANGUAGE_REFRESH_INTERVAL {
            return None;
        }
        Some(self.language_detection_job(repo_root))
    }

    fn language_detection_job(&self, repo_root: &Path) -> Arc<LanguageJob> {
        let language_cache = self.language_cache.clone();
        let render = self.render.clone();
        let repo_root_owned = repo_root.to_path_buf();
        // Fallback if the job panics before finishing.
        self.in_flight
            .get_or_start(repo_root_owned.clone(), Vec::new(), move |guard| {
                Self::spawn_language_detection_job(repo_root_owned, language_cache, render, guard);
            })
    }

    fn spawn_language_detection_job(
        repo_root: PathBuf,
        language_cache: DetectionCache,
        render: RenderDeps,
        guard: LanguageJobGuard,
    ) {
        let job = move || {
            let detection_mode = render.config_manager.get().language.detection_mode;
            let detected = crate::language::detector::Detector::detect_directory_bounded(
                &repo_root,
                detection_mode,
            );
            language_cache.set(&repo_root, detected.clone());
            guard.finish(detected.clone());
            Self::publish_language_status(&render, &repo_root, &detected);
        };

        spawn_detached(job);
    }

    /// Write the instant-prompt cache and repaint live shells when the detected
    /// languages change the rendered output.
    fn publish_language_status(
        deps: &RenderDeps,
        repo_root: &Path,
        detected: &[crate::language::DetectedLanguage],
    ) {
        let config = deps.config_manager.get();
        let theme = deps.theme_manager.get();
        // A background job, so a cold version is probed here. No request
        // context, so prev_bg is None; reuse the stashed venv.
        let stashed_venv = crate::language::venv::stashed_project_venv(repo_root);
        let languages = build_language_display_info_at(
            detected,
            &theme,
            &config.language,
            Some(repo_root),
            stashed_venv.as_ref(),
        );
        Self::publish_languages(deps, repo_root, &languages, None);
    }

    /// Write `languages` into the instant cache for `cache_path` and repaint
    /// live shells there on change.
    fn publish_languages(
        deps: &RenderDeps,
        cache_path: &Path,
        languages: &[LanguageInfo],
        prev_bg: Option<&str>,
    ) {
        let config = deps.config_manager.get();
        let theme = deps.theme_manager.get();
        let palette = deps.palette_cache.get();
        let result = deps
            .instant_cache
            .write_language_variants(cache_path, languages, &config, &theme, prev_bg, &palette);
        notify_if_changed(&deps.client_registry, cache_path, result, "language");
    }
}

/// Time left until `deadline`, zero once it has passed.
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

impl RequestHandler for LanguageHandler {
    fn handle(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::LanguageDetect {
                path,
                is_last,
                prev_bg,
                virtual_env,
                ..
            } => self.handle_language_detection(
                path,
                *is_last,
                prev_bg.as_deref(),
                virtual_env.as_deref(),
            ),
            _ => Err(HandlerError::UnexpectedMessage {
                handler: "LanguageHandler",
                qualifier: "non-language",
                message: format!("{message:?}"),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "LanguageHandler"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    use super::*;
    use crate::cache::InstantPromptCache;
    use crate::config::manager::ConfigManager;
    use crate::formatter::Format;
    use crate::ipc::ClientDirectory;
    use crate::language::DetectedLanguage;
    use crate::palette::PaletteCache;
    use crate::theme::ThemeManager;

    fn make_handler() -> (LanguageHandler, Arc<ClientDirectory>) {
        make_handler_with_cache(InstantPromptCache::new_for_test())
    }

    fn make_handler_with_cache(
        instant_cache: InstantPromptCache,
    ) -> (LanguageHandler, Arc<ClientDirectory>) {
        let registry = ClientDirectory::new().shared();
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
        let render = RenderDeps {
            config_manager: Arc::clone(&config_manager),
            // Builtin theme keeps the test hermetic against on-disk themes.
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("theme")),
            palette_cache,
            instant_cache: Arc::new(instant_cache),
            client_registry: Arc::clone(&registry),
        };
        let handler = LanguageHandler::new(render, DetectionCache::new());
        (handler, registry)
    }

    /// Build a synchronous `LanguageDetect` request for `repo_root`.
    fn language_detect_request(repo_root: &Path) -> Message {
        Message::LanguageDetect {
            path: crate::security::SafePath::new(repo_root.to_str().expect("utf8 path"))
                .expect("safe path"),
            format: Format::default(),
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        }
    }

    /// Same as [`language_detect_request`] but forwarding `virtual_env`.
    fn language_detect_request_with_venv(repo_root: &Path, venv: Option<&Path>) -> Message {
        Message::LanguageDetect {
            path: crate::security::SafePath::new(repo_root.to_str().expect("utf8 path"))
                .expect("safe path"),
            format: Format::default(),
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: venv.map(|v| v.to_str().expect("utf8 path").to_owned()),
        }
    }

    /// A request without a usable `virtual_env` clears the stash.
    #[test]
    fn request_without_virtual_env_clears_stash() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        std::fs::write(repo_dir.join("pyproject.toml"), "[project]\nname=\"x\"\n")
            .expect("pyproject");
        std::fs::write(repo_dir.join("m.py"), "p = 1\n").expect("py file");
        let repo_root = std::fs::canonicalize(&repo_dir).expect("canonicalize repo root");
        let ext_venv = temp_dir.path().join("extvenv");
        std::fs::create_dir_all(&ext_venv).expect("venv dir");
        std::fs::write(ext_venv.join("pyvenv.cfg"), "home = /x\nversion = 3.9.9\n")
            .expect("pyvenv.cfg");
        let not_a_venv = temp_dir.path().join("plain");
        std::fs::create_dir_all(&not_a_venv).expect("plain dir");

        handler.language_cache.set(
            &repo_root,
            vec![DetectedLanguage {
                name: "python".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 6,
            }],
        );

        for follow_up in [None, Some(not_a_venv.as_path())] {
            handler
                .handle(&language_detect_request_with_venv(
                    &repo_root,
                    Some(&ext_venv),
                ))
                .expect("request with venv");
            assert!(
                crate::language::venv::stashed_project_venv(&repo_root).is_some(),
                "a forwarded venv must be stashed"
            );
            handler
                .handle(&language_detect_request_with_venv(&repo_root, follow_up))
                .expect("request without usable venv");
            assert_eq!(
                crate::language::venv::stashed_project_venv(&repo_root),
                None,
                "a request without a usable virtual_env must clear the stash"
            );
        }
    }

    /// Python version a `LanguageDetect` response reports, if any.
    #[cfg(unix)]
    fn python_version(response: &Response) -> Option<String> {
        let Response::Language { languages } = response else {
            return None;
        };
        languages
            .iter()
            .find(|lang| lang.name == "python")
            .and_then(|lang| lang.version.clone())
    }

    /// Write an executable `<venv>/bin/python` that records each run in `log`.
    #[cfg(unix)]
    fn write_logging_python(venv: &Path, log: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        let python = crate::language::venv::venv_python_binary(venv);
        std::fs::create_dir_all(python.parent().expect("bin dir")).expect("bin dir");
        std::fs::write(
            &python,
            format!(
                "#!/bin/sh\necho x >> '{}'\necho 'Python {version}'\n",
                log.display()
            ),
        )
        .expect("python script");
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755))
            .expect("chmod python");
    }

    /// The project `.venv` interpreter never runs; a forwarded venv's does.
    #[cfg(unix)]
    #[test]
    fn only_a_forwarded_venv_interpreter_is_probed() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        let repo_root = std::fs::canonicalize(&repo_dir).expect("canonicalize repo root");
        let project_log = temp_dir.path().join("project.log");
        write_logging_python(&repo_root.join(".venv"), &project_log, "3.11.9");
        let forwarded = temp_dir.path().join("activated");
        let forwarded_log = temp_dir.path().join("forwarded.log");
        write_logging_python(&forwarded, &forwarded_log, "3.8.2");
        handler.language_cache.set(
            &repo_root,
            vec![DetectedLanguage {
                name: "python".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 6,
            }],
        );

        let response = handler
            .handle(&language_detect_request(&repo_root))
            .expect("request without venv");
        assert_eq!(python_version(&response), None);
        assert!(
            !project_log.exists(),
            "the project interpreter must not run"
        );

        let response = handler
            .handle(&language_detect_request_with_venv(
                &repo_root,
                Some(&forwarded),
            ))
            .expect("request with forwarded venv");
        assert_eq!(python_version(&response).as_deref(), Some("3.8.2"));
        assert!(
            forwarded_log.exists(),
            "the forwarded interpreter is probed"
        );
        assert!(
            !project_log.exists(),
            "the project interpreter must not run"
        );
    }

    /// A version probe that outlasts the request budget does not hold the
    /// request: it fails within the budget, and the probe publishes its result
    /// (instant cache written, shells rung) when it lands.
    #[cfg(unix)]
    #[test]
    fn slow_version_probe_is_published_after_the_request_gives_up() {
        use std::os::unix::fs::PermissionsExt;

        let (handler, registry) = make_handler();
        let mut config = (*handler.render.config_manager.get()).clone();
        config.agent.timeout_seconds =
            crate::config::types::AgentTimeout::new(1).expect("1 s timeout");
        handler.render.config_manager.overwrite_for_tests(config);

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        let repo_root = std::fs::canonicalize(&repo_dir).expect("canonicalize repo root");
        // A forwarded venv without a version in pyvenv.cfg: only its
        // interpreter can answer, and it takes 2 s.
        let forwarded = temp_dir.path().join("activated");
        let python = crate::language::venv::venv_python_binary(&forwarded);
        std::fs::create_dir_all(python.parent().expect("bin dir")).expect("bin dir");
        std::fs::write(&python, "#!/bin/sh\nsleep 2\necho 'Python 3.7.4'\n").expect("python");
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755))
            .expect("chmod python");
        handler.language_cache.set(
            &repo_root,
            vec![DetectedLanguage {
                name: "python".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 6,
            }],
        );
        let request = language_detect_request_with_venv(&repo_root, Some(&forwarded));
        let notified_before = registry.notify_invocations();

        let started = std::time::Instant::now();
        let result = handler.handle(&request);
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(HandlerError::TimedOut(_))),
            "a cold probe past the budget must time out, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1800),
            "the request waited {elapsed:?} on a 2 s probe with a 1 s budget"
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while registry.notify_invocations() == notified_before
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            registry.notify_invocations() > notified_before,
            "the probe's result must be published and rung"
        );

        let started = std::time::Instant::now();
        let response = handler.handle(&request).expect("warm request");
        assert_eq!(python_version(&response).as_deref(), Some("3.7.4"));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "a warm request must not probe again"
        );
    }

    /// A synchronous request repaints other live shells only on change.
    #[test]
    fn handle_language_detection_notifies_only_on_content_change() {
        let (handler, registry) = make_handler();
        // Unique temp path so the first write is always a change.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        // The cache key must match the canonicalized root.
        let repo_root = std::fs::canonicalize(&repo_dir).expect("canonicalize repo root");

        handler.language_cache.set(
            &repo_root,
            vec![DetectedLanguage {
                name: "rust".to_owned(),
                confidence: 1.0,
                file_count: 3,
                total_bytes: 512,
            }],
        );

        let message = language_detect_request(&repo_root);

        handler.handle(&message).expect("first request");
        assert_eq!(
            registry.notify_invocations(),
            1,
            "a synchronous request that changes the cached output should repaint live shells"
        );

        handler.handle(&message).expect("second request");
        assert_eq!(
            registry.notify_invocations(),
            1,
            "an unchanged refresh must not wake terminals"
        );
    }

    /// Seed an empty cache entry for `root`, aged past the refresh interval.
    fn seed_stale_empty_entry(handler: &LanguageHandler, root: &Path) {
        handler.language_cache.set(root, Vec::new());
        handler.language_cache.age_entry_for_test(
            root,
            LANGUAGE_REFRESH_INTERVAL.saturating_add(Duration::from_secs(1)),
        );
    }

    fn write_rust_project(dir: &Path) {
        std::fs::create_dir_all(dir.join("src")).expect("src dir");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("Cargo.toml");
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("main.rs");
    }

    /// A stale non-git cache hit is served immediately and re-detects in the background.
    #[test]
    fn stale_non_git_hit_triggers_background_redetection() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = std::fs::canonicalize(temp_dir.path()).expect("canonicalize");
        write_rust_project(&root);
        seed_stale_empty_entry(&handler, &root);

        let response = handler
            .handle(&language_detect_request(&root))
            .expect("stale hit request");
        assert!(
            matches!(&response, Response::Language { languages } if languages.is_empty()),
            "a cache hit must be served immediately from the stale entry: {response:?}"
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let detected_rust = loop {
            let found = handler
                .language_cache
                .get(&root)
                .is_some_and(|langs| langs.iter().any(|l| l.name == "rust"));
            if found || std::time::Instant::now() >= deadline {
                break found;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            detected_rust,
            "stale non-git hit must re-detect in the background"
        );
    }

    /// A stale git-root hit starts no detection job (the watcher covers it).
    #[test]
    fn stale_git_hit_does_not_trigger_redetection() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        let root = std::fs::canonicalize(&repo_dir).expect("canonicalize");
        write_rust_project(&root);
        seed_stale_empty_entry(&handler, &root);

        handler
            .handle(&language_detect_request(&root))
            .expect("stale hit request");

        assert!(
            handler
                .revalidate_stale_hit(&root, true, LANGUAGE_REFRESH_INTERVAL)
                .is_none(),
            "a git root must not start a detection job from a stale hit"
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            handler
                .language_cache
                .get(&root)
                .is_some_and(|langs| langs.is_empty()),
            "a git root's entry must be left untouched by a stale hit"
        );
    }

    /// A non-git subdirectory reports its project's languages; the instant cache
    /// stays keyed by the request path.
    #[test]
    fn non_git_subdirectory_detects_at_marker_ancestor() {
        let cache_dir = tempfile::TempDir::new().expect("cache dir");
        let (handler, _registry) = make_handler_with_cache(
            InstantPromptCache::new_in_dir(cache_dir.path().to_path_buf()).expect("cache"),
        );
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = std::fs::canonicalize(temp_dir.path()).expect("canonicalize");
        write_rust_project(&root);
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs).expect("docs dir");
        std::fs::write(docs.join("README.md"), "# d\n").expect("README.md");

        let response = handler
            .handle(&language_detect_request(&docs))
            .expect("docs request");
        assert!(
            matches!(&response, Response::Language { languages }
                if languages.iter().any(|l| l.name == "rust")),
            "docs/ must report the project's rust: {response:?}"
        );
        assert!(
            handler.language_cache.get(&root).is_some(),
            "detection must be cached under the marker ancestor"
        );

        // Key ends `docs` after the escaped separator (`_s` for `/`, `_b` for `\`).
        let docs_marker = if cfg!(windows) {
            "_bdocs.lang"
        } else {
            "_sdocs.lang"
        };
        let docs_entry_rendered = std::fs::read_dir(cache_dir.path())
            .expect("read cache dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(docs_marker))
            .any(|entry| {
                std::fs::read_to_string(entry.path()).is_ok_and(|prompt| !prompt.is_empty())
            });
        assert!(
            docs_entry_rendered,
            "the instant cache must stay keyed by the request path"
        );
    }

    /// A marker-less non-git subdirectory follows an ancestor marker appearing
    /// and disappearing once its cached root and detection go stale.
    #[test]
    fn non_git_subdirectory_follows_ancestor_marker_changes() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let project = std::fs::canonicalize(temp_dir.path()).expect("canonicalize");
        let docs = project.join("docs");
        std::fs::create_dir_all(&docs).expect("docs dir");
        std::fs::write(docs.join("README.md"), "# d\n").expect("README.md");
        let names = |response: Response| match response {
            Response::Language { languages } => languages
                .into_iter()
                .map(|lang| lang.name)
                .collect::<Vec<_>>(),
            other => panic!("unexpected response: {other:?}"),
        };
        let request = || {
            handler
                .handle(&language_detect_request(&docs))
                .expect("docs request")
        };
        let go_stale = || {
            crate::language::age_non_git_root_for_test(&docs, LANGUAGE_REFRESH_INTERVAL);
            handler.language_cache.age_entry_for_test(
                &docs,
                LANGUAGE_REFRESH_INTERVAL.saturating_add(Duration::from_secs(1)),
            );
        };

        assert_eq!(names(request()), Vec::<String>::new());

        write_rust_project(&project);
        go_stale();
        assert_eq!(
            names(request()),
            ["rust"],
            "a new ancestor marker is honored"
        );

        std::fs::remove_file(project.join("Cargo.toml")).expect("remove marker");
        go_stale();
        assert_eq!(
            names(request()),
            Vec::<String>::new(),
            "a removed ancestor marker is dropped"
        );
    }
}
