//! Instant prompt caching for 0ms perceived latency
//!
//! This module provides instant prompt caching that enables shells to display prompts
//! immediately (0ms) by reading cached formatted prompts from disk, while the daemon
//! keeps those caches fresh via live file watching.
//!
//! ## Architecture
//!
//! When the daemon detects repository changes:
//! 1. Refresh git status (existing)
//! 2. Update `GitStatusCache` (existing)
//! 3. **Write instant-prompt cache files** (this module)
//! 4. Send SIGUSR1/2 to shells (existing)
//!
//! When shells need a prompt:
//! 1. Read instant-prompt cache file (0ms) ✨
//! 2. Display cached prompt immediately
//! 3. User can start typing
//! 4. (Fish/Zsh only) Repaint on signal when cache updates
//!
//! ## Cache Files
//!
//! Prompts are cached in `~/.cache/gpy/instant-prompts/{hash}.{suffix}.{token}.ansi`:
//! - `{hash}`: encoded repository root path (see [`path_to_cache_key`])
//! - `{suffix}`: `git`, `git_last`, `lang`, or `lang_last`
//! - `{token}`: previous-segment background the entry was rendered with (see
//!   [`prev_bg_token`]); `none` for context-free writers. Because the rendered
//!   ANSI bakes in the `fg:prev_bg` opening chevron, the token is part of the key
//!   so a render is only served to the context it was produced for.
//! - Format: ANSI escape codes (works across Fish, Zsh, Bash)
//!
//! Example:
//! ```text
//! ~/.cache/gpy/instant-prompts/
//!   ├── a1b2c3d4.git.none.ansi       # Git status, no prev_bg context
//!   ├── a1b2c3d4.git.blue.ansi       # Git status, prev segment bg = blue
//!   ├── a1b2c3d4.lang.none.ansi      # Language segment (non-last)
//!   ├── a1b2c3d4.lang_last.none.ansi # Language segment (last)
//!   └── ...
//! ```
//!
//! ## Performance
//!
//! - **Write cost**: ~1ms per update (1 small file)
//! - **Read cost**: ~0.1ms (simple cat from cache)
//! - **Perceived latency**: 400ms → **0ms** ✨
//! - **Freshness**: 0-2 seconds (updated by daemon on file changes)

use crate::cache::bounded::INSTANT_PROMPT_LAST_WRITTEN_CAPACITY;
use crate::config::Config;
use crate::formatter::{Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter};
use crate::git::RepositoryStatus;
use crate::ipc::Response;
use crate::template::{Color, Palette, parse_color};
use crate::theme::ThemeConfig;
use crate::{Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, SystemTime};

/// Per-process counter appended to temp-file names, alongside the pid.
///
/// This ensures concurrent writers (watcher flush thread, reconcile, IPC
/// jobs) racing to write the same cache key never share one temp path —
/// sharing one would let a slower writer's rename clobber a faster one's
/// still-being-written temp file, landing a partially-written or
/// wrong-generation file at the final path (#323).
///
/// `pub(super)` so the sibling `theme_export` module (same `cache` parent) can
/// reuse it for its own atomic temp-file naming, without exposing it outside
/// the `cache` module tree.
pub(super) fn next_tmp_suffix() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Write `content` to `dir/name` atomically.
///
/// Writes to a temp file in the same directory, then renames. `rename(2)` is
/// atomic on the same filesystem, so a reader always sees either the complete
/// old content or the complete new content -- never a partial write, even if
/// the process is killed between the write and the rename (the target path
/// is untouched until the rename succeeds; at worst a stray `.tmp-` file is
/// left in `dir`, never a half-written `name`).
///
/// `pub(super)` so the sibling `theme_export` module (same `cache` parent)
/// shares this instead of carrying its own copy of the same sequence (#591).
///
/// # Errors
///
/// Returns an error if the write or the rename fails. On either failure the
/// temp file is removed on a best-effort basis (its own removal failure is
/// not itself an error -- the write/rename failure is what's reported).
pub(super) fn write_atomic(dir: &Path, name: &str, content: &str) -> Result<()> {
    let target = dir.join(name);
    let tmp_path = dir.join(format!(
        "{name}.tmp-{}-{}",
        std::process::id(),
        next_tmp_suffix()
    ));

    if let Err(e) = std::fs::write(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    std::fs::rename(&tmp_path, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::from(e)
    })
}

/// Maximum distinct `prev_bg` render contexts tracked per repository.
///
/// The powerline theme produces only a handful (one per segment background), so
/// this bounds the per-key set far above any realistic count while capping memory.
const MAX_PREV_BG_CONTEXTS_PER_KEY: usize = 16;

/// Per-repository insertion-ordered set of `prev_bg` render contexts (`None` =
/// context-free). Keyed by cache key. See [`InstantPromptCache::seen_contexts`].
type SeenContexts = HashMap<String, Vec<Option<String>>>;

/// Manager for instant-prompt cache files
pub struct InstantPromptCache {
    cache_dir: PathBuf,
    /// In-memory record of the content last written for each `{cache_key}:{suffix}`,
    /// used to skip a redundant write when nothing changed. A pure performance
    /// optimization: a lost or stale read/record here means at most an
    /// occasional redundant write, never incorrect cache content.
    ///
    /// Poison policy (#591): every access recovers via
    /// `unwrap_or_else(PoisonError::into_inner)` rather than silently skipping
    /// the dedup check/record on a poisoned lock (the prior behavior). This
    /// matches `template::parse`'s convention elsewhere in this crate -- a
    /// writer panicking mid-update should not also disable every subsequent
    /// cache interaction on this table.
    last_written: RwLock<HashMap<String, String>>,
    /// Per-repository set of `prev_bg` render contexts seen so far (`None` =
    /// context-free). The rendered ANSI bakes in the `fg:prev_bg` chevron, so a
    /// background write (watcher/registration, which has no `prev_bg`) must refresh
    /// **every** known context — otherwise a live shell pinned to a specific
    /// context file would show stale git/language status until its TTL expires,
    /// defeating the instant live-update path (#145/#160). Keyed by cache key; the
    /// `Vec` preserves insertion order so eviction drops the oldest context.
    seen_contexts: RwLock<SeenContexts>,
    /// Test-only count of [`Self::write_cache_file`] invocations, incremented
    /// regardless of whether the write actually changed content on disk. Lets
    /// tests assert exactly how many low-level writes one request triggers
    /// (#570: the language instant cache was being written twice per request).
    #[cfg(test)]
    write_calls: AtomicU64,
}

impl InstantPromptCache {
    /// Create a new instant-prompt cache manager
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created
    pub fn new() -> Result<Self> {
        let cache_dir = get_instant_cache_dir()?;
        std::fs::create_dir_all(&cache_dir)?;
        // Nothing else ever removes entries from this directory, so without a
        // sweep it grows without bound for the life of the machine. Two things
        // accumulate: entries keyed by directories that no longer exist (every
        // test run that reaches a real cache dir leaves a set keyed by its temp
        // repo), and `.tmp-<pid>` files orphaned when a process is killed
        // between `write_atomic`'s write and its rename. Best-effort: a cache
        // that cannot be tidied must not stop the agent from starting.
        let _ = prune_stale_entries(&cache_dir, SystemTime::now());
        Ok(Self {
            cache_dir,
            last_written: RwLock::new(HashMap::new()),
            seen_contexts: RwLock::new(HashMap::new()),
            #[cfg(test)]
            write_calls: AtomicU64::new(0),
        })
    }

