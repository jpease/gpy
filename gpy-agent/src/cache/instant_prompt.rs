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
//! 4. Ring the SIGURG doorbell for shells (existing)
//!
//! When shells need a prompt:
//! 1. Read instant-prompt cache file (0ms) ✨
//! 2. Display cached prompt immediately
//! 3. User can start typing
//! 4. (Fish/Zsh only) Repaint on signal when cache updates
//!
//! ## Cache Files
//!
//! Prompts are cached in `~/.cache/gpy/instant-prompts/{hash}.{suffix}.{token}.{ext}`:
//! - `{hash}`: encoded repository root path (see [`path_to_cache_key`])
//! - `{suffix}`: `git`, `git_last`, `lang`, or `lang_last`
//! - `{token}`: previous-segment background the entry was rendered with (see
//!   [`prev_bg_token`]); `none` for context-free writers. Because the rendered
//!   ANSI bakes in the `fg:prev_bg` opening chevron, the token is part of the key
//!   so a render is only served to the context it was produced for.
//! - `{ext}`: the output dialect ([`PromptDialect::cache_ext`]): `ansi` (Fish,
//!   text verbatim), `bash` (escaped for `PS1`) or `zsh` (escaped for
//!   `PROMPT`). Each shell reads only its own dialect, so a cache hit is
//!   escaped exactly like a fresh render (#677).
//!
//! Example:
//! ```text
//! ~/.cache/gpy/instant-prompts/
//!   ├── a1b2c3d4.git.none.ansi       # Git status, no prev_bg context (Fish)
//!   ├── a1b2c3d4.git.none.bash       # Same entry, bash dialect
//!   ├── a1b2c3d4.git.blue.zsh        # Git status, prev segment bg = blue, zsh
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
use crate::debug_log;
use crate::formatter::{
    FishAnsiFormatter, Formatter, IsFirst, IsLast, PromptDialect, RenderContext, SegmentPosition,
};
use crate::git::RepositoryStatus;
use crate::ipc::Response;
use crate::template::{Color, Palette, parse_color};
use crate::theme::ThemeConfig;
use crate::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
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
/// is untouched until the rename succeeds; at worst a stray `.tmp-<pid>-<n>`
/// file is left in `dir`, never a half-written `name`). The temp name does not
/// embed `name`, so any `name` that fits the filesystem limit can be written;
/// its leading dot keeps it out of shell globs and lets the startup sweep
/// recognise orphans by prefix.
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
    // Independent of `name`: a name derived from the target overflowed
    // NAME_MAX for ~240+ byte cache names (#708).
    let tmp_path = dir.join(format!(".tmp-{}-{}", std::process::id(), next_tmp_suffix()));

    if let Err(e) = std::fs::write(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    std::fs::rename(&tmp_path, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::from(e)
    })
}

