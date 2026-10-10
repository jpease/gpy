//! IPC handler for language detection requests.
//!
//! Language requests combine project language detection, optional version
//! lookup, and prompt formatting context. The expensive detection logic lives in
//! [`crate::language`]; this handler applies IPC validation, configuration
//! toggles, and cache-aware orchestration for shell clients.

use super::{
    HandlerError, JobGuard, RenderDeps, RequestHandler, SharedJob, SingleFlight, notify_if_changed,
    spawn_detached,
};
use crate::ipc::{Message, Response};
use crate::language::DetectionCache;
use crate::language::detection_cache::LANGUAGE_REFRESH_INTERVAL;
use crate::language::display::build_language_display_info_at;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Handler for programming language detection queries
pub struct LanguageHandler {
    /// Everything needed to publish detected languages to the instant-prompt
    /// cache and repaint live shells; see [`LanguageHandler::publish_language_status`].
    render: RenderDeps,
    language_cache: DetectionCache,
    in_flight: SingleFlight<PathBuf, Vec<crate::language::DetectedLanguage>>,
}

type LanguageJob = SharedJob<Vec<crate::language::DetectedLanguage>>;
type LanguageJobGuard = JobGuard<PathBuf, Vec<crate::language::DetectedLanguage>>;

impl LanguageHandler {
    /// Create a new `LanguageHandler` with the given dependencies
    #[must_use]
    pub fn new(render: RenderDeps, language_cache: DetectionCache) -> Self {
        Self {
            render,
            language_cache,
            in_flight: SingleFlight::new(),
        }
    }

    /// Handle language detection for a given path
    ///
    /// # Errors
    ///
    /// Returns an error if a cold cache miss does not complete within the agent
    /// timeout budget (#154); the detection keeps running on the blocking pool and
    /// its result is served from cache on a subsequent request.
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

        // Use the shared theme manager instead of loading from disk
        let theme = self.render.theme_manager.get();
        let request_path = path.as_path();
        // Detect at the project root: the git root, else the nearest marker
        // ancestor (#727). The instant cache stays keyed by the request path
        // outside git, because the shells look it up by `realpath $PWD` there;
        // inside git both are the repo root.
        let project_root = crate::language::project_root(request_path);
        let in_git_repo = project_root.is_git();
        let repo_root = project_root.into_path();
        let cache_path = if in_git_repo {
            repo_root.as_path()
        } else {
            request_path
        };

        // Resolve the forwarded venv (if any) against this repo and stash it so
        // the background detection job renders the same interpreter version,
        // preventing a flip-flop between the synchronous reply and later refreshes.
        // The stash mirrors the most recent request: shells omit `virtual_env`
        // once the venv is deactivated, so a request without a usable one clears
        // it (#726). Known limitation: the instant-cache files are keyed by repo,
        // not venv, so two shells in one repo with different venv states share one
        // rendered result and the most recent request wins.
        let forwarded_venv = virtual_env
            .map(std::path::Path::new)
            .and_then(|env| crate::language::venv::resolve_python_venv(&repo_root, Some(env)));
        match forwarded_venv.as_deref() {
            Some(venv) => crate::language::venv::stash_project_venv(&repo_root, venv),
            None => crate::language::venv::clear_project_venv(&repo_root),
        }
        // The venv the reply renders with, resolved against the project root:
        // the instant cache is written for `cache_path`, which outside git
        // may be a subdirectory without the project's `.venv`.
        let project_venv =
            forwarded_venv.or_else(|| crate::language::venv::resolve_python_venv(&repo_root, None));

        // Check the in-memory cache first. On a cold miss, wait on the single-flight
        // detection job only up to a bounded budget so a slow scan never blocks the
        // request indefinitely (#154). The wait runs on the blocking pool (the IPC
        // server routes requests via `spawn_blocking`), so it never ties up a Tokio
        // worker thread.
        let detected_languages =
            if let Some((cached, age)) = self.language_cache.get_with_age(&repo_root) {
                // Never wait here: the stale value is served now and the job's
                // result reaches the shell through the repaint doorbell (#709).
                drop(self.revalidate_stale_hit(&repo_root, in_git_repo, age));
                cached
            } else {
                let job = self.language_detection_job(&repo_root);
                let wait_budget = Duration::from_secs(config.agent.timeout_seconds.get());
                let Some(detected) = job.wait_timeout(wait_budget) else {
                    // Detection exceeded the budget and is still running on the
                    // blocking pool. Return an error (mirroring the git handler's
                    // timeout path) rather than an empty success: an error response
                    // leaves the instant cache and live shells untouched, so a slow
                    // scan never clobbers a good prompt with a blank one. The result
                    // lands in the in-memory cache and is served on the next prompt.
                    return Err(HandlerError::TimedOut("Language detection"));
                };
                detected
            };