    /// A cache rooted in a fresh temporary directory, for tests.
    ///
    /// Prefer this over [`InstantPromptCache::new`] in tests. `new` resolves a
    /// real user cache directory from the environment, so a test using it
    /// writes into the developer's `~/.cache/gpy` and fails outright wherever
    /// neither `XDG_CACHE_HOME` nor `HOME` is set -- which is every Windows
    /// runner, and was the entire cause of 36 failing lib tests there (#474).
    /// `std::env::temp_dir()` needs neither variable on any platform.
    ///
    /// Each call gets its own directory, keyed by pid and a counter, so tests
    /// running concurrently cannot interfere with each other. The directory is
    /// deliberately left behind rather than guarded by a `TempDir`: several
    /// callers build a cache inside a helper that has nowhere to keep a guard,
    /// and a dropped guard would delete the directory out from under them.
    ///
    /// Not `#[cfg(test)]`: `EndpointHandle::new_test_handle` is a
    /// `#[doc(hidden)]` helper compiled into the real lib (integration tests in
    /// `tests/` link the crate normally and cannot see `cfg(test)` items), and
    /// it needs a hermetic cache too.
    ///
    /// # Panics
    ///
    /// Panics if the directory cannot be created, which in a test means the
    /// environment is unusable.
    #[doc(hidden)]
    #[expect(
        clippy::expect_used,
        reason = "test-only helper returning Self (not Result); a failed temp-dir create means the test environment itself is unusable, so panicking with a descriptive message is the sanctioned pattern here, not a shortcut"
    )]
    pub(crate) fn new_for_test() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let cache_dir = std::env::temp_dir().join(format!(
            "gpy-test-instant-cache-{}-{unique}",
            std::process::id()
        ));
        Self::new_in_dir(cache_dir).expect("instant cache in a temp directory")
    }

    /// Ungated for the same reason as [`Self::new_for_test`]: it backs that
    /// helper, which the lib-compiled `new_test_handle` calls.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    #[doc(hidden)]
    pub(crate) fn new_in_dir(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            cache_dir,
            last_written: RwLock::new(HashMap::new()),
            seen_contexts: RwLock::new(HashMap::new()),
            #[cfg(test)]
            write_calls: AtomicU64::new(0),
        })
    }

    /// Number of times [`Self::write_cache_file`] has actually run (test-only).
    ///
    /// Counts every invocation regardless of whether the write changed
    /// content on disk, mirroring `ClientDirectory::notify_invocations`'s
    /// idiom for exercising an internal counter from tests (#570).
    #[cfg(test)]
    pub(crate) fn write_call_count(&self) -> u64 {
        self.write_calls.load(Ordering::Relaxed)
    }

    /// Write instant-prompt cache with ANSI format (shell-agnostic)
    ///
    /// Renders the git status as a prompt using ANSI escape codes, then writes
    /// it to a cache file that any shell can read instantly.
    ///
    /// Writes all four `is_last`/`is_first` combinations (`git`, `git_last`,
    /// `git_first`, `git_first_last`) regardless of the requesting client's own
    /// position, mirroring the existing `is_last`-only behavior: the writer
    /// refreshes every variant a shell might read, and the *reading* shell picks
    /// the file matching its own position (#401).
    ///
    /// Returns `true` when any rendered variant was actually written (the
    /// content differs from the last write, or the file was missing), so callers
    /// serving stale prompts can fire a repaint signal only when the visible
    /// output changed. Returns `false` when every variant was already current.
    ///
    /// # Errors
    ///
    /// Returns an error if prompt rendering or file writing fails
    ///
    /// `prev_bg` is the wire-format color string the requesting shell sent for the
    /// opening powerline chevron (`None` for context-free writers: watcher,
    /// registration, background jobs). It is the single source of truth for both
    /// the rendered color *and* the cache-key token, so the shell that later reads
    /// the cache resolves the same file from the same string (see [`prev_bg_token`]).
    #[expect(
        clippy::too_many_arguments,
        reason = "path + status + config + theme + prev_bg + palette; a struct would add construction overhead without clarity"
    )]
    pub fn write_git(
        &self,
        path: &Path,
        status: &RepositoryStatus,
        config: &Config,
        theme: &ThemeConfig,
        prev_bg: Option<&str>,
        palette: &Palette,
    ) -> Result<bool> {
        let project_root = find_project_root(path);
        let key = path_to_cache_key(&project_root);

        // Refresh every known render context, not just the requested one: a
        // background writer (watcher/registration) passes `prev_bg = None` but must
        // keep context-specific cache files fresh too, or a live shell pinned to one
        // would serve stale status (#145/#160).
        let mut wrote_any = false;
        for ctx_prev_bg in self.known_contexts(&key, prev_bg) {
            let token = prev_bg_token(ctx_prev_bg.as_deref());
            let color = ctx_prev_bg.as_deref().and_then(|s| parse_color(s).ok());

            for pos in SEGMENT_POSITIONS {
                let prompt = render_git_prompt(status, config, theme, pos, color.clone(), palette)?;
                let suffix = variant_suffix("git", pos.is_last, pos.is_first);
                wrote_any |= self.write_cache_file(&key, &format!("{suffix}.{token}"), &prompt)?;
            }
        }
        Ok(wrote_any)
    }

    /// Write instant-prompt cache for language segment.
    ///
    /// Returns `true` when the cache file was written (content changed since the
    /// last write), mirroring the contract of [`write_git`](Self::write_git).
    ///
    /// `pub(crate)`, not `pub`: [`write_language_variants`](Self::write_language_variants)
    /// is the intended external entry point (it writes every combination this
    /// crate ever reads back); this one-variant primitive stays internal so
    /// its [`SegmentPosition`] parameter doesn't have to be constructible
    /// from outside the crate.
    ///
    /// # Errors
    ///
    /// Returns an error if prompt rendering or file writing fails
    #[expect(
        clippy::too_many_arguments,
        reason = "8 params: path + languages + config + theme + pos + prev_bg + palette + virtual_env; each is an independent piece of render context"
    )]
    pub(crate) fn write_language(
        &self,
        path: &Path,
        languages: &[crate::language::DetectedLanguage],
        config: &Config,
        theme: &ThemeConfig,
        pos: SegmentPosition,
        prev_bg: Option<&str>,
        palette: &Palette,
        virtual_env: Option<&Path>,
    ) -> Result<bool> {
        let project_root = find_project_root(path);
        let key = path_to_cache_key(&project_root);
        let base = variant_suffix("lang", pos.is_last, pos.is_first);

        // Refresh every known render context (see `write_git` for the rationale):
        // background language writes carry no `prev_bg` but must keep context files fresh.
        let mut wrote_any = false;
        for ctx_prev_bg in self.known_contexts(&key, prev_bg) {
            let token = prev_bg_token(ctx_prev_bg.as_deref());
            let color = ctx_prev_bg.as_deref().and_then(|s| parse_color(s).ok());
            let prompt = render_language_prompt(
                languages,
                config,
                theme,
                pos,
                &project_root,
                color,
                palette,
                virtual_env,
            )?;
            wrote_any |= self.write_cache_file(&key, &format!("{base}.{token}"), &prompt)?;
        }
        Ok(wrote_any)
    }

    /// Write all four language cache variants (`is_last` × `is_first`).
    ///
    /// Returns `true` when any variant was written (content changed), so callers
    /// serving stale prompts can decide whether a repaint signal is warranted —
    /// mirroring the contract of [`write_git`](Self::write_git). Writes every
    /// combination regardless of the requesting client's own position (#401),
    /// the same "refresh everything, let the reader pick" pattern `write_git`
    /// already uses for `is_last`.
    ///
    /// # Errors
    ///
    /// Returns an error if prompt rendering or file writing fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "path + languages + config + theme + prev_bg + palette + virtual_env; a struct would add construction overhead without clarity"
    )]
    pub fn write_language_variants(
        &self,
        path: &Path,
        languages: &[crate::language::DetectedLanguage],
        config: &Config,
        theme: &ThemeConfig,
        prev_bg: Option<&str>,
        palette: &Palette,
        virtual_env: Option<&Path>,
    ) -> Result<bool> {
        let mut wrote_any = false;
        for pos in SEGMENT_POSITIONS {
            wrote_any |= self.write_language(
                path,
                languages,
                config,
                theme,
                pos,
                prev_bg,
                palette,
                virtual_env,
            )?;
        }
        Ok(wrote_any)
    }

    /// Record `prev_bg` as a render context seen for `key` and return every known
    /// context for it (the requested one included).
    ///
    /// Background writers (watcher, registration) call with `prev_bg = None`; they
    /// still get back the full set so they refresh every context-specific cache file
    /// a live shell might be reading. New contexts are appended; once the per-key cap
    /// is reached the oldest is evicted (its cache files simply age out). On lock
    /// poisoning this degrades to the single requested context rather than failing.
    fn known_contexts(&self, key: &str, prev_bg: Option<&str>) -> Vec<Option<String>> {
        let requested = prev_bg.map(str::to_owned);
        let Ok(mut guard) = self.seen_contexts.write() else {
            return vec![requested];
        };

        // Bound the number of repositories tracked, mirroring `write_cache_file`.
        if !guard.contains_key(key)
            && guard.len() >= INSTANT_PROMPT_LAST_WRITTEN_CAPACITY
            && let Some(victim) = guard.keys().next().cloned()
        {
            guard.remove(&victim);
        }

        let contexts = guard.entry(key.to_owned()).or_default();
        if !contexts.contains(&requested) {
            if contexts.len() >= MAX_PREV_BG_CONTEXTS_PER_KEY {
                contexts.remove(0_usize);
            }
            contexts.push(requested);
        }
        contexts.clone()
    }

    /// Remove rendered language prompt cache files.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be scanned or a matching
    /// cache file cannot be removed.
    pub fn clear_language_files(&self) -> Result<()> {
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let path = entry?.path();
            let Some(file_name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };

            // Cache files are named `{key}.{base}.{token}.ansi`. Neither the base
            // (`lang`/`lang_last`/...) nor the token contains a dot, so stripping
            // `.ansi`, then the trailing `.{token}`, then the trailing `.{base}`
            // isolates the base for an exact match (the key may itself contain dots).
            if let Some(stem) = file_name.strip_suffix(".ansi")
                && let Some((without_token, _token)) = stem.rsplit_once('.')
                && let Some((_key, base)) = without_token.rsplit_once('.')
                && (base == "lang" || base == "lang_last")
            {
                std::fs::remove_file(&path)?;
            }
        }

        // Poison policy (#591, see `last_written`'s doc comment): recover
        // rather than silently skip.
        self.last_written
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|map_key, _content| {
                // map_key is `{cache_key}:{base}.{token}`; the cache key has no
                // colon (path separators are escaped), so split on the first one.
                let suffix = map_key.split_once(':').map_or("", |(_key, s)| s);
                !(suffix.starts_with("lang.") || suffix.starts_with("lang_last."))
            });

        Ok(())
    }

    /// Write a single cache file atomically, skipping the write when the content
    /// is already current.
    ///
    /// Returns `true` when the file was written (content changed since the last
    /// write, or the file was missing on disk) and `false` when the existing file
    /// already held identical content. The boolean lets callers serving stale
    /// prompts decide whether a repaint signal is warranted.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache file cannot be written.
    fn write_cache_file(&self, key: &str, suffix: &str, content: &str) -> Result<bool> {
        #[cfg(test)]
        self.write_calls.fetch_add(1, Ordering::Relaxed);

        let map_key = format!("{key}:{suffix}");
        let cache_file = self.cache_file_path(key, suffix);

        // Poison policy (#591, see `last_written`'s doc comment): recover via
        // `into_inner` rather than silently skip the dedup check on a
        // poisoned lock. Scoped to a block so the read guard is dropped
        // before the write guard is taken below.
        {
            let cache = self
                .last_written
                .read()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(cached_content) = cache.get(&map_key)
                && cached_content == content
                && cache_file.exists()
            {
                return Ok(false);
            }
        }

        write_atomic(&self.cache_dir, &format!("{key}.{suffix}.ansi"), content)?;

        // Same poison policy as the read above: recover, don't skip the record.
        let mut cache = self
            .last_written
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        cache.insert(map_key, content.to_owned());
        // Evict arbitrary entries to keep the in-memory dedup table bounded.
        // The map is keyed by "{cache_key}:{suffix}" so the number of entries
        // scales with distinct (repo, variant) pairs. Evict by draining to
        // the capacity limit using retain (stable-order not required here).
        if cache.len() > INSTANT_PROMPT_LAST_WRITTEN_CAPACITY {
            let excess = cache
                .len()
                .saturating_sub(INSTANT_PROMPT_LAST_WRITTEN_CAPACITY);
            let to_remove: Vec<String> = cache.keys().take(excess).cloned().collect();
            for k in to_remove {
                cache.remove(&k);
            }
        }
        drop(cache);

        Ok(true)
    }

    /// Get the cache file path for a given cache key and suffix
    fn cache_file_path(&self, key: &str, suffix: &str) -> PathBuf {
        self.cache_dir.join(format!("{key}.{suffix}.ansi"))
    }

    /// Get the cache file path that a shell should read for a given directory
    ///
    /// This is the public API for shells to determine which cache file to read.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be determined.
    pub fn cache_file_for_dir(dir: &Path, suffix: &str, prev_bg: Option<&str>) -> Result<PathBuf> {
        let cache_dir = get_instant_cache_dir()?;
        let key = path_to_cache_key(dir);
        let token = prev_bg_token(prev_bg);
        Ok(cache_dir.join(format!("{key}.{suffix}.{token}.ansi")))
    }
}