/// Combine the outcomes of a multi-variant write.
///
/// A partial success counts as success (so the caller still rings for the
/// variants that changed) and the first error is logged; only a total failure
/// is returned (#708).
///
/// # Errors
///
/// Returns the first error when no variant was written.
fn settle_variant_writes(wrote_any: bool, first_err: Option<Error>) -> Result<bool> {
    match first_err {
        Some(e) if !wrote_any => Err(e),
        Some(e) => {
            debug_log!("cache", "Some cache variants were not written: {e}");
            Ok(wrote_any)
        }
        None => Ok(wrote_any),
    }
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
    /// used to skip a redundant content write when nothing changed. The dedup
    /// path still bumps the file's mtime: shells read the mtime as "last write
    /// or last verification" for their TTL check (#704).
    ///
    /// Invariant (#706): a recorded entry always equals the file's content.
    /// `write_cache_file` holds this mutex across the check, the touch, the
    /// atomic write and the record, so concurrent writers of the same file
    /// cannot leave the record saying X while disk holds Y. A missing entry
    /// means at most a redundant write.
    ///
    /// Poison policy (#591): every access recovers via
    /// `unwrap_or_else(PoisonError::into_inner)` rather than silently skipping
    /// the dedup check/record on a poisoned lock (the prior behavior). This
    /// matches `template::parse`'s convention elsewhere in this crate -- a
    /// writer panicking mid-update should not also disable every subsequent
    /// cache interaction on this table.
    last_written: Mutex<HashMap<String, String>>,
    /// Per-repository set of `prev_bg` render contexts seen so far (`None` =
    /// context-free). The rendered ANSI bakes in the `fg:prev_bg` chevron, so a
    /// background write (watcher/registration, which has no `prev_bg`) must refresh
    /// **every** known context — otherwise a live shell pinned to a specific
    /// context file would show stale git/language status until its TTL expires,
    /// defeating the instant live-update path (#145/#160). Keyed by cache key; the
    /// `Vec` preserves insertion order so eviction drops the oldest context.
    seen_contexts: RwLock<SeenContexts>,
    /// Cache keys whose final filename cannot fit `NAME_MAX` even after
    /// chunking (a flat non-ASCII key of at most [`FLAT_CACHE_KEY_MAX_CHARS`]
    /// characters but over ~230 bytes). Their writes are skipped, and this set
    /// keeps that to one `debug_log!` per root (#771).
    unwritable_keys: Mutex<HashSet<String>>,
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
            last_written: Mutex::new(HashMap::new()),
            seen_contexts: RwLock::new(HashMap::new()),
            unwritable_keys: Mutex::new(HashSet::new()),
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
    /// Not `#[cfg(test)]`, and `pub`: `EndpointHandle::new_test_handle` is a
    /// `#[doc(hidden)]` helper compiled into the real lib, and integration
    /// tests in `tests/` link the crate normally and cannot see `cfg(test)` or
    /// `pub(crate)` items; both need a hermetic cache too (#839).
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
    #[must_use]
    pub fn new_for_test() -> Self {
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
            last_written: Mutex::new(HashMap::new()),
            seen_contexts: RwLock::new(HashMap::new()),
            unwritable_keys: Mutex::new(HashSet::new()),
            #[cfg(test)]
            write_calls: AtomicU64::new(0),
        })
    }

    /// Number of times [`Self::write_cache_file`] has actually run (test-only).
    ///
    /// Counts every invocation regardless of whether the write changed
    /// content on disk, mirroring `ClientDirectory::notify_invocations`'s
    /// idiom for exercising an internal counter from tests (#570).
    #[cfg(all(test, unix))]
    pub(crate) fn write_call_count(&self) -> u64 {
        self.write_calls.load(Ordering::Relaxed)
    }

    /// Write instant-prompt cache files in every [`PromptDialect`]
    ///
    /// Renders the git status as a prompt using ANSI escape codes, once per
    /// dialect, and writes each to the cache file its shell reads instantly.
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
        let mut first_err = None;
        for ctx_prev_bg in self.known_contexts(&key, prev_bg) {
            let token = prev_bg_token(ctx_prev_bg.as_deref());
            let color = ctx_prev_bg.as_deref().and_then(|s| parse_color(s).ok());

            for pos in SEGMENT_POSITIONS {
                let prompts =
                    match render_git_prompts(status, config, theme, pos, color.clone(), palette) {
                        Ok(prompts) => prompts,
                        Err(e) => {
                            first_err.get_or_insert(e);
                            continue;
                        }
                    };
                let suffix = variant_suffix("git", pos.is_last, pos.is_first);
                let suffix_token = format!("{suffix}.{token}");
                for (dialect, prompt) in prompts {
                    match self.write_cache_file(&key, &suffix_token, dialect, &prompt) {
                        Ok(wrote) => wrote_any |= wrote,
                        Err(e) => {
                            first_err.get_or_insert(e);
                        }
                    }
                }
            }
        }
        settle_variant_writes(wrote_any, first_err)
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
        let mut first_err = None;
        for ctx_prev_bg in self.known_contexts(&key, prev_bg) {
            let token = prev_bg_token(ctx_prev_bg.as_deref());
            let color = ctx_prev_bg.as_deref().and_then(|s| parse_color(s).ok());
            let prompts = match render_language_prompts(
                languages,
                config,
                theme,
                pos,
                &project_root,
                color,
                palette,
                virtual_env,
            ) {
                Ok(prompts) => prompts,
                Err(e) => {
                    first_err.get_or_insert(e);
                    continue;
                }
            };
            let base_token = format!("{base}.{token}");
            for (dialect, prompt) in prompts {
                match self.write_cache_file(&key, &base_token, dialect, &prompt) {
                    Ok(wrote) => wrote_any |= wrote,
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                }
            }
        }
        settle_variant_writes(wrote_any, first_err)
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
        let mut first_err = None;
        for pos in SEGMENT_POSITIONS {
            match self.write_language(
                path,
                languages,
                config,
                theme,
                pos,
                prev_bg,
                palette,
                virtual_env,
            ) {
                Ok(wrote) => wrote_any |= wrote,
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        settle_variant_writes(wrote_any, first_err)
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

    /// Remove the cache files a freshly started agent cannot keep current:
    /// rendered language prompts, in every position variant (`lang`,
    /// `lang_last`, `lang_first`, `lang_first_last`), and every
    /// context-specific entry (a `{token}` other than `none`), in every dialect.
    ///
    /// The set of `prev_bg` contexts to refresh ([`Self::known_contexts`]) lives
    /// in memory, so after a restart a background write (watcher, registration)
    /// refreshes only the context-free `none` files. A surviving
    /// `{key}.git.blue.bash` would be served as fresh and never updated: an idle
    /// shell would repaint on the doorbell and still show the pre-edit status
    /// until a render found the entry stale. With the file gone the shell takes
    /// the variant-fallback path, which serves the correct `none` content and
    /// re-registers its context with the agent.
    ///
    /// Best-effort, like [`prune_stale_entries`]: an unreadable directory or
    /// an entry that cannot be removed (including `NotFound` from another
    /// agent clearing the same directory concurrently, #815) is logged and
    /// skipped, never fatal to agent startup (#773). Chunked entries (#771)
    /// are cleared too, and chunk directories left empty are removed.
    pub fn clear_unrefreshable_files(&self) {
        let bases = SEGMENT_POSITIONS.map(|pos| variant_suffix("lang", pos.is_last, pos.is_first));

        clear_unrefreshable_files_in(&self.cache_dir, &bases);

        // Poison policy (#591, see `last_written`'s doc comment): recover
        // rather than silently skip.
        self.last_written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|map_key, _content| {
                // map_key is `{cache_key}:{base}.{token}.{ext}`; the cache key has no
                // colon (path separators are escaped), so split on the first one.
                let suffix = map_key.split_once(':').map_or("", |(_key, s)| s);
                let is_language = bases.iter().any(|base| {
                    suffix
                        .strip_prefix(base.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
                });
                // `{base}.{token}.{ext}`: the token is the second-to-last
                // component.
                let token = suffix.rsplit('.').nth(1).unwrap_or("");
                !(is_language || token != NO_PREV_BG_TOKEN)
            });
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
    fn write_cache_file(
        &self,
        key: &str,
        suffix: &str,
        dialect: PromptDialect,
        content: &str,
    ) -> Result<bool> {
        #[cfg(test)]
        self.write_calls.fetch_add(1, Ordering::Relaxed);

        let ext = dialect.cache_ext();
        let map_key = format!("{key}:{suffix}.{ext}");
        let (dir, file_name) = cache_file_parts(&self.cache_dir, key, &format!("{suffix}.{ext}"));
        if file_name.len() > NAME_MAX_BYTES {
            self.note_unwritable_key(key);
            return Ok(false);
        }
        let cache_file = dir.join(&file_name);

        // Poison policy (#591, see `last_written`'s doc comment): recover via
        // `into_inner` rather than silently skip the dedup check on a
        // poisoned lock. The guard is held across check, touch, write and
        // record so the record always equals disk (#706).
        let mut cache = self
            .last_written
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(cached_content) = cache.get(&map_key)
            && cached_content == content
        {
            // Verified unchanged: refresh the mtime, which shells read as
            // "last write or last verification" (#704). A file that
            // vanished falls through to a rewrite; other touch errors are
            // ignored (the content on disk is still correct).
            match std::fs::File::options()
                .write(true)
                .open(&cache_file)
                .and_then(|f| f.set_modified(SystemTime::now()))
            {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Ok(()) | Err(_) => return Ok(false),
            }
        }

        // The directory is created at construction only, and a chunk
        // directory (#771) on first write; if something removed it since
        // (cache cleaner, `rm -rf ~/.cache/gpy`, the sweep), recreate it and
        // retry exactly once, inside the same guard so the record is never
        // ahead of disk (#707). Not done up front: it would cost a `stat` per
        // write.
        match write_atomic(&dir, &file_name, content) {
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(&dir)?;
                write_atomic(&dir, &file_name, content)?;
            }
            result => result?,
        }

        cache.insert(map_key, content.to_owned());
        // Evict arbitrary entries to keep the in-memory dedup table bounded.
        // The map is keyed by "{cache_key}:{suffix}.{ext}" so the number of entries
        // scales with distinct (repo, variant, dialect) triples. Evict by draining to
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

    /// Log, once per root, that `key`'s cache files cannot be named within
    /// `NAME_MAX` and are therefore skipped (#771).
    fn note_unwritable_key(&self, key: &str) {
        let mut seen = self
            .unwritable_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if seen.len() < INSTANT_PROMPT_LAST_WRITTEN_CAPACITY && seen.insert(key.to_owned()) {
            debug_log!(
                "cache",
                "Skipping instant cache for {key}: its filename exceeds {NAME_MAX_BYTES} bytes"
            );
        }
    }

    /// Get the cache file path for a given cache key, suffix and dialect
    /// (tests; the writer needs the directory and name separately).
    #[cfg(test)]
    fn cache_file_path(&self, key: &str, suffix: &str, dialect: PromptDialect) -> PathBuf {
        let ext = dialect.cache_ext();
        let (dir, file_name) = cache_file_parts(&self.cache_dir, key, &format!("{suffix}.{ext}"));
        dir.join(file_name)
    }

    /// Get the cache file path that a shell should read for a given directory
    ///
    /// This is the public API for shells to determine which cache file to read.
    /// `dialect` is the reading shell's output dialect (`Ansi` for Fish).
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be determined.
    pub fn cache_file_for_dir(
        dir: &Path,
        suffix: &str,
        prev_bg: Option<&str>,
        dialect: PromptDialect,
    ) -> Result<PathBuf> {
        let cache_dir = get_instant_cache_dir()?;
        let key = path_to_cache_key(dir);
        let token = prev_bg_token(prev_bg);
        let ext = dialect.cache_ext();
        let (parent, file_name) =
            cache_file_parts(&cache_dir, &key, &format!("{suffix}.{token}.{ext}"));
        Ok(parent.join(file_name))
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

/// A rendered cache entry for each [`PromptDialect`].
type DialectPrompts = Vec<(PromptDialect, String)>;

/// Render `response` once per [`PromptDialect`].
///
/// # Errors
///
/// Returns an error if prompt rendering fails.
fn render_all_dialects(response: &Response, ctx: &RenderContext<'_>) -> Result<DialectPrompts> {
    PromptDialect::ALL
        .iter()
        .map(|&dialect| {
            Ok((
                dialect,
                FishAnsiFormatter::new(dialect).render(response, ctx)?,
            ))
        })
        .collect()
}

/// Render a git status as a formatted prompt string in every dialect
///
/// # Errors
///
/// Returns an error if prompt rendering fails.
#[expect(
    clippy::too_many_arguments,
    reason = "6 params: status + config + theme + pos + prev_bg + palette; a struct would add construction overhead without clarity"
)]
fn render_git_prompts(
    status: &RepositoryStatus,
    config: &Config,
    theme: &ThemeConfig,
    pos: SegmentPosition,
    prev_bg: Option<Color>,
    palette: &Palette,
) -> Result<DialectPrompts> {
    let ctx = RenderContext::new(config, theme, pos)
        .with_palette(palette.clone())
        .with_prev_colors(None, prev_bg);

    let response_status = if config.git.show_upstream {
        status.clone()
    } else {
        status.clone().without_upstream()
    };

    let response = Response::RepositoryStatus(response_status);
    render_all_dialects(&response, &ctx)
}

/// Render the language segment in every dialect.
///
/// # Errors
///
/// Returns an error if prompt rendering fails.
#[expect(
    clippy::too_many_arguments,
    reason = "7 params: languages + config + theme + pos + root + prev_bg + palette + virtual_env; each is an independent piece of render context, and a struct would add construction overhead without clarity"
)]
fn render_language_prompts(
    languages: &[crate::language::DetectedLanguage],
    config: &Config,
    theme: &ThemeConfig,
    pos: SegmentPosition,
    root: &Path,
    prev_bg: Option<Color>,
    palette: &Palette,
    virtual_env: Option<&Path>,
) -> Result<DialectPrompts> {
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
    render_all_dialects(&response, &ctx)
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
        crate::paths::root_var("XDG_CACHE_HOME").as_deref(),
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
/// "Unused" is measured from the file's mtime, which means "last write or last
/// verification" (an unchanged re-write refreshes it, #704).
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
/// or fail agent startup. Chunk directories (#771) are swept recursively and
/// removed once empty.
///
/// `now` is injected rather than read here so tests can age files deterministically
/// without sleeping.
fn prune_stale_entries(dir: &Path, now: SystemTime) -> (u64, u64) {
    let mut removed_entries = 0_u64;
    let mut removed_orphans = 0_u64;
    prune_stale_entries_in(dir, now, &mut removed_entries, &mut removed_orphans);
    (removed_entries, removed_orphans)
}

/// One directory level of [`prune_stale_entries`].
fn prune_stale_entries_in(
    dir: &Path,
    now: SystemTime,
    removed_entries: &mut u64,
    removed_orphans: &mut u64,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if is_chunk_dir(&entry) {
            prune_stale_entries_in(&path, now, removed_entries, removed_orphans);
            // Fails, harmlessly, unless the sweep emptied it.
            let _ = std::fs::remove_dir(&path);
            continue;
        }
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
            .is_some_and(|n| n.starts_with(".tmp-"));

        let (limit, counter) = if is_orphan_tmp {
            (ORPHAN_TMP_MAX_AGE, &mut *removed_orphans)
        } else {
            (CACHE_ENTRY_MAX_AGE, &mut *removed_entries)
        };

        if age > limit && std::fs::remove_file(&path).is_ok() {
            *counter = counter.saturating_add(1);
        }
    }
}