        // Refresh the instant cache so subsequent shell prompts render without IPC,
        // and repaint any other live shells in this repo when the rendered output
        // actually changed (the requesting shell already gets the fresh reply
        // directly via the response below). Mirrors the background path's
        // `publish_language_status` and `git_handler`'s synchronous
        // `publish_and_repaint` (#624).
        let palette = self.render.palette_cache.get();
        let result = self.render.instant_cache.write_language_variants(
            cache_path,
            &detected_languages,
            &config,
            &theme,
            prev_bg,
            &palette,
            project_venv.as_deref(),
        );
        notify_if_changed(&self.render.client_registry, cache_path, result, "language");

        if detected_languages.is_empty() {
            return Ok(Response::Language { languages: vec![] });
        }

        let languages = build_language_display_info_at(
            &detected_languages,
            &theme,
            &config.language,
            Some(&repo_root),
            project_venv.as_deref(),
        );

        Ok(Response::Language { languages })
    }

    /// Start a background re-detection for a cache hit older than
    /// [`LANGUAGE_REFRESH_INTERVAL`] when `repo_root` is not inside a git repo.
    ///
    /// Git repos are refreshed by the watcher and the signature-throttled
    /// git-status path; non-git directories are not watched, so without this a
    /// cached result would be served for the agent's lifetime (#709). Returns
    /// the job handle (for tests), or `None` when no revalidation is due. NEVER
    /// awaited on the request path; the job is single-flight per root.
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
        // On a panic mid-job, the guard's Drop completes the shared job with
        // an empty result (matching this type's "no languages detected"
        // semantics) instead of leaving the slot permanently poisoned (#318).
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
            // Mirror the git handler's publish_and_repaint: write the
            // instant-prompt cache and ring a content-gated repaint doorbell so live
            // prompts repaint without waiting for the next render (#166).
            Self::publish_language_status(&render, &repo_root, &detected);
        };

        spawn_detached(job);
    }

    /// Write the instant-prompt cache and ring a content-gated repaint doorbell when the
    /// detected languages produce different rendered output. The language-domain
    /// counterpart of `git_handler::publish_and_repaint`, so live prompts receive
    /// deferred results without requiring another render.
    fn publish_language_status(
        deps: &RenderDeps,
        repo_root: &Path,
        detected: &[crate::language::DetectedLanguage],
    ) {
        let config = deps.config_manager.get();
        let theme = deps.theme_manager.get();
        let palette = deps.palette_cache.get();
        // Background detection job: no IPC request context, so prev_bg is None.
        // Reuse any venv the synchronous handler stashed for this repo so the
        // refreshed render matches the interpreter version the client saw.
        let stashed_venv = crate::language::venv::stashed_project_venv(repo_root);
        let result = deps.instant_cache.write_language_variants(
            repo_root,
            detected,
            &config,
            &theme,
            None,
            &palette,
            stashed_venv.as_deref(),
        );
        notify_if_changed(&deps.client_registry, repo_root, result, "language");
    }
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
            // Use the embedded builtin theme so the test is hermetic against a
            // stale on-disk ~/.config/gpy/themes/default.toml, matching
            // git_handler's equivalent fixture.
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

    /// #726: the stash mirrors the most recent request, so a request that
    /// carries no usable `virtual_env` (shell ran `deactivate`, or the value is
    /// not a venv dir) must clear it for background refreshes.
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

    /// #624: a synchronous language request must repaint other live shells.
    ///
    /// Exactly like `git_handler`'s equivalent synchronous path
    /// (`publish_and_repaint_notifies_only_on_content_change`). The requesting
    /// shell already receives the fresh detection in its reply; every *other*
    /// registered shell in the repo is still showing the instant-cache content
    /// the write just replaced. The change gate in `notify_if_changed` keeps a
    /// no-op refresh silent (#145/#146), so a second identical request must not
    /// add another notification.
    #[test]
    fn handle_language_detection_notifies_only_on_content_change() {
        let (handler, registry) = make_handler();
        // A unique temp path keeps the instant-cache key (and on-disk file)
        // isolated from other runs so the first write is always a change.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_dir.join(".git")).expect("git dir");
        // `handle_language_detection` resolves the repo root via
        // `MultiRepoWatcher::find_git_root`, which canonicalizes; the cache key
        // used below must match that canonical form.
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

        // First request writes fresh content -> repaint.
        handler.handle(&message).expect("first request");
        assert_eq!(
            registry.notify_invocations(),
            1,
            "a synchronous request that changes the cached output should repaint live shells"
        );

        // Second, identical request -> no repaint (change-gated).
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

    /// #709: a stale non-git cache hit re-detects in the background.
    ///
    /// Nothing watches a non-git directory, so the hit must still be answered
    /// from the cache without waiting, or new marker files are never noticed.
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

    /// #709: a stale git-root hit starts no detection job.
    ///
    /// Git roots are covered by the watcher and the signature-throttled refresh.
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

    /// #727: a non-git subdirectory reports its project's languages.
    ///
    /// Detection (and its cache) runs at the nearest marker ancestor, while the
    /// instant cache stays keyed by the request path the shells read.
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

        // The synchronous reply's render must land in the request path's
        // entries (key ends `docs`, after the escaped path separator: `_s`
        // for `/`, `_b` for the `\` of a native Windows path), not only in
        // the project root's.
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
}