/// All four segment positions, in one fixed order.
///
/// Shared by every site that renders and caches all four variants of a
/// segment (#586) -- each such site previously re-derived the combination
/// with its own pair of nested loops.
const SEGMENT_POSITIONS: [SegmentPosition; 4] = [
    SegmentPosition::new(IsLast::No, IsFirst::No),
    SegmentPosition::new(IsLast::No, IsFirst::Yes),
    SegmentPosition::new(IsLast::Yes, IsFirst::No),
    SegmentPosition::new(IsLast::Yes, IsFirst::Yes),
];

/// Cache-filename suffix for a given `(is_last, is_first)` combination, e.g.
/// `variant_suffix("git", true, false)` -> `"git_last"`.
///
/// `write_git`/`write_language_variants` write all four combinations
/// unconditionally (mirroring the pre-#401 `is_last`-only behavior), and the
/// reading shell (`__gpy_read_instant_cache` in Fish/Bash/Zsh) picks the file
/// matching its own position. The Fish/Bash/Zsh implementations MUST derive
/// the same four suffixes from the same rule.
fn variant_suffix(base: &str, is_last: IsLast, is_first: IsFirst) -> String {
    match (is_first, is_last) {
        (IsFirst::No, IsLast::No) => base.to_owned(),
        (IsFirst::No, IsLast::Yes) => format!("{base}_last"),
        (IsFirst::Yes, IsLast::No) => format!("{base}_first"),
        (IsFirst::Yes, IsLast::Yes) => format!("{base}_first_last"),
    }
}

/// Render a git status as a formatted prompt string
///
/// # Errors
///
/// Returns an error if prompt rendering fails.
#[expect(
    clippy::too_many_arguments,
    reason = "6 params: status + config + theme + pos + prev_bg + palette; a struct would add construction overhead without clarity"
)]
fn render_git_prompt(
    status: &RepositoryStatus,
    config: &Config,
    theme: &ThemeConfig,
    pos: SegmentPosition,
    prev_bg: Option<Color>,
    palette: &Palette,
) -> Result<String> {
    let ctx = RenderContext::new(config, theme, pos)
        .with_palette(palette.clone())
        .with_prev_colors(None, prev_bg);

    let response_status = if config.git.show_upstream {
        status.clone()
    } else {
        status.clone().without_upstream()
    };

    let response = Response::RepositoryStatus(response_status);

    let formatter = create_formatter(Format::Ansi)?;
    formatter.render(&response, &ctx)
}

/// # Errors
///
/// Returns an error if prompt rendering fails.
#[expect(
    clippy::too_many_arguments,
    reason = "7 params: languages + config + theme + pos + root + prev_bg + palette + virtual_env; each is an independent piece of render context, and a struct would add construction overhead without clarity"
)]
fn render_language_prompt(
    languages: &[crate::language::DetectedLanguage],
    config: &Config,
    theme: &ThemeConfig,
    pos: SegmentPosition,
    root: &Path,
    prev_bg: Option<Color>,
    palette: &Palette,
    virtual_env: Option<&Path>,
) -> Result<String> {
    let ctx = RenderContext::new(config, theme, pos)
        .with_palette(palette.clone())
        .with_prev_colors(None, prev_bg);

    // Convert DetectedLanguage to language info for rendering
    let langs = crate::language::display::build_language_display_info_at(
        languages,
        theme,
        &config.language,
        Some(root),
        virtual_env,
    );

    let response = Response::Language { languages: langs };
    let formatter = create_formatter(Format::Ansi)?;
    formatter.render(&response, &ctx)
}

/// Get the GPY base cache directory (`~/.cache/gpy/` or XDG equivalent; on
/// native Windows, `%LOCALAPPDATA%\gpy`, see #478).
///
/// # Errors
///
/// Returns an error if neither `XDG_CACHE_HOME` nor the platform's fallback
/// variable (`HOME` on Unix, `LOCALAPPDATA` on Windows) is set.
///
/// `pub(super)` so the sibling `theme_export` module (same `cache` parent) can
/// resolve the same base directory for its own cache file, without exposing
/// this helper outside the `cache` module tree.
///
/// `LOCALAPPDATA` is read only on an actual `#[cfg(windows)]` compilation
/// target, never as a runtime probe: under WSL interop `LOCALAPPDATA` can be
/// inherited holding a Windows-style path like `C:\Users\x\AppData\Local`,
/// and this Unix binary would otherwise `create_dir_all` a literal relative
/// directory of that name in its CWD (#478).
pub(super) fn get_gpy_cache_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    let (os, home, local_app_data): (crate::paths::Os, Option<String>, Option<String>) = (
        crate::paths::Os::Windows,
        None,
        std::env::var("LOCALAPPDATA").ok(),
    );
    #[cfg(not(windows))]
    let (os, home, local_app_data): (crate::paths::Os, Option<String>, Option<String>) =
        (crate::paths::Os::Unix, crate::paths::home_dir(), None);

    crate::paths::cache_root_for(
        std::env::var("XDG_CACHE_HOME").ok().as_deref(),
        home.as_deref(),
        local_app_data.as_deref(),
        os,
    )
    .ok_or_else(|| Error::config("No cache directory found (XDG_CACHE_HOME or HOME not set)"))
}

/// Get the instant-prompt cache directory
///
/// Returns `~/.cache/gpy/instant-prompts/` (or XDG equivalent)
///
/// # Errors
///
/// Returns an error if `XDG_CACHE_HOME` or `HOME` environment variables are not set.
fn get_instant_cache_dir() -> Result<PathBuf> {
    get_gpy_cache_dir().map(|d| d.join("instant-prompts"))
}