/// Whether `entry` is a chunk directory of a long cache key (#771).
///
/// Every chunk but the last is exactly [`CACHE_KEY_CHUNK_CHARS`] characters
/// and becomes a directory; nothing else in the cache directory is a
/// directory of that name length. Symlinks are never followed.
fn is_chunk_dir(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|t| t.is_dir())
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.chars().count() == CACHE_KEY_CHUNK_CHARS)
}

/// Remove language and context-specific cache files from `dir` and its chunk
/// directories (#771), removing chunk directories left empty. See
/// [`InstantPromptCache::clear_unrefreshable_files`].
fn clear_unrefreshable_files_in(dir: &Path, bases: &[String]) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            debug_log!(
                "cache",
                "Could not scan {} to clear unrefreshable caches: {e}",
                dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_chunk_dir(&entry) {
            clear_unrefreshable_files_in(&path, bases);
            // Fails, harmlessly, unless it is now empty.
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        let Some(file_name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };

        // Cache files are named `{key}.{base}.{token}.{ext}` (for a chunked
        // key, `{last chunk}.{base}.{token}.{ext}`). Neither the base, the
        // token nor the dialect extension contains a dot, so stripping the
        // extension, then the trailing `.{token}`, then the trailing `.{base}`
        // isolates the base for an exact match (the key may itself contain
        // dots).
        if let Some((stem, ext)) = file_name.rsplit_once('.')
            && PromptDialect::ALL
                .iter()
                .any(|dialect| dialect.cache_ext() == ext)
            && let Some((without_token, token)) = stem.rsplit_once('.')
            && let Some((_key, base)) = without_token.rsplit_once('.')
            && (bases.iter().any(|lang_base| lang_base == base) || token != NO_PREV_BG_TOKEN)
            && let Err(e) = std::fs::remove_file(&path)
        {
            debug_log!(
                "cache",
                "Could not remove unrefreshable cache {}: {e}",
                path.display()
            );
        }
    }
}

/// Cache-filename token of a context-free render (no `prev_bg`): the one
/// [`InstantPromptCache::clear_unrefreshable_files`] keeps.
const NO_PREV_BG_TOKEN: &str = "none";

/// Filesystem-safe token identifying the previous-segment background a cache
/// entry was rendered with.
///
/// The rendered ANSI bakes in the opening powerline chevron, whose color is
/// `fg:prev_bg`. A cache entry is therefore only valid for the `prev_bg` it was
/// rendered with, so the token becomes part of the cache filename
/// (`{key}.{suffix}.{token}.{ext}`). `None`/empty → `"none"` (context-free writers:
/// watcher, registration, background jobs). Otherwise every byte that is not
/// ASCII-alphanumeric is replaced with `_`.
///
/// The shells derive the SAME token from the SAME wire string they sent as
/// `prev_bg`, so both sides resolve the identical cache file without a subprocess.
/// The Fish/Bash/Zsh implementations (`__gpy_prev_bg_token`) MUST stay in lockstep
/// with this rule. Tokens never contain a `.`, which `clear_unrefreshable_files` relies
/// on to parse `{key}.{base}.{token}.{ext}` filenames.
fn prev_bg_token(prev_bg: Option<&str>) -> String {
    match prev_bg {
        Some(s) if !s.is_empty() => s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
        _ => NO_PREV_BG_TOKEN.to_owned(),
    }
}

/// Above this many characters, the `Os::Windows` arm of
/// [`path_to_cache_key_for`] truncates the escaped key and appends a
/// hash-tail (see that function) instead of returning it whole.
///
/// Sized against `cache_file_for_dir`'s actual filename shape,
/// `"{key}.{suffix}.{token}.{ext}"`: the longest `suffix` in this codebase is
/// `variant_suffix`'s `"lang_first_last"` (16 chars), and the longest
/// dialect extension (`ansi`/`bash`; `zsh` is shorter) plus the three
/// separator dots add another 7 characters on top of that. Add the
/// hash-tail overhead (`"_h"` plus 16 hex digits, 18 characters) to a
/// 200-char truncated prefix and the key alone tops out at 218, leaving
/// roughly 37 characters of headroom under Windows' 255-char
/// filename-component limit for the rest of the filename (`suffix`, dots,
/// `token`, and `ext`) -- comfortably covering the 16-char `suffix` and
/// its extension/dots overhead, with room left over for realistic `prev_bg`
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
/// introduced for the separators are not re-escaped. The Fish/Bash/Zsh
/// implementations (`__gpy_path_to_cache_key`) MUST stay byte-for-byte
/// identical so both sides resolve the same cache file without a subprocess --
/// on native Windows, though, no shell integration ever runs in this issue's
/// scoping (#478: Rust-only, no shell integration on native Windows), so that
/// contract binds only the [`crate::paths::Os::Unix`] arm this function
/// dispatches to; see [`path_to_cache_key_for`] for the Windows-specific
/// hardening on top of the same escape scheme.
///
/// The key is not yet the on-disk name: [`cache_key_path`] splits a key longer
/// than [`FLAT_CACHE_KEY_MAX_CHARS`] into `/`-joined chunks (#771), and the
/// shells mirror that too (`__gpy_chunk_cache_key`).
///
/// Example: `/Users/foo/project` -> `_sUsers_sfoo_sproject`
fn path_to_cache_key(path: &Path) -> String {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    path_to_cache_key_for(&path.to_string_lossy(), os)
}

/// Longest Unix cache key, in characters, stored as one flat filename (#771).
///
/// Keys up to this length keep their pre-#771 flat name byte-for-byte, so
/// existing caches stay warm. Longer keys are split into
/// [`CACHE_KEY_CHUNK_CHARS`]-character chunks: the flat form plus the longest
/// `.{suffix}.{token}.{ext}` tail (~30 bytes) would exceed `NAME_MAX`.
const FLAT_CACHE_KEY_MAX_CHARS: usize = 200;

/// Characters per chunk of a long cache key (#771): at most 200 bytes even at
/// 4 bytes per character, leaving room for the tail on the last chunk.
const CACHE_KEY_CHUNK_CHARS: usize = 50;

/// Longest filename component Linux and macOS accept, in bytes.
const NAME_MAX_BYTES: usize = 255;

/// Relative on-disk stem for a cache key on this platform; see
/// [`cache_key_path_for`].
fn cache_key_path(key: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    cache_key_path_for(key, os)
}

/// Relative on-disk stem for a cache key (#771).
///
/// | Key length (characters)          | Stem                                |
/// |----------------------------------|-------------------------------------|
/// | at most [`FLAT_CACHE_KEY_MAX_CHARS`] | the key, byte-for-byte          |
/// | longer                           | 50-character chunks joined by `/`   |
///
/// A cache file is `{stem}.{suffix}.{token}.{ext}`, so every chunk but the
/// last is a directory. Chunking an injective key keeps the mapping
/// injective, and flat keys never contain `/`, so the two forms cannot
/// collide. Chunks count characters (`chars()`), as Fish does; Bash and Zsh
/// match under a UTF-8 locale (under `LC_ALL=C` they count bytes, so a long
/// non-ASCII key resolves elsewhere and the shell just misses the cache).
/// The shells (`__gpy_chunk_cache_key`) MUST stay in lockstep with this, as
/// pinned by `tests/fixtures/cache_key_vectors.tsv`.
///
/// The `Os::Windows` arm returns the key unchanged: its key is already capped
/// by [`windows_harden_cache_key`] and no shell reads it (#478).
fn cache_key_path_for(key: &str, os: crate::paths::Os) -> std::borrow::Cow<'_, str> {
    if matches!(os, crate::paths::Os::Windows) || key.chars().count() <= FLAT_CACHE_KEY_MAX_CHARS {
        return std::borrow::Cow::Borrowed(key);
    }
    // Byte length bounds the char count, so this bounds the separators too.
    let separators = key.len().div_ceil(CACHE_KEY_CHUNK_CHARS);
    let mut stem = String::with_capacity(key.len().saturating_add(separators));
    for (i, c) in key.chars().enumerate() {
        if i > 0 && i.is_multiple_of(CACHE_KEY_CHUNK_CHARS) {
            stem.push('/');
        }
        stem.push(c);
    }
    std::borrow::Cow::Owned(stem)
}