/// How long an unused cache entry is kept before the startup sweep removes it.
///
/// Entries are pure derived data — a miss costs one render, not correctness —
/// so this only has to be long enough that a directory someone returns to
/// after a holiday still hits warm.
/// Clippy suggests `Duration::from_days`/`from_hours` here, but both are
/// still behind the unstable `duration_constructors` feature, so the
/// arithmetic form is the only one that builds on stable.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
const CACHE_ENTRY_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// How long a leftover `.tmp-*` file is kept before it is treated as orphaned.
///
/// `write_atomic` removes its own temp file on either failure path, so one
/// surviving this long means the writing process died between the write and
/// the rename. An hour is far beyond any in-flight write while still leaving
/// a concurrently-running agent's temp files alone.
/// Clippy suggests `Duration::from_days`/`from_hours` here, but both are
/// still behind the unstable `duration_constructors` feature, so the
/// arithmetic form is the only one that builds on stable.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
const ORPHAN_TMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Remove stale cache entries and orphaned temp files from `dir`.
///
/// Returns `(entries_removed, orphans_removed)`. Errors reading the directory
/// or any individual entry are swallowed: this is opportunistic tidying on the
/// startup path, and a permission problem on one file must not abort the sweep
/// or fail agent startup.
///
/// `now` is injected rather than read here so tests can age files deterministically
/// without sleeping.
fn prune_stale_entries(dir: &Path, now: SystemTime) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };

    let mut removed_entries = 0_u64;
    let mut removed_orphans = 0_u64;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            // Modified in the future (clock skew, or a file being written right
            // now). Leave it alone rather than guess.
            continue;
        };

        let is_orphan_tmp = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".tmp-"));

        let (limit, counter) = if is_orphan_tmp {
            (ORPHAN_TMP_MAX_AGE, &mut removed_orphans)
        } else {
            (CACHE_ENTRY_MAX_AGE, &mut removed_entries)
        };

        if age > limit && std::fs::remove_file(&path).is_ok() {
            *counter = counter.saturating_add(1);
        }
    }

    (removed_entries, removed_orphans)
}

/// Filesystem-safe token identifying the previous-segment background a cache
/// entry was rendered with.
///
/// The rendered ANSI bakes in the opening powerline chevron, whose color is
/// `fg:prev_bg`. A cache entry is therefore only valid for the `prev_bg` it was
/// rendered with, so the token becomes part of the cache filename
/// (`{key}.{suffix}.{token}.ansi`). `None`/empty → `"none"` (context-free writers:
/// watcher, registration, background jobs). Otherwise every byte that is not
/// ASCII-alphanumeric is replaced with `_`.
///
/// The shells derive the SAME token from the SAME wire string they sent as
/// `prev_bg`, so both sides resolve the identical cache file without a subprocess.
/// The Fish/Bash/Zsh implementations (`__gpy_prev_bg_token`) MUST stay in lockstep
/// with this rule. Tokens never contain a `.`, which `clear_language_files` relies
/// on to parse `{key}.{base}.{token}.ansi` filenames.
fn prev_bg_token(prev_bg: Option<&str>) -> String {
    match prev_bg {
        Some(s) if !s.is_empty() => s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
        _ => "none".to_owned(),
    }
}

/// Above this many characters, the `Os::Windows` arm of
/// [`path_to_cache_key_for`] truncates the escaped key and appends a
/// hash-tail (see that function) instead of returning it whole.
///
/// Sized against `cache_file_for_dir`'s actual filename shape,
/// `"{key}.{suffix}.{token}.ansi"`: the longest `suffix` in this codebase is
/// `variant_suffix`'s `"lang_first_last"` (16 chars), and the extension plus
/// its three separator dots add another 9 characters on top of that. Add the
/// hash-tail overhead (`"_h"` plus 16 hex digits, 18 characters) to a
/// 200-char truncated prefix and the key alone tops out at 218, leaving
/// roughly 37 characters of headroom under Windows' 255-char
/// filename-component limit for the rest of the filename (`suffix`, dots,
/// `token`, and `ansi`) -- comfortably covering the 16-char `suffix` and
/// its `ansi`/dots overhead, with room left over for realistic `prev_bg`
/// tokens.
const WINDOWS_CACHE_KEY_MAX_LEN: usize = 200;

/// Convert a path to a stable, collision-free cache key.
///
/// Uses a reversible escape scheme so that distinct paths always map to
/// distinct keys. The escape character `_` is doubled first, then each path
/// separator class is replaced with a unique `_`-prefixed token. Because every
/// literal `_` becomes `__`, a single `_` in the output is always an escape
/// introducer, which makes the mapping injective.
///
/// | Input | Token |
/// |-------|-------|
/// | `_`   | `__`  |
/// | `/`   | `_s`  |
/// | `\`   | `_b`  |
/// | `:`   | `_c`  |
/// | ` `   | `_w`  |
///
/// The order of replacements matters: `_` MUST be escaped first so the tokens
/// introduced for the separators are not re-escaped. The Fish implementation in
/// `fish/core/ipc.fish` (`__gpy_path_to_cache_key`) MUST stay byte-for-byte
/// identical so both sides resolve the same cache file without a subprocess --
/// on native Windows, though, no Fish process ever runs in this issue's
/// scoping (#478: Rust-only, no shell integration on native Windows), so that
/// contract binds only the [`crate::paths::Os::Unix`] arm this function
/// dispatches to; see [`path_to_cache_key_for`] for the Windows-specific
/// hardening on top of the same escape scheme.
///
/// Example: `/Users/foo/project` -> `_sUsers_sfoo_sproject`
fn path_to_cache_key(path: &Path) -> String {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    path_to_cache_key_for(&path.to_string_lossy(), os)
}

/// Pure core of [`path_to_cache_key`], parameterized by an explicit [`Os`].
///
/// Rather than reading `cfg!(windows)` internally, so the `Os::Windows` arm can be
/// table-tested from this Unix development machine (#478).
///
/// The `Os::Unix` arm MUST stay byte-for-byte identical to the pre-#478
/// function for every input -- including inputs that would trigger the
/// `Os::Windows` hardening below if they occurred on that arm instead. See
/// `test_path_to_cache_key_unix_characterization`, captured against the
/// unmodified function before this refactor. Nothing on native Windows runs
/// a Fish/Bash/Zsh process in this issue's scoping (#478 is Rust-only, no
/// shell integration on native Windows -- see the `paths` module docs), so
/// the `Os::Windows` hardening below never has to match
/// `fish/core/ipc.fish`'s `__gpy_path_to_cache_key`; only the `Os::Unix` arm
/// carries that byte-for-byte contract.
///
/// [`Os`]: crate::paths::Os
fn path_to_cache_key_for(path_str: &str, os: crate::paths::Os) -> String {
    // `std::fs::canonicalize` returns the `\\?\`-prefixed verbatim form on
    // Windows, so every Windows-reserved filename character (not just the
    // path separators) must be escaped here, or the resulting cache key is
    // rejected by the filesystem with `ERROR_INVALID_NAME`.
    let escaped = path_str
        .replace('_', "__")
        .replace('/', "_s")
        .replace('\\', "_b")
        .replace(':', "_c")
        .replace(' ', "_w")
        .replace('?', "_q")
        .replace('*', "_a")
        .replace('<', "_l")
        .replace('>', "_g")
        .replace('"', "_d")
        .replace('|', "_p");

    match os {
        crate::paths::Os::Unix => escaped,
        crate::paths::Os::Windows => windows_harden_cache_key(&escaped),
    }
}