/// Directory and filename of the cache file `{stem}.{tail}` under
/// `cache_dir`, where `stem` is [`cache_key_path`] of `key` (#771).
fn cache_file_parts(cache_dir: &Path, key: &str, tail: &str) -> (PathBuf, String) {
    let stem = cache_key_path(key);
    match stem.rsplit_once('/') {
        Some((chunk_dirs, last)) => (cache_dir.join(chunk_dirs), format!("{last}.{tail}")),
        None => (cache_dir.to_path_buf(), format!("{stem}.{tail}")),
    }
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
    // The five documented escapes apply on both arms; the shells implement exactly these
    // (#705). `_` MUST be first so later tokens are not re-escaped.
    let escaped = path_str
        .replace('_', "__")
        .replace('/', "_s")
        .replace('\\', "_b")
        .replace(':', "_c")
        .replace(' ', "_w");

    match os {
        crate::paths::Os::Unix => escaped,
        // `std::fs::canonicalize` returns the `\\?\`-prefixed verbatim form on Windows, so
        // every Windows-reserved filename character must also be escaped, or the
        // filesystem rejects the key with `ERROR_INVALID_NAME`. Windows-only: no shell runs
        // there, and on Unix these are ordinary filename bytes the shells leave alone.
        // Done before hardening so the case-fold sees lowercase tokens.
        crate::paths::Os::Windows => windows_harden_cache_key(
            &escaped
                .replace('?', "_q")
                .replace('*', "_a")
                .replace('<', "_l")
                .replace('>', "_g")
                .replace('"', "_d")
                .replace('|', "_p"),
        ),
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
            ".tmp-1234-5",
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
    fn sweep_keeps_cache_files_whose_key_contains_tmp() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let legit = aged_file(
            dir.path(),
            "_sx.tmp-1.git.none.ansi",
            Duration::from_hours(2),
        );

        let (entries, orphans) = prune_stale_entries(dir.path(), SystemTime::now());

        assert_eq!((entries, orphans), (0, 0));
        assert!(legit.exists(), "a key containing .tmp- is not an orphan");
    }

    #[test]
    fn sweep_spares_an_in_flight_temp_file() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let in_flight = aged_file(dir.path(), ".tmp-99-1", Duration::from_secs(1));

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
            .write_cache_file("repo", "git", PromptDialect::Ansi, "hello world")
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
        let content =
            std::fs::read_to_string(cache.cache_file_path("repo", "git", PromptDialect::Ansi))
                .expect("read");
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

        #[test]
        fn write_atomic_succeeds_for_names_near_name_max() {
            let temp_dir = tempfile::TempDir::new().expect("temp dir");
            let name = format!("{}.ansi", "a".repeat(245));

            write_atomic(temp_dir.path(), &name, "x").expect("write");

            assert_eq!(
                std::fs::read_to_string(temp_dir.path().join(&name)).expect("read"),
                "x"
            );
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
                .write_cache_file("repo", "git", PromptDialect::Ansi, "alpha")
                .expect("write"),
            "first write of new content should report a change"
        );
        // Identical content with the file present is a no-op.
        assert!(
            !cache
                .write_cache_file("repo", "git", PromptDialect::Ansi, "alpha")
                .expect("write"),
            "rewriting identical content should report no change"
        );
        // Different content is reported as changed.
        assert!(
            cache
                .write_cache_file("repo", "git", PromptDialect::Ansi, "beta")
                .expect("write"),
            "writing different content should report a change"
        );
        // A missing file is rewritten and reported as changed even when the
        // last-written content matches, so serve-stale readers that saw nothing
        // still get a repaint.
        std::fs::remove_file(cache.cache_file_path("repo", "git", PromptDialect::Ansi))
            .expect("remove");
        assert!(
            cache
                .write_cache_file("repo", "git", PromptDialect::Ansi, "beta")
                .expect("write"),
            "rewriting a missing file should report a change"
        );
    }

    // ---- INSTANT-CACHE TEST harness (#704) ------------------------------------
    //
    // Invariant: the instant-prompts dir equals what a fresh render of the
    // latest inputs would write (same file set, same bytes), every file touched
    // by the last operation has an mtime reflecting that write-or-verification,
    // and no temp/orphan files remain. Later rows (concurrent writers #706,
    // deleted dir #707, ~220-byte repo paths #708) extend `Op` and reuse `check`.

    /// One step applied to the cache under test.
    enum Op {
        /// Write `content` for `(key, suffix)` (Ansi dialect).
        Write(&'static str, &'static str, &'static str),
        /// Backdate every file in the directory by this many seconds, standing
        /// in for the clock advancing.
        Age(u64),
        /// `threads` writers released together by a `Barrier`, thread `i`
        /// writing `versions[i % versions.len()]` for `(key, suffix)` (#706).
        Concurrent(&'static str, &'static str, &'static [&'static str], usize),
        /// Remove the whole cache directory, as `rm -rf ~/.cache/gpy` would (#707).
        DeleteDir,
        /// Write all four git variants for a `key_len`-byte key, so the longest
        /// final name is `key_len + 25` bytes (#708); above 200 the key is
        /// stored chunked (#771).
        WriteVariants(usize, &'static str),
        /// Age every file past `CACHE_ENTRY_MAX_AGE` and run the startup
        /// sweep, which must empty the directory, chunk dirs included (#771).
        #[cfg(unix)]
        Sweep,
    }

    /// Independent model of the on-disk name of `(key, suffix)` (#771).
    ///
    /// Flat up to 200 characters, else 50-character chunks joined by `/`.
    /// Native Windows never chunks: its keys are capped by
    /// `windows_harden_cache_key` instead (#478).
    fn expected_name(key: &str, suffix: &str) -> String {
        let chars: Vec<char> = key.chars().collect();
        let stem = if cfg!(windows) || chars.len() <= 200 {
            key.to_owned()
        } else {
            chars
                .chunks(50)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("/")
        };
        format!("{stem}.{suffix}.ansi")
    }

    /// Every file under `dir` as `relative/path -> content`; panics on an
    /// empty subdirectory, which the writer, sweep and clear must never leave.
    fn collect_files(dir: &Path, prefix: &str, out: &mut HashMap<String, String>) {
        for dir_entry in std::fs::read_dir(dir).expect("read dir") {
            let entry = dir_entry.expect("dir entry");
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().expect("file type").is_dir() {
                let before = out.len();
                collect_files(&entry.path(), &format!("{name}/"), out);
                assert!(out.len() > before, "empty directory {name} left behind");
            } else {
                out.insert(
                    name,
                    std::fs::read_to_string(entry.path()).expect("read file"),
                );
            }
        }
    }

    struct CacheHarness {
        _dir: tempfile::TempDir,
        cache: InstantPromptCache,
        /// Independently computed expected directory: file name -> bytes.
        expected: HashMap<String, String>,
        /// Files the most recent `Write` wrote or verified.
        touched: Option<String>,
    }

    impl CacheHarness {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().expect("temp dir");
            let cache = InstantPromptCache::new_in_dir(dir.path().join("ip")).expect("cache");
            Self {
                _dir: dir,
                cache,
                expected: HashMap::new(),
                touched: None,
            }
        }

        fn write_variants(&mut self, key_len: usize, content: &str) {
            let key = "k".repeat(key_len);
            for suffix in [
                "git.none",
                "git_last.none",
                "git_first.none",
                "git_first_last.none",
            ] {
                self.cache
                    .write_cache_file(&key, suffix, PromptDialect::Ansi, content)
                    .expect("every variant name that fits NAME_MAX must be writable");
                let name = expected_name(&key, suffix);
                self.expected.insert(name.clone(), content.to_owned());
                self.touched = Some(name);
            }
        }

        #[cfg(unix)]
        fn sweep(&mut self) {
            let when = SystemTime::now()
                .checked_sub(CACHE_ENTRY_MAX_AGE)
                .and_then(|t| t.checked_sub(Duration::from_secs(60)))
                .expect("backdating within range");
            for name in self.expected.keys() {
                set_mtime(&self.cache.cache_dir.join(name), when);
            }
            let (removed, _) = prune_stale_entries(&self.cache.cache_dir, SystemTime::now());
            assert_eq!(removed, u64::try_from(self.expected.len()).expect("count"));
            self.expected.clear();
            self.touched = None;
        }

        fn apply(&mut self, op: &Op) {
            match *op {
                Op::WriteVariants(key_len, content) => self.write_variants(key_len, content),
                #[cfg(unix)]
                Op::Sweep => self.sweep(),
                Op::DeleteDir => {
                    std::fs::remove_dir_all(&self.cache.cache_dir).expect("remove dir");
                    self.expected.clear();
                    self.touched = None;
                    // Nothing to compare: the directory is legitimately absent.
                    return;
                }
                Op::Write(key, suffix, content) => {
                    self.cache
                        .write_cache_file(key, suffix, PromptDialect::Ansi, content)
                        .expect("write");
                    let name = expected_name(key, suffix);
                    self.expected.insert(name.clone(), content.to_owned());
                    self.touched = Some(name);
                }
                Op::Age(secs) => {
                    let when = SystemTime::now()
                        .checked_sub(Duration::from_secs(secs))
                        .expect("backdating within range");
                    for name in self.expected.keys() {
                        set_mtime(&self.cache.cache_dir.join(name), when);
                    }
                    self.touched = None;
                }
                Op::Concurrent(key, suffix, versions, threads) => {
                    let gate = std::sync::Barrier::new(threads);
                    let writer_cache = &self.cache;
                    std::thread::scope(|s| {
                        for &content in versions.iter().cycle().take(threads) {
                            let gate_ref = &gate;
                            s.spawn(move || {
                                gate_ref.wait();
                                writer_cache
                                    .write_cache_file(key, suffix, PromptDialect::Ansi, content)
                                    .expect("write");
                            });
                        }
                    });
                    let name = expected_name(key, suffix);
                    let recorded = self
                        .cache
                        .last_written
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&format!("{key}:{suffix}.ansi"))
                        .cloned()
                        .expect("a concurrent write must be recorded");
                    assert!(
                        versions.contains(&recorded.as_str()),
                        "record is a written version"
                    );
                    // The record must equal disk; `check` compares disk to it.
                    self.expected.insert(name.clone(), recorded);
                    self.touched = Some(name);
                }
            }
            self.check();
        }

        fn check(&self) {
            let mut actual = HashMap::new();
            collect_files(&self.cache.cache_dir, "", &mut actual);
            assert_eq!(
                actual, self.expected,
                "directory must equal a fresh render (no temp/orphan files)"
            );
            if let Some(name) = &self.touched {
                let mtime = std::fs::metadata(self.cache.cache_dir.join(name))
                    .and_then(|m| m.modified())
                    .expect("mtime");
                let floor = SystemTime::now()
                    .checked_sub(Duration::from_secs(2))
                    .expect("floor within range");
                assert!(
                    mtime >= floor,
                    "{name}: mtime must reflect the last write-or-verification"
                );
            }
        }
    }

    #[test]
    fn instant_cache_writes_every_variant_for_a_near_name_max_key() {
        let mut h = CacheHarness::new();
        // 225-byte key -> 250-byte longest variant name; its old temp name overflowed.
        h.apply(&Op::WriteVariants(225, "x"));
        h.apply(&Op::Age(60));
        h.apply(&Op::WriteVariants(225, "x"));
        h.apply(&Op::WriteVariants(225, "y"));
    }

    // Chunking is a Unix-only scheme: native Windows keys are capped by
    // `windows_harden_cache_key` and a 300-byte key is simply skipped (#478).
    #[cfg(unix)]
    #[test]
    fn instant_cache_writes_and_sweeps_a_chunked_300_byte_key() {
        let mut h = CacheHarness::new();
        // 300-byte key: flat it would be a 325-byte name; chunked it is
        // 5 directories of 50 plus a 75-byte final name (#771).
        h.apply(&Op::WriteVariants(300, "x"));
        h.apply(&Op::Age(60));
        h.apply(&Op::WriteVariants(300, "x"));
        h.apply(&Op::WriteVariants(300, "y"));
        h.apply(&Op::Write("repo", "git.none", "x"));
        // The sweep removes chunked entries and their emptied directories.
        h.apply(&Op::Sweep);
        // Chunk directories are recreated inside the write's retry, both
        // after the sweep and after the whole cache dir is deleted (#707).
        h.apply(&Op::WriteVariants(300, "z"));
        h.apply(&Op::DeleteDir);
        h.apply(&Op::WriteVariants(300, "z"));
    }

    #[test]
    fn instant_cache_dedup_after_aging_keeps_mtime_fresh() {
        let mut h = CacheHarness::new();
        h.apply(&Op::Write("repo", "git.none", "x"));
        h.apply(&Op::Age(60));
        h.apply(&Op::Write("repo", "git.none", "x"));
        h.apply(&Op::Age(600));
        h.apply(&Op::Write("repo", "git.none", "x"));
    }

    #[test]
    fn instant_cache_content_change_rewrites_and_variants_stay_independent() {
        let mut h = CacheHarness::new();
        h.apply(&Op::Write("repo", "git.none", "x"));
        h.apply(&Op::Write("repo", "git.red", "v"));
        h.apply(&Op::Age(60));
        h.apply(&Op::Write("repo", "git.none", "y"));
        h.apply(&Op::Write("repo", "git.red", "v"));
        h.apply(&Op::Age(60));
        h.apply(&Op::Write("repo", "git.none", "y"));
    }

    #[test]
    fn instant_cache_recovers_after_directory_deleted() {
        let mut h = CacheHarness::new();
        h.apply(&Op::Write("repo", "git.none", "x"));
        h.apply(&Op::DeleteDir);
        // Changed value: write path recreates the directory.
        h.apply(&Op::Write("repo", "git.none", "y"));
        h.apply(&Op::DeleteDir);
        // Identical to the recorded value: dedup finds the file gone and rewrites.
        h.apply(&Op::Write("repo", "git.none", "y"));
        h.apply(&Op::Write("repo", "git.red", "v"));
        h.apply(&Op::DeleteDir);
        h.apply(&Op::Write("repo", "git.red", "v"));
        h.apply(&Op::Write("repo", "git.none", "z"));
    }

    #[test]
    fn write_cache_file_recreates_missing_cache_dir() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let cache =
            InstantPromptCache::new_in_dir(dir.path().join("instant-prompts")).expect("cache");
        std::fs::remove_dir_all(dir.path().join("instant-prompts")).expect("remove dir");

        assert!(
            cache
                .write_cache_file("repo", "git.none", PromptDialect::Ansi, "x")
                .expect("write after dir removal"),
            "first write after removal reports a change"
        );
        assert_eq!(
            std::fs::read_to_string(cache.cache_file_path("repo", "git.none", PromptDialect::Ansi))
                .expect("read"),
            "x"
        );
    }

    #[test]
    fn instant_cache_concurrent_writers_keep_record_equal_to_disk() {
        let mut h = CacheHarness::new();
        for _ in 0..500_u32 {
            h.apply(&Op::Concurrent("repo", "git.none", &["A", "B"], 32));
            h.apply(&Op::Write("repo", "git.none", "A"));
        }
    }

    #[test]
    fn write_cache_file_record_matches_disk_under_contention() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let cache = std::sync::Arc::new(
            InstantPromptCache::new_in_dir(dir.path().to_path_buf()).expect("cache"),
        );
        let path = cache.cache_file_path("repo", "git.none", PromptDialect::Ansi);
        for round in 0..500_u32 {
            let gate = std::sync::Arc::new(std::sync::Barrier::new(32));
            let handles: Vec<_> = ["A", "B"]
                .iter()
                .cycle()
                .take(32)
                .map(|&content| {
                    let writer_cache = std::sync::Arc::clone(&cache);
                    let writer_gate = std::sync::Arc::clone(&gate);
                    std::thread::spawn(move || {
                        writer_gate.wait();
                        writer_cache
                            .write_cache_file("repo", "git.none", PromptDialect::Ansi, content)
                            .expect("write");
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("writer thread");
            }
            cache
                .write_cache_file("repo", "git.none", PromptDialect::Ansi, "A")
                .expect("write");
            assert_eq!(
                std::fs::read_to_string(&path).expect("read"),
                "A",
                "round {round}: write of A after contention must leave A on disk"
            );
        }
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
            "/odd/q?a*b<c>d\"e|f",
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
        let cases: [(&str, &str); 6] = [
            ("/Users/foo/project", "_sUsers_sfoo_sproject"),
            ("/tmp/what?proj", "_stmp_swhat?proj"),
            ("/a*b<c>d\"e|f", "_sa*b<c>d\"e|f"),
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

    /// Keys up to 200 characters keep their flat name; longer ones are split
    /// into 50-character chunks, counted in characters (#771).
    #[test]
    fn test_cache_key_path_unix_characterization() {
        let unix = crate::paths::Os::Unix;
        let flat = "k".repeat(FLAT_CACHE_KEY_MAX_CHARS);
        assert_eq!(cache_key_path_for(&flat, unix), flat.as_str());
        let deep = path_to_cache_key_for(
            "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/issue/478s/implementation/work",
            unix,
        );
        assert_eq!(
            cache_key_path_for(&deep, unix),
            "_sa_svery_sdeeply_snested_sdirectory_sstructure_st/hat_sgoes_son_sand_son_sand_son_sand_son_sand_son_/sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_s/logic_sand_smake_ssure_snothing_schanges_sunexpect/edly_swhen_sthis_sfunction_sgains_swindows_sharden/ing_sas_spart_sof_sissue_s478s_simplementation_swo/rk"
        );
        // 201 two-byte characters: chunked by character, not byte.
        let wide = "é".repeat(201);
        let chunked = cache_key_path_for(&wide, unix);
        let chunks: Vec<&str> = chunked.split('/').collect();
        assert_eq!(chunks.len(), 5);
        assert!(chunks.iter().take(4).all(|c| c.chars().count() == 50));
        assert_eq!(chunks.last().copied(), Some("é"));
        // The Windows key is already capped; it is never chunked.
        let long = "k".repeat(FLAT_CACHE_KEY_MAX_CHARS + 18);
        assert_eq!(
            cache_key_path_for(&long, crate::paths::Os::Windows),
            long.as_str()
        );
    }

    /// A flat key that fits 200 characters but not `NAME_MAX` bytes is
    /// skipped without an error (#771 residual).
    #[test]
    fn write_cache_file_skips_a_flat_key_over_name_max() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
        let key = "\u{1F600}".repeat(64);
        for _ in 0_u8..2_u8 {
            assert!(
                !cache
                    .write_cache_file(&key, "git.none", PromptDialect::Ansi, "x")
                    .expect("an unrepresentable name is skipped, not an error")
            );
        }
        assert_eq!(
            cache
                .unwritable_keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len(),
            1
        );
    }

    /// The Windows arm escapes the reserved filename characters `? * < > " |`.
    #[test]
    fn test_path_to_cache_key_windows_escapes_reserved_chars() {
        assert_eq!(
            path_to_cache_key_for("a?*<>\"|b", crate::paths::Os::Windows),
            "a_q_a_l_g_d_pb"
        );
        assert_eq!(
            path_to_cache_key_for(r"C:\a?b", crate::paths::Os::Windows),
            "c_c_ba_qb"
        );
    }

    /// Shared vectors (`tests/fixtures/cache_key_vectors.tsv`, #705) also read by the
    /// Bash/Zsh/Fish encoder tests: the Unix arm must match every row and round-trip.
    #[test]
    fn test_path_to_cache_key_shared_vectors() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/cache_key_vectors.tsv");
        let text = std::fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));
        let unescape = |raw: &str| -> String {
            let mut out = String::new();
            let mut chars = raw.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('\\') => out.push('\\'),
                        Some('n') => out.push('\n'),
                        Some('r') => out.push('\r'),
                        Some('t') => out.push('\t'),
                        other => panic!("bad fixture escape: \\{other:?}"),
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
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
        let mut count = 0_usize;
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(raw_input), Some(expected), Some(stem)) =
                (cols.next(), cols.next(), cols.next())
            else {
                panic!("malformed vector row: {line:?}");
            };
            let input = unescape(raw_input);
            let key = path_to_cache_key_for(&input, crate::paths::Os::Unix);
            assert_eq!(key, expected, "vector for {raw_input}");
            assert_eq!(decode(&key), input, "round-trip for {raw_input}");
            assert_vector_stem(raw_input, &key, stem);
            count += 1;
        }
        assert!(count > 0, "no vectors read from {}", fixture.display());
    }

    /// #771: the shared vector's `stem` is the relative cache-file path the
    /// writer builds for `key`.
    fn assert_vector_stem(raw_input: &str, key: &str, stem: &str) {
        assert_eq!(
            cache_key_path_for(key, crate::paths::Os::Unix),
            stem,
            "stem for {raw_input}"
        );
        #[cfg(not(windows))]
        {
            let (dir, name) = cache_file_parts(Path::new("ip"), key, "git.none.ansi");
            assert_eq!(
                dir.join(name),
                Path::new("ip").join(format!("{stem}.git.none.ansi")),
                "cache file for {raw_input}"
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
        let lang_path = cache.cache_file_path(&key, "lang.none", PromptDialect::Ansi);
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
        let lang_last_path = cache.cache_file_path(&key, "lang_last.none", PromptDialect::Ansi);
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
        let lang_path = cache.cache_file_path(&key, "lang.none", PromptDialect::Ansi);
        let lang_last_path = cache.cache_file_path(&key, "lang_last.none", PromptDialect::Ansi);

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
    fn test_clear_unrefreshable_files_keeps_only_context_free_git_cache() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache =
            InstantPromptCache::new_in_dir(temp_dir.path().join("instant-prompts")).expect("cache");

        // Use the real `{base}.{token}.{ext}` scheme, in every dialect, so the
        // clear logic is exercised against the filenames the writers produce.
        for dialect in PromptDialect::ALL {
            cache
                .write_cache_file("repo", "git.none", dialect, "git status")
                .expect("write git cache");
            cache
                .write_cache_file("repo", "git_last.blue", dialect, "git status")
                .expect("write context-specific git cache");
            cache
                .write_cache_file("repo", "lang.none", dialect, "ruby 4.0.5")
                .expect("write lang cache");
            cache
                .write_cache_file("repo", "lang_last.blue", dialect, "ruby 4.0.5")
                .expect("write lang_last cache");
            cache
                .write_cache_file("repo", "lang_first.none", dialect, "ruby 4.0.5")
                .expect("write lang_first cache");
            cache
                .write_cache_file("repo", "lang_first_last.blue", dialect, "ruby 4.0.5")
                .expect("write lang_first_last cache");
        }

        cache.clear_unrefreshable_files();

        for dialect in PromptDialect::ALL {
            assert!(cache.cache_file_path("repo", "git.none", dialect).exists());
            assert!(
                !cache
                    .cache_file_path("repo", "git_last.blue", dialect)
                    .exists(),
                "a context-specific git file would never be refreshed after a restart"
            );
            assert!(!cache.cache_file_path("repo", "lang.none", dialect).exists());
            assert!(
                !cache
                    .cache_file_path("repo", "lang_last.blue", dialect)
                    .exists()
            );
            for name in ["lang_first.none", "lang_first_last.blue"] {
                assert!(
                    !cache.cache_file_path("repo", name, dialect).exists(),
                    "{name} must be cleared like lang/lang_last (#773)"
                );
            }
        }
    }

    #[test]
    fn clear_unrefreshable_files_skips_unremovable_entries() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache_dir = temp_dir.path().join("instant-prompts");
        let cache = InstantPromptCache::new_in_dir(cache_dir.clone()).expect("cache");

        // A directory whose name matches the language-cache pattern cannot be
        // removed with `remove_file`; it must not abort the sweep (#773).
        let blocker = cache_dir.join("x.lang.none.ansi");
        std::fs::create_dir_all(&blocker).expect("create blocking dir");
        cache
            .write_cache_file("repo", "lang.none", PromptDialect::Ansi, "ruby 4.0.5")
            .expect("write lang cache");

        cache.clear_unrefreshable_files();

        assert!(
            !cache
                .cache_file_path("repo", "lang.none", PromptDialect::Ansi)
                .exists(),
            "the regular language cache file is removed"
        );
        assert!(blocker.is_dir(), "the unremovable entry is left in place");
    }

    /// `clear_unrefreshable_files` reaches chunked entries and removes the chunk
    /// directories it empties, but keeps git caches and their dirs (#771).
    #[cfg(unix)]
    #[test]
    fn clear_unrefreshable_files_clears_chunked_entries() {
        let temp_dir = tempfile::TempDir::new().expect("temp cache dir");
        let cache_dir = temp_dir.path().join("instant-prompts");
        let cache = InstantPromptCache::new_in_dir(cache_dir.clone()).expect("cache");
        let lang_only = "l".repeat(300);
        let both = "b".repeat(300);
        for (key, suffix) in [
            (&lang_only, "lang_first_last.none"),
            (&both, "lang.none"),
            (&both, "git.none"),
        ] {
            cache
                .write_cache_file(key, suffix, PromptDialect::Ansi, "x")
                .expect("write");
        }

        cache.clear_unrefreshable_files();

        assert!(
            !cache_dir.join("l".repeat(50)).exists(),
            "emptied chunk directories are removed"
        );
        assert!(
            !cache
                .cache_file_path(&both, "lang.none", PromptDialect::Ansi)
                .exists()
        );
        assert!(
            cache
                .cache_file_path(&both, "git.none", PromptDialect::Ansi)
                .is_file()
        );
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
        let git_path = cache.cache_file_path(&key, "git.none", PromptDialect::Ansi);
        let git_last_path = cache.cache_file_path(&key, "git_last.none", PromptDialect::Ansi);

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
            std::fs::read_to_string(cache.cache_file_path(&key, suffix, PromptDialect::Ansi))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        for suffix in [
            "git.none",
            "git_last.none",
            "git_first.none",
            "git_first_last.none",
        ] {
            assert!(
                cache
                    .cache_file_path(&key, suffix, PromptDialect::Ansi)
                    .exists(),
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
            std::fs::read_to_string(cache.cache_file_path(&key, suffix, PromptDialect::Ansi))
                .unwrap_or_else(|_| panic!("read {suffix}"))
        };

        for suffix in [
            "lang.none",
            "lang_last.none",
            "lang_first.none",
            "lang_first_last.none",
        ] {
            assert!(
                cache
                    .cache_file_path(&key, suffix, PromptDialect::Ansi)
                    .exists(),
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

    /// Build a git repo under `root` with a near-`NAME_MAX` cache key.
    ///
    /// The longest variant name (`<key>.git_first_last.none.ansi`) is exactly
    /// 250 bytes: a legal final name whose old `<name>.tmp-<pid>-<n>` temp
    /// name exceeded `NAME_MAX` (#708).
    fn near_name_max_repo(root: &Path) -> (PathBuf, String) {
        let parent = std::fs::canonicalize(root).expect("canonicalize root");
        let fixed = path_to_cache_key(&parent)
            .len()
            .checked_add("_s".len())
            .and_then(|n| n.checked_add(".git_first_last.none.ansi".len()))
            .expect("length");
        let pad = 250_usize.checked_sub(fixed).expect("root fits");
        let repo = parent.join("a".repeat(pad));
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let key = path_to_cache_key(&repo);
        (repo, key)
    }

    const GIT_VARIANTS: [&str; 4] = [
        "git.none",
        "git_last.none",
        "git_first.none",
        "git_first_last.none",
    ];

    #[test]
    fn write_git_writes_every_variant_for_a_near_name_max_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
        let (repo, key) = near_name_max_repo(temp_dir.path());
        let config = Config::default();

        let wrote = cache
            .write_git(
                &repo,
                &clean_status("main"),
                &config,
                &ThemeConfig::default(),
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git");

        assert!(wrote);
        for suffix in GIT_VARIANTS {
            assert!(
                cache
                    .cache_file_path(&key, suffix, PromptDialect::Ansi)
                    .exists(),
                "{suffix} must be written"
            );
        }
    }

    #[test]
    fn write_language_variants_writes_every_variant_for_a_near_name_max_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
        let (repo, key) = near_name_max_repo(temp_dir.path());
        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let languages = vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }];

        cache
            .write_language_variants(
                &repo,
                &languages,
                &config,
                &theme_mgr.get(),
                None,
                &crate::palette::active_palette(&config),
                None,
            )
            .expect("write variants");

        for suffix in [
            "lang.none",
            "lang_last.none",
            "lang_first.none",
            "lang_first_last.none",
        ] {
            assert!(
                cache
                    .cache_file_path(&key, suffix, PromptDialect::Ansi)
                    .exists(),
                "{suffix} must be written"
            );
        }
    }

    /// A repo under three 80-character components has a ~300-byte key; every
    /// variant must still be written, at the path both the writer and
    /// `cache_file_for_dir` resolve (#771).
    #[test]
    fn write_git_caches_repo_with_very_long_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
        let repo = temp_dir
            .path()
            .join("a".repeat(80))
            .join("b".repeat(80))
            .join("c".repeat(80));
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let canonical = std::fs::canonicalize(&repo).expect("canonicalize");
        let key = path_to_cache_key(&canonical);
        // Unix chunks an over-long key into directories; native Windows caps
        // it by hardening instead, so it never exceeds NAME_MAX there (#478).
        #[cfg(unix)]
        assert!(key.len() > 255, "the key alone must exceed NAME_MAX");
        #[cfg(windows)]
        assert!(
            key.chars().count() <= WINDOWS_CACHE_KEY_MAX_LEN + 18,
            "the Windows key is capped by hardening"
        );
        let config = Config::default();

        let wrote = cache
            .write_git(
                &repo,
                &clean_status("main"),
                &config,
                &ThemeConfig::default(),
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("write git");

        assert!(wrote);
        for suffix in ["git", "git_last", "git_first", "git_first_last"] {
            let written =
                cache.cache_file_path(&key, &format!("{suffix}.none"), PromptDialect::Ansi);
            assert!(written.is_file(), "{suffix} must be written");
            // `cache_file_for_dir` resolves the same path under the
            // environment's cache root.
            if let Ok(shell_path) = InstantPromptCache::cache_file_for_dir(
                &canonical,
                suffix,
                None,
                PromptDialect::Ansi,
            ) {
                let root = get_instant_cache_dir().expect("cache dir");
                assert_eq!(
                    shell_path.strip_prefix(&root).expect("under root"),
                    written.strip_prefix(&cache.cache_dir).expect("under root"),
                );
            }
        }
    }

    #[test]
    fn write_git_writes_every_variant_when_one_fails() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
        let repo = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let key = path_to_cache_key(&std::fs::canonicalize(&repo).expect("canonicalize"));
        // A directory at the target: the rename onto it fails.
        std::fs::create_dir_all(cache.cache_file_path(&key, "git_first.none", PromptDialect::Ansi))
            .expect("blocking dir");
        let config = Config::default();

        let wrote = cache
            .write_git(
                &repo,
                &clean_status("main"),
                &config,
                &ThemeConfig::default(),
                None,
                &crate::palette::active_palette(&config),
            )
            .expect("one failing variant must not fail the call");

        assert!(wrote, "the other variants changed, so the caller must ring");
        for suffix in ["git.none", "git_last.none", "git_first_last.none"] {
            assert!(
                cache
                    .cache_file_path(&key, suffix, PromptDialect::Ansi)
                    .is_file(),
                "{suffix} must be written despite git_first failing"
            );
        }
    }

    /// Part A: the instant-prompt cache must contain the branch name AND ANSI escapes,
    /// confirming that the active palette flows through `render_git_prompts` and produces
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
            std::fs::read_to_string(cache.cache_file_path(&key, "git.none", PromptDialect::Ansi))
                .expect("read cache");

        assert!(
            rendered.contains("main"),
            "cached git prompt must contain branch name: {rendered:?}"
        );
        assert!(
            rendered.contains('\u{1b}'),
            "cached git prompt must contain ANSI escape: {rendered:?}"
        );
    }

    /// Regression for #677: each dialect's cache file escapes the branch for
    /// its own shell, so an instant-cache hit is as safe as a fresh render.
    #[test]
    fn write_git_writes_each_dialect_escaped_for_its_shell() {
        use crate::git::{RepositoryState, RepositoryStatus};

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

        let status = RepositoryStatus {
            branch: "feat/$HOME-100%_x".to_owned(),
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
        let read = |dialect| {
            std::fs::read_to_string(cache.cache_file_path(&key, "git.none", dialect))
                .expect("read cache")
        };
        assert!(read(PromptDialect::Ansi).contains("feat/$HOME-100%_x"));
        assert!(read(PromptDialect::BashPrompt).contains("feat/\\\\$HOME-100%_x"));
        assert!(read(PromptDialect::ZshPrompt).contains("feat/\\$HOME-100%%_x"));
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
            std::fs::read_to_string(cache.cache_file_path(&key, "git.none", PromptDialect::Ansi))
                .expect("read cache");

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
    /// reload doorbell) is covered by the Fish integration suite.
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
            "token must never contain a dot (clear_unrefreshable_files relies on this)"
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
        let path = |suffix: &str| cache.cache_file_path(&key, suffix, PromptDialect::Ansi);
        let read = |suffix: &str| {
            std::fs::read_to_string(path(suffix)).unwrap_or_else(|_| panic!("read {suffix}"))
        };

        // All three context files coexist.
        assert!(path("git.blue").exists());
        assert!(path("git.green").exists());
        assert!(path("git.none").exists());

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
            std::fs::read_to_string(cache.cache_file_path(&key, "git.blue", PromptDialect::Ansi))
                .expect("read blue");

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
            std::fs::read_to_string(cache.cache_file_path(&key, "git.blue", PromptDialect::Ansi))
                .expect("read blue");
        assert_eq!(
            blue_before, blue_after,
            "a None write must not clobber the context-specific cache file"
        );
        assert!(
            cache
                .cache_file_path(&key, "git.none", PromptDialect::Ansi)
                .exists(),
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
        assert!(
            cache
                .cache_file_path(&key, "lang.blue", PromptDialect::Ansi)
                .exists()
        );
        assert!(
            cache
                .cache_file_path(&key, "lang.green", PromptDialect::Ansi)
                .exists()
        );

        let blue =
            std::fs::read_to_string(cache.cache_file_path(&key, "lang.blue", PromptDialect::Ansi))
                .expect("read blue");
        let green =
            std::fs::read_to_string(cache.cache_file_path(&key, "lang.green", PromptDialect::Ansi))
                .expect("read green");
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
            std::fs::read_to_string(cache.cache_file_path(&key, suffix, PromptDialect::Ansi))
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