/// Windows-only hardening for an already-escaped cache key.
///
/// Case-folds it (Windows filesystems are case-insensitive-but-case-preserving, so `C:\Foo`
/// and `C:\foo` are the same directory on disk and must collide to the same key -- restoring
/// the injectivity the escape scheme's doc comment claims), then, above
/// [`WINDOWS_CACHE_KEY_MAX_LEN`] characters, truncates and appends a hash-tail so two long
/// paths that share a truncated prefix still resolve to distinct keys.
///
/// Case-folding runs on the already-escaped string, not the raw path,
/// because every token the escape step introduces (`_s`, `_b`, `_c`, ...) is
/// already lowercase ASCII, so folding the whole string afterward cannot
/// perturb an escape token -- only the literal path characters it left
/// untouched.
///
/// The hash reuses this codebase's existing pattern for a stable
/// within-process fingerprint (`content_token` in
/// `ipc::server::handle`): `DefaultHasher` over the string's bytes, no new
/// dependency. The hash is taken over the *full* case-folded string (before
/// truncation) so two paths whose first `WINDOWS_CACHE_KEY_MAX_LEN`
/// characters happen to match still get different tails.
fn windows_harden_cache_key(escaped: &str) -> String {
    use std::hash::{Hash, Hasher};

    let case_folded = escaped.to_lowercase();
    if case_folded.chars().count() <= WINDOWS_CACHE_KEY_MAX_LEN {
        return case_folded;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    case_folded.as_bytes().hash(&mut hasher);
    let hash = hasher.finish();

    let truncated: String = case_folded
        .chars()
        .take(WINDOWS_CACHE_KEY_MAX_LEN)
        .collect();
    format!("{truncated}_h{hash:016x}")
}

/// Find the project root for a given path (git root or current directory).
/// This matches the logic used in shells to find the cache key.
fn find_project_root(path: &Path) -> std::path::PathBuf {
    let mut current = path;
    loop {
        let git_dir = current.join(".git");
        if git_dir.exists() {
            return std::fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use crate::language::DetectedLanguage;

    // ---- Startup sweep (unbounded-growth fix) --------------------------------

    /// Write `name` into `dir` and backdate its mtime by `age`.
    fn aged_file(dir: &Path, name: &str, age: Duration) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "x").expect("write cache file");
        set_mtime(
            &path,
            SystemTime::now()
                .checked_sub(age)
                .expect("backdating must stay within SystemTime range"),
        );
        path
    }

    fn set_mtime(path: &Path, when: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open for mtime")
            .set_modified(when)
            .expect("set mtime");
    }

    #[test]
    fn sweep_keeps_recent_entries_and_removes_ancient_ones() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let fresh = aged_file(dir.path(), "repo.git.none.ansi", Duration::from_secs(60));
        let ancient = aged_file(
            dir.path(),
            "old.git.none.ansi",
            CACHE_ENTRY_MAX_AGE.saturating_add(Duration::from_secs(60)),
        );

        let (entries, orphans) = prune_stale_entries(dir.path(), SystemTime::now());

        assert_eq!(entries, 1, "only the ancient entry should be swept");
        assert_eq!(orphans, 0);
        assert!(fresh.exists(), "a recently used entry must survive");
        assert!(
            !ancient.exists(),
            "an entry unused for weeks must be removed"
        );
    }

    #[test]
    fn sweep_removes_orphaned_temp_files_on_a_shorter_clock() {
        // write_atomic removes its own temp file on both failure paths, so one
        // that survives means the writer was killed between the write and the
        // rename -- which is what left tens of thousands behind in a real
        // cache directory.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let orphan = aged_file(
            dir.path(),
            "repo.git.none.ansi.tmp-1234-5",
            ORPHAN_TMP_MAX_AGE.saturating_add(Duration::from_secs(60)),
        );
        // Well past the orphan clock but nowhere near the entry clock, proving
        // temp files are swept on their own much shorter threshold.
        let live_entry = aged_file(
            dir.path(),
            "repo.git.none.ansi",
            ORPHAN_TMP_MAX_AGE.saturating_add(Duration::from_secs(60)),
        );

        let (entries, orphans) = prune_stale_entries(dir.path(), SystemTime::now());

        assert_eq!(orphans, 1, "the orphaned temp file should be swept");
        assert_eq!(entries, 0, "a normal entry that age must be left alone");
        assert!(!orphan.exists());
        assert!(live_entry.exists());
    }

    #[test]
    fn sweep_spares_an_in_flight_temp_file() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let in_flight = aged_file(
            dir.path(),
            "repo.git.none.ansi.tmp-99-1",
            Duration::from_secs(1),
        );

        let (_, orphans) = prune_stale_entries(dir.path(), SystemTime::now());

        assert_eq!(
            orphans, 0,
            "a concurrently running agent's in-flight write must not be deleted"
        );
        assert!(in_flight.exists());
    }

    #[test]
    fn sweep_tolerates_a_missing_directory() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let absent = dir.path().join("does-not-exist");
        assert_eq!(prune_stale_entries(&absent, SystemTime::now()), (0, 0));
    }

    #[test]
    fn sweep_ignores_files_dated_in_the_future() {
        // Clock skew on a networked filesystem must not make the sweep guess.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("skewed.git.none.ansi");
        std::fs::write(&path, "x").expect("write");
        set_mtime(
            &path,
            SystemTime::now()
                .checked_add(ORPHAN_TMP_MAX_AGE)
                .expect("future timestamp must stay within SystemTime range"),
        );

        let (entries, orphans) = prune_stale_entries(dir.path(), SystemTime::now());

        assert_eq!((entries, orphans), (0, 0));
        assert!(path.exists());
    }

    #[test]
    fn test_write_cache_file_is_atomic_no_temp_leftovers() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        cache
            .write_cache_file("repo", "git", "hello world")
            .expect("write");

        // No .tmp files should remain after a successful write
        let leftover_count = std::fs::read_dir(temp_dir.path())
            .expect("read dir")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(
            leftover_count, 0_usize,
            "stray temp files remain after atomic write"
        );

        // Content must be exact
        let content = std::fs::read_to_string(cache.cache_file_path("repo", "git")).expect("read");
        assert_eq!(content, "hello world");
    }

    /// Direct tests of [`write_atomic`] itself (#591).
    ///
    /// Rather than only through `write_cache_file`/`write_theme_export_to_dir`
    /// -- both callers share this one implementation, so its own guarantees
    /// get their own coverage.
    mod write_atomic_tests {
        use super::*;

        #[test]
        fn writes_new_content() {
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            write_atomic(temp_dir.path(), "target.txt", "hello").expect("write");

            let content =
                std::fs::read_to_string(temp_dir.path().join("target.txt")).expect("read");
            assert_eq!(content, "hello");
        }

        /// After a successful write, the target is the ONLY file in the
        /// directory -- proving the temp file was consumed by the rename
        /// (moved, not copied) rather than merely deleted after the fact.
        #[test]
        fn leaves_only_the_target_file_no_tmp_residue() {
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            write_atomic(temp_dir.path(), "target.txt", "hello").expect("write");

            let entries: Vec<String> = std::fs::read_dir(temp_dir.path())
                .expect("read dir")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                entries,
                vec!["target.txt".to_owned()],
                "only the target file should remain -- no .tmp- leftover"
            );
        }

        /// Simulates the aftermath of a real crash between a prior write's
        /// write-to-temp and its rename.
        ///
        /// A stray `.tmp-` file, holding garbage, already sits in the
        /// directory before this call. The "no partial file" guarantee is
        /// about the TARGET path -- only ever touched by `rename` -- so an
        /// unrelated leftover temp file from an earlier interrupted write
        /// must not interfere with a fresh successful write.
        #[test]
        fn ignores_a_stray_leftover_tmp_file_from_a_prior_crash() {
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            std::fs::write(
                temp_dir.path().join("target.txt.tmp-99999-1"),
                "stale garbage from a crashed writer",
            )
            .expect("seed stray tmp file");

            write_atomic(temp_dir.path(), "target.txt", "real content").expect("write");

            let content =
                std::fs::read_to_string(temp_dir.path().join("target.txt")).expect("read target");
            assert_eq!(
                content, "real content",
                "a stray leftover tmp file must not corrupt a fresh atomic write"
            );
        }

        /// Failure path: writing into a directory that doesn't exist fails
        /// cleanly (no crash-simulation trickery needed -- `fs::write` itself
        /// fails before anything touches disk) and never creates the target.
        #[test]
        fn fails_and_creates_no_target_when_dir_is_missing() {
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let missing_dir = temp_dir.path().join("does-not-exist");

            let result = write_atomic(&missing_dir, "target.txt", "content");

            assert!(result.is_err(), "write into a missing directory must fail");
            assert!(
                !missing_dir.join("target.txt").exists(),
                "target must not be created on a failed write"
            );
        }
    }

    #[test]
    fn write_cache_file_reports_content_change() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");

        // First write of new content is reported as written (changed).
        assert!(
            cache
                .write_cache_file("repo", "git", "alpha")
                .expect("write"),
            "first write of new content should report a change"
        );
        // Identical content with the file present is a no-op.
        assert!(
            !cache
                .write_cache_file("repo", "git", "alpha")
                .expect("write"),
            "rewriting identical content should report no change"
        );
        // Different content is reported as changed.
        assert!(
            cache
                .write_cache_file("repo", "git", "beta")
                .expect("write"),
            "writing different content should report a change"
        );
        // A missing file is rewritten and reported as changed even when the
        // last-written content matches, so serve-stale readers that saw nothing
        // still get a repaint.
        std::fs::remove_file(cache.cache_file_path("repo", "git")).expect("remove");
        assert!(
            cache
                .write_cache_file("repo", "git", "beta")
                .expect("write"),
            "rewriting a missing file should report a change"
        );
    }

    #[test]
    fn test_path_to_cache_key_consistency() {
        let path = Path::new("/home/user/project");
        let key1 = path_to_cache_key(path);
        let key2 = path_to_cache_key(path);
        assert_eq!(key1, key2, "Cache key should be consistent");
    }

    #[test]
    fn test_path_to_cache_key_different() {
        let path1 = Path::new("/home/user/project1");
        let path2 = Path::new("/home/user/project2");
        let key1 = path_to_cache_key(path1);
        let key2 = path_to_cache_key(path2);
        assert_ne!(
            key1, key2,
            "Different paths should have different cache keys"
        );
    }

    #[test]
    fn test_path_to_cache_key_escaping() {
        let path = Path::new("/home/user/my project");
        let key = path_to_cache_key(path);
        assert_eq!(
            key, "_shome_suser_smy_wproject",
            "Should escape separators with unique tokens"
        );
    }

    /// Distinct paths that previously collapsed to the same underscore-only key
    /// must now produce distinct keys. Regression test for cache key collisions.
    #[test]
    fn test_path_to_cache_key_no_collision_for_separator_underscore_swaps() {
        let collision_pairs = [
            ("/a/b_c", "/a_b/c"),
            ("/x/y z", "/x y/z"),
            ("/p:q/r", "/p/q:r"),
            ("/foo_bar/baz", "/foo/bar_baz"),
        ];
        for (left, right) in collision_pairs {
            let key_left = path_to_cache_key(Path::new(left));
            let key_right = path_to_cache_key(Path::new(right));
            assert_ne!(
                key_left, key_right,
                "Paths {left} and {right} must not share a cache key"
            );
        }
    }

    /// The escape scheme must be injective: the original path is recoverable
    /// from the key, which guarantees no two paths map to the same key.
    ///
    /// Pins `Os::Unix` explicitly (#487) rather than calling the
    /// platform-dispatching `path_to_cache_key`: on an actual Windows build
    /// that wrapper correctly dispatches to `Os::Windows`, whose case-fold
    /// hardening is *intentionally* non-reversible (`C:\Foo` and `C:\foo`
    /// collide to the same key by design, since they're the same directory
    /// on a case-insensitive filesystem) -- injectivity is a property of the
    /// Unix arm's plain escape scheme, not something the Windows arm ever
    /// promised.
    #[test]
    fn test_path_to_cache_key_is_reversible() {
        let decode = |key: &str| -> String {
            let mut out = String::new();
            let mut chars = key.chars();
            while let Some(c) = chars.next() {
                if c == '_' {
                    match chars.next() {
                        Some('_') => out.push('_'),
                        Some('s') => out.push('/'),
                        Some('b') => out.push('\\'),
                        Some('c') => out.push(':'),
                        Some('w') => out.push(' '),
                        other => panic!("malformed key token: _{other:?}"),
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };

        let paths = [
            "/home/user/project",
            "/a/b_c",
            "/a_b/c",
            "/path with spaces/repo",
            "/weird/__double__/under_scores",
            "C:\\Users\\foo\\bar",
            "/unicode/café/项目/repo",
            "/very/long/path/that/keeps/going/on/and/on/for/a/while/repo",
        ];
        for path in paths {
            let key = path_to_cache_key_for(path, crate::paths::Os::Unix);
            assert_eq!(decode(&key), path, "round-trip failed for {path}");
        }
    }

    /// Characterization test written BEFORE the #478 Windows-hardening refactor, against the
    /// then-unmodified `path_to_cache_key`.
    ///
    /// Captures the exact current output for a handful of representative inputs (including a
    /// long/deep path) so the post-refactor `Os::Unix` arm can be asserted byte-for-byte
    /// identical -- the Fish `__gpy_path_to_cache_key` contract this function documents means
    /// the Unix output must not move by a single byte.
    ///
    /// Pins `Os::Unix` explicitly (#487) rather than calling the
    /// platform-dispatching `path_to_cache_key`: on an actual Windows build
    /// that wrapper correctly dispatches to `Os::Windows` (case-folded), so
    /// calling it here made this test assert Unix-shaped output only by
    /// coincidence of running on a Unix CI runner -- confirmed failing when
    /// this crate is actually compiled and run on Windows.
    #[test]
    fn test_path_to_cache_key_unix_characterization() {
        let cases: [(&str, &str); 4] = [
            ("/Users/foo/project", "_sUsers_sfoo_sproject"),
            ("/home/user/my project", "_shome_suser_smy_wproject"),
            ("C:\\Users\\foo\\bar", "C_c_bUsers_bfoo_bbar"),
            (
                "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/issue/478s/implementation/work",
                "_sa_svery_sdeeply_snested_sdirectory_sstructure_sthat_sgoes_son_sand_son_sand_son_sand_son_sand_son_sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_slogic_sand_smake_ssure_snothing_schanges_sunexpectedly_swhen_sthis_sfunction_sgains_swindows_shardening_sas_spart_sof_sissue_s478s_simplementation_swork",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                path_to_cache_key_for(input, crate::paths::Os::Unix),
                expected,
                "unix arm output for {input} must stay byte-for-byte identical"
            );
        }
    }

    /// Two differently-cased paths to the same Windows directory must collide to one cache key.
    ///
    /// Windows filesystems are case-insensitive-but-case-preserving, so `C:\Foo\Bar` and
    /// `C:\foo\bar` are the same directory on disk and MUST collide to the same cache key --
    /// restoring the injectivity the escape scheme's doc comment claims. Driven with
    /// `Os::Windows` from this Unix runner per #478's table-testing requirement.
    #[test]
    fn test_path_to_cache_key_windows_case_fold_collision() {
        let left = path_to_cache_key_for(r"C:\Foo\Bar", crate::paths::Os::Windows);
        let right = path_to_cache_key_for(r"C:\foo\bar", crate::paths::Os::Windows);
        assert_eq!(
            left, right,
            "differently-cased paths to the same Windows directory must share a cache key"
        );

        // The Unix arm must NOT case-fold: it has no case-insensitive
        // filesystem to protect, and folding there would make genuinely
        // distinct Unix directories collide.
        let unix_left = path_to_cache_key_for(r"C:\Foo\Bar", crate::paths::Os::Unix);
        let unix_right = path_to_cache_key_for(r"C:\foo\bar", crate::paths::Os::Unix);
        assert_ne!(
            unix_left, unix_right,
            "the Unix arm must not case-fold -- it must stay byte-for-byte unmodified"
        );
    }

    /// Two long paths sharing a truncated prefix must still resolve to distinct cache keys.
    ///
    /// Above [`WINDOWS_CACHE_KEY_MAX_LEN`] characters, the `Os::Windows` arm truncates and
    /// appends a hash-tail rather than returning an arbitrarily long filename component
    /// (Windows' 255-char-per-component limit, #478).
    #[test]
    fn test_path_to_cache_key_windows_length_cap() {
        let long_prefix = "a".repeat(WINDOWS_CACHE_KEY_MAX_LEN + 50);
        let path_a = format!("/{long_prefix}/repo-one");
        let path_b = format!("/{long_prefix}/repo-two");

        let key_a = path_to_cache_key_for(&path_a, crate::paths::Os::Windows);
        let key_b = path_to_cache_key_for(&path_b, crate::paths::Os::Windows);

        assert!(
            key_a.chars().count() < path_a.len(),
            "an over-long Windows key must be shorter than the escaped input"
        );
        assert_ne!(
            key_a, key_b,
            "two long paths sharing a truncated prefix must not collide"
        );
        assert!(
            key_a.contains("_h") && key_b.contains("_h"),
            "an over-long Windows key must carry the hash-tail marker"
        );

        // Well under the cap: unaffected, no hash-tail, matches the folded
        // escape output exactly.
        let short = path_to_cache_key_for("/Users/foo/project", crate::paths::Os::Windows);
        assert_eq!(short, "_susers_sfoo_sproject");
    }

    /// `new()` resolves its directory from the environment, so what it should do depends on
    /// the environment it runs in.
    ///
    /// Asserting unconditional success made this test fail wherever no cache root resolves --
    /// every Windows runner (#474) -- while asserting the contract works everywhere.
    ///
    /// The expectation comes from `get_gpy_cache_dir`, the same resolver
    /// `new()` uses, rather than from a hand-written list of variable names.
    /// The old predicate read only `XDG_CACHE_HOME` and `HOME`, so when #473
    /// taught `paths::cache_root_for` to resolve `%LOCALAPPDATA%\gpy` the two
    /// drifted apart: on `windows-latest`, where neither of those is set,
    /// `new()` succeeded and this test demanded that it fail (#527). Deriving
    /// the expectation from the resolver means they cannot drift again.
    ///
    /// The environment is read rather than set: `std::env::set_var` is
    /// unavailable under `#![forbid(unsafe_code)]`.
    #[test]
    fn instant_cache_creation_follows_the_environment() {
        let resolved_root = get_gpy_cache_dir();

        let cache = InstantPromptCache::new();

        assert_eq!(
            cache.is_ok(),
            resolved_root.is_ok(),
            "new() must succeed exactly when a cache root resolves"
        );

        if let (Ok(instant_cache), Ok(root)) = (cache, resolved_root) {
            assert!(
                instant_cache.cache_dir.starts_with(&root),
                "the cache must live under the resolved root: {:?} not under {:?}",
                instant_cache.cache_dir,
                root
            );
        }
    }

    #[test]
    fn test_find_project_root_finds_git_root() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = temp_dir.path().join("repo");
        let subdir = repo_root.join("subdir/nested");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let found = find_project_root(&subdir);
        // Canonicalize both to match implementation
        let expected = std::fs::canonicalize(&repo_root).unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn test_write_language_uses_git_root_for_key() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("cache")).unwrap();

        let repo_root = temp_dir.path().join("repo");
        let subdir = repo_root.join("subdir");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let config = Config::default();
        let theme = ThemeConfig::default();
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 1,
        }];

        // Pass subdir but expect key to be based on repo_root
        cache
            .write_language(
                &subdir,
                &languages,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .unwrap();

        let expected_key = path_to_cache_key(&std::fs::canonicalize(&repo_root).unwrap());
        let expected_path = temp_dir
            .path()
            .join("cache/")
            .join(format!("{expected_key}.lang.none.ansi"));
        assert!(
            expected_path.exists(),
            "Cache file should exist at git root key path"
        );
    }

    #[test]
    fn test_language_cache_suffixes_use_repo_root() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache =
            InstantPromptCache::new_in_dir(temp_dir.path().join("instant-prompts")).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo root");

        let config = Config::default();
        let theme = ThemeConfig::default();
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 1,
        }];

        cache
            .write_language(
                &repo_root,
                &languages,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write lang");
        let key = path_to_cache_key(&repo_root);
        let lang_path = cache.cache_file_path(&key, "lang.none");
        assert!(lang_path.exists(), "lang cache file should exist");

        cache
            .write_language(
                &repo_root,
                &languages,
                &config,
                &theme,
                SegmentPosition::new(IsLast::Yes, IsFirst::No),
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write lang_last");
        let lang_last_path = cache.cache_file_path(&key, "lang_last.none");
        assert!(lang_last_path.exists(), "lang_last cache file should exist");
    }

    #[test]
    fn test_write_language_variants_rewrites_missing_files() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache =
            InstantPromptCache::new_in_dir(temp_dir.path().join("instant-prompts")).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo root");

        let config = Config::default();
        let theme = ThemeConfig::default();
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }];

        cache
            .write_language_variants(
                &repo_root,
                &languages,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write variants");

        let key = path_to_cache_key(&repo_root);
        let lang_path = cache.cache_file_path(&key, "lang.none");
        let lang_last_path = cache.cache_file_path(&key, "lang_last.none");

        std::fs::remove_file(&lang_path).expect("remove lang cache");
        std::fs::remove_file(&lang_last_path).expect("remove lang_last cache");

        cache
            .write_language_variants(
                &repo_root,
                &languages,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("rewrite variants");

        assert!(lang_path.exists(), "lang cache should be rewritten");
        assert!(
            lang_last_path.exists(),
            "lang_last cache should be rewritten"
        );
    }

    #[test]
    fn test_clear_language_files_keeps_git_cache() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache =
            InstantPromptCache::new_in_dir(temp_dir.path().join("instant-prompts")).expect("cache");

        // Use the real `{base}.{token}` suffix scheme so the clear logic is
        // exercised against the filenames the writers actually produce.
        cache
            .write_cache_file("repo", "git.none", "git status")
            .expect("write git cache");
        cache
            .write_cache_file("repo", "lang.none", "ruby 4.0.5")
            .expect("write lang cache");
        cache
            .write_cache_file("repo", "lang_last.blue", "ruby 4.0.5")
            .expect("write lang_last cache");

        cache.clear_language_files().expect("clear language caches");

        assert!(cache.cache_file_path("repo", "git.none").exists());
        assert!(!cache.cache_file_path("repo", "lang.none").exists());
        assert!(!cache.cache_file_path("repo", "lang_last.blue").exists());
    }

    #[test]
    fn test_write_git_creates_both_is_last_variants() {
        use crate::git::RepositoryStatus;
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");

        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let config = Config::default();
        let theme = ThemeConfig::default();

        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canonicalize"));
        let git_path = cache.cache_file_path(&key, "git.none");
        let git_last_path = cache.cache_file_path(&key, "git_last.none");

        assert!(git_path.exists(), "git.none.ansi must exist");
        assert!(git_last_path.exists(), "git_last.none.ansi must exist");

        let normal = std::fs::read_to_string(&git_path).expect("read git");
        let last = std::fs::read_to_string(&git_last_path).expect("read git_last");
        assert!(
            normal.is_empty() == last.is_empty(),
            "both should be consistently empty or non-empty: normal={}, last={}",
            normal.len(),
            last.len()
        );
    }

    /// #401: `write_git` also writes `is_first`-aware variants (`git_first`,
    /// `git_first_last`), and the opening powerline cap (`$sep_open`) is
    /// suppressed only in those variants.
    #[test]
    fn write_git_creates_is_first_variants_and_suppresses_opening_cap() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = clean_status("main");
        let config = Config::default();
        // Embedded builtin default theme -- its git format includes $sep_open,
        // unlike ThemeConfig::default() (hermetic, bypasses ~/.config).
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();

        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canonicalize"));
        let read = |suffix: &str| {
            std::fs::read_to_string(cache.cache_file_path(&key, suffix))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        for suffix in [
            "git.none",
            "git_last.none",
            "git_first.none",
            "git_first_last.none",
        ] {
            assert!(
                cache.cache_file_path(&key, suffix).exists(),
                "{suffix}.ansi must exist"
            );
        }

        let sep_open = '\u{e0ba}';
        assert!(
            read("git.none").contains(sep_open),
            "non-first variant must render the opening powerline cap"
        );
        assert!(
            read("git_last.none").contains(sep_open),
            "last-but-not-first variant must render the opening powerline cap"
        );
        assert!(
            !read("git_first.none").contains(sep_open),
            "is_first variant must suppress the opening powerline cap"
        );
        assert!(
            !read("git_first_last.none").contains(sep_open),
            "is_first+is_last variant must suppress the opening powerline cap"
        );
    }

    /// #401: `write_language_variants` writes all four `is_last`/`is_first`
    /// combinations, and the opening powerline cap is suppressed only in the
    /// `is_first` variants.
    #[test]
    fn write_language_variants_creates_is_first_variants_and_suppresses_opening_cap() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }];

        cache
            .write_language_variants(
                &repo_root,
                &languages,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write variants");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canonicalize"));
        let read = |suffix: &str| {
            std::fs::read_to_string(cache.cache_file_path(&key, suffix))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        for suffix in [
            "lang.none",
            "lang_last.none",
            "lang_first.none",
            "lang_first_last.none",
        ] {
            assert!(
                cache.cache_file_path(&key, suffix).exists(),
                "{suffix}.ansi must exist"
            );
        }

        let sep_open = '\u{e0ba}';
        assert!(
            read("lang.none").contains(sep_open),
            "non-first variant must render the opening powerline cap"
        );
        assert!(
            !read("lang_first.none").contains(sep_open),
            "is_first variant must suppress the opening powerline cap"
        );
        assert!(
            !read("lang_first_last.none").contains(sep_open),
            "is_first+is_last variant must suppress the opening powerline cap"
        );
    }

    /// Part A: the instant-prompt cache must contain the branch name AND ANSI escapes,
    /// confirming that the active palette flows through `render_git_prompt` and produces
    /// styled output.
    ///
    /// Uses `ThemeManager::builtin` to bypass `~/.config` (hermetic).
    #[test]
    fn git_cache_reflects_active_palette() {
        use crate::git::{RepositoryState, RepositoryStatus};

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let config = Config::default();
        // Use the embedded builtin default theme — hermetic, bypasses ~/.config.
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme_arc = theme_mgr.get();

        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme_arc,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        let rendered =
            std::fs::read_to_string(cache.cache_file_path(&key, "git.none")).expect("read cache");

        assert!(
            rendered.contains("main"),
            "cached git prompt must contain branch name: {rendered:?}"
        );
        assert!(
            rendered.contains('\u{1b}'),
            "cached git prompt must contain ANSI escape: {rendered:?}"
        );
    }

    /// Watcher-triggered cache writes pass `prev_bg = None` (no per-request color context).
    ///
    /// Verify that `write_git` still produces non-empty, ANSI-escaped output so the
    /// graceful fallback path never leaves a blank instant-prompt cache on disk.
    #[test]
    fn write_git_with_none_prev_bg_produces_valid_output() {
        use crate::git::{RepositoryState, RepositoryStatus};

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = RepositoryStatus {
            branch: "watcher-branch".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme_arc = theme_mgr.get();

        // Watcher path: prev_bg = None (no IPC request context)
        let changed = cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme_arc,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git with no prev_bg");
        assert!(changed, "first write should report a change");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        let rendered =
            std::fs::read_to_string(cache.cache_file_path(&key, "git.none")).expect("read cache");

        assert!(
            !rendered.is_empty(),
            "cache file must not be empty when prev_bg is None (graceful fallback)"
        );
        assert!(
            rendered.contains("watcher-branch"),
            "cached prompt must contain branch name: {rendered:?}"
        );
        assert!(
            rendered.contains('\u{1b}'),
            "cached prompt must contain ANSI escape even with no prev_bg: {rendered:?}"
        );
    }

    /// Part B: remapping a color used by the default git format changes the ANSI bytes.
    ///
    /// The default theme's clean state uses `clean_bg_color = "green"`.  The template
    /// engine resolves `Color::Named("green")` through the active palette before encoding
    /// an ANSI escape, so substituting an RGB value produces a different byte sequence.
    ///
    /// This is a formatter-level invariant test.  End-to-end cache-file invalidation
    /// triggered by `gpy palette use` (`config_reload` → `config_refresh_required` →
    /// SIGUSR1/SIGUSR2) is covered by the Fish integration suite.
    #[test]
    fn git_cache_changes_when_palette_redefines_used_color() {
        use crate::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
        use crate::git::{RepositoryState, RepositoryStatus};
        use crate::template::{Color, Palette};
        use std::collections::HashMap;

        let status = RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let config = Config::default();
        // Hermetic: embedded builtin default theme, no ~/.config read.
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme_arc = theme_mgr.get();

        // Palette A: empty — "green" falls back to Color::Named("green") → ANSI code 42.
        let palette_a = Palette::default();

        // Palette B: remaps "green" to a specific RGB value.
        // The default theme uses clean_bg_color = "green" for clean repositories,
        // so remapping "green" changes the background escape code for the "main" branch.
        let mut map_b: HashMap<String, Color> = HashMap::new();
        map_b.insert(
            "green".to_owned(),
            Color::Rgb {
                r: 0_u8,
                g: 200_u8,
                b: 0_u8,
            },
        );
        let palette_b = Palette::new(map_b);

        let formatter = create_formatter(Format::Ansi).expect("ansi formatter");

        let ctx_a = RenderContext::new(&config, &theme_arc, SegmentPosition::MIDDLE)
            .with_palette(palette_a);
        let ctx_b = RenderContext::new(&config, &theme_arc, SegmentPosition::MIDDLE)
            .with_palette(palette_b);

        let response_a = Response::RepositoryStatus(status.clone());
        let response_b = Response::RepositoryStatus(status);

        let output_a = formatter
            .render(&response_a, &ctx_a)
            .expect("render palette A");
        let output_b = formatter
            .render(&response_b, &ctx_b)
            .expect("render palette B");

        assert!(
            !output_a.is_empty(),
            "palette A render must be non-empty (check builtin default theme has git format)"
        );
        assert_ne!(
            output_a, output_b,
            "remapping 'green' in the palette must alter ANSI output: \
             palette_a={output_a:?}, palette_b={output_b:?}"
        );
    }

    /// The cache-key token is filesystem-safe, deterministic, and `none` by default.
    ///
    /// Context-free writers use `none`, and shells derive the same token from the
    /// same wire string, so this rule is part of the cross-shell contract.
    #[test]
    fn prev_bg_token_sanitizes_and_defaults_to_none() {
        assert_eq!(prev_bg_token(None), "none");
        assert_eq!(prev_bg_token(Some("")), "none");
        assert_eq!(prev_bg_token(Some("blue")), "blue");
        // Non-alphanumerics (hex `#`, separators) collapse to `_` so the token is
        // a safe single filename component and never contains a `.`.
        assert_eq!(prev_bg_token(Some("#5277C3")), "_5277C3");
        assert_eq!(prev_bg_token(Some("bright-green")), "bright_green");
        assert!(
            !prev_bg_token(Some("a.b")).contains('.'),
            "token must never contain a dot (clear_language_files relies on this)"
        );
    }

    fn clean_status(branch: &str) -> RepositoryStatus {
        use crate::git::RepositoryState;
        RepositoryStatus {
            branch: branch.to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    /// #222: distinct `prev_bg` contexts still produce distinct cache files.
    ///
    /// The default theme's segment caps are now self-colored on a transparent
    /// background rather than chained to `prev_bg`, so the rendered ANSI is
    /// identical across contexts today — but each context must still resolve
    /// to its own file (no cross-context clobbering) for themes that do key
    /// rendering on `prev_bg`.
    #[test]
    fn git_cache_writes_one_file_per_prev_bg_context() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = clean_status("main");
        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();

        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme,
                Some("blue"),
                &crate::palette::active_palette(&config),
            )
            .expect("write blue");
        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme,
                Some("green"),
                &crate::palette::active_palette(&config),
            )
            .expect("write green");
        cache
            .write_git(
                &repo_root,
                &status,
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write none");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        let read = |suffix: &str| {
            std::fs::read_to_string(cache.cache_file_path(&key, suffix))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        // All three context files coexist.
        assert!(cache.cache_file_path(&key, "git.blue").exists());
        assert!(cache.cache_file_path(&key, "git.green").exists());
        assert!(cache.cache_file_path(&key, "git.none").exists());

        let blue = read("git.blue");
        let green = read("git.green");
        let none = read("git.none");

        // The default theme's segment caps no longer read `prev_bg`, so the
        // rendered ANSI is identical across contexts — only the file-per-context
        // bookkeeping (asserted above) matters today.
        assert_eq!(blue, green);
        assert_eq!(blue, none);
    }

    /// #222 acceptance: a context-free (`None`) write goes to the `none` token file
    /// and must NOT overwrite a previously written context-specific file.
    #[test]
    fn git_none_write_does_not_clobber_context_file() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();

        cache
            .write_git(
                &repo_root,
                &clean_status("ctx"),
                &config,
                &theme,
                Some("blue"),
                &crate::palette::active_palette(&config),
            )
            .expect("write blue");
        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        let blue_before =
            std::fs::read_to_string(cache.cache_file_path(&key, "git.blue")).expect("read blue");

        // A watcher-style None write must land on `git.none`, leaving `git.blue` intact.
        cache
            .write_git(
                &repo_root,
                &clean_status("ctx"),
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write none");

        let blue_after =
            std::fs::read_to_string(cache.cache_file_path(&key, "git.blue")).expect("read blue");
        assert_eq!(
            blue_before, blue_after,
            "a None write must not clobber the context-specific cache file"
        );
        assert!(
            cache.cache_file_path(&key, "git.none").exists(),
            "the None write must populate its own token file"
        );
    }

    /// #222: language cache writes must also be `prev_bg`-aware (parity with git).
    ///
    /// See `git_cache_writes_one_file_per_prev_bg_context` — the default theme's
    /// segment caps no longer read `prev_bg`, so distinct contexts render
    /// identical ANSI; only the file-per-context bookkeeping matters today.
    #[test]
    fn language_cache_writes_one_file_per_prev_bg_context() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }];

        cache
            .write_language_variants(
                &repo_root,
                &languages,
                &config,
                &theme,
                Some("blue"),
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write blue");
        cache
            .write_language_variants(
                &repo_root,
                &languages,
                &config,
                &theme,
                Some("green"),
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write green");

        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        assert!(cache.cache_file_path(&key, "lang.blue").exists());
        assert!(cache.cache_file_path(&key, "lang.green").exists());

        let blue =
            std::fs::read_to_string(cache.cache_file_path(&key, "lang.blue")).expect("read blue");
        let green =
            std::fs::read_to_string(cache.cache_file_path(&key, "lang.green")).expect("read green");
        assert_eq!(blue, green);
    }

    /// #145/#160 + #222: a context-free (`None`) write refreshes every context.
    ///
    /// The watcher and registration paths issue `None` writes; they must refresh
    /// EVERY render context the cache has served. Otherwise a live shell pinned to a
    /// specific `prev_bg` variant reads a stale file until its TTL expires, silently
    /// defeating live updates.
    #[test]
    fn none_write_refreshes_all_known_contexts() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();
        let key = path_to_cache_key(&std::fs::canonicalize(&repo_root).expect("canon"));
        let read = |suffix: &str| {
            std::fs::read_to_string(cache.cache_file_path(&key, suffix))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        // Two live shells establish "blue" and "green" contexts on a clean repo.
        cache
            .write_git(
                &repo_root,
                &clean_status("main"),
                &config,
                &theme,
                Some("blue"),
                &crate::palette::active_palette(&config),
            )
            .expect("write blue");
        cache
            .write_git(
                &repo_root,
                &clean_status("main"),
                &config,
                &theme,
                Some("green"),
                &crate::palette::active_palette(&config),
            )
            .expect("write green");
        let blue_before = read("git.blue");
        let green_before = read("git.green");

        // A watcher-style None write with a *different* status (branch changed) must
        // refresh the blue and green context files too — not only `git.none`.
        cache
            .write_git(
                &repo_root,
                &clean_status("feature"),
                &config,
                &theme,
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write none");

        let blue_after = read("git.blue");
        let green_after = read("git.green");
        assert_ne!(
            blue_before, blue_after,
            "context-free write must refresh the blue context file"
        );
        assert_ne!(
            green_before, green_after,
            "context-free write must refresh the green context file"
        );
        assert!(
            blue_after.contains("feature") && green_after.contains("feature"),
            "refreshed context files must carry the new branch name"
        );
    }
}
