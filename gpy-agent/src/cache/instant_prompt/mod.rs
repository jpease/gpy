//! Instant prompt caching: shells read pre-rendered prompts the daemon keeps fresh.
//!
//! Files live in `~/.cache/gpy/instant-prompts/{key}.{suffix}.{token}.{ext}`:
//! `{key}` from [`path_to_cache_key`], `{suffix}` the segment variant,
//! `{token}` the [`prev_bg_token`] (the ANSI bakes in the `fg:prev_bg`
//! chevron), `{ext}` the [`PromptDialect::cache_ext`].

use crate::cache::bounded::{INSTANT_PROMPT_LAST_WRITTEN_CAPACITY, evict_to_capacity};
use crate::config::Config;
use crate::debug_log;
use crate::formatter::{
    FishAnsiFormatter, Formatter, IsFirst, IsLast, PromptDialect, RenderContext, SegmentPosition,
};
use crate::git::RepositoryStatus;
use crate::ipc::{LanguageInfo, Response};
use crate::sync_ext::{MutexExt, RwLockExt};
use crate::template::{Color, Palette, parse_color};
use crate::theme::ThemeConfig;
use crate::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime};

/// Per-process temp-file counter so concurrent writers never share a temp path.
pub(super) fn next_tmp_suffix() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Create `path` owner-only (`0700`); missing parents get the default mode.
/// An existing real directory is restricted too; a symlink is left alone.
/// # Errors
/// Returns an error if a directory cannot be created or inspected.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        Ok(()) | Err(_) => {}
    }
    if std::fs::symlink_metadata(path)?.file_type().is_dir()
        && let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    {
        crate::warn_log!("cache", "cannot restrict {}: {e}", path.display());
    }
    Ok(())
}

/// Create `path` and its parents.
/// # Errors
/// Returns an error if a directory cannot be created.
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Create the temp file `path` owner-only (`0600` on unix), replacing a
/// leftover from an earlier attempt once.
fn create_temp_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(path);
            options.open(path)
        }
        result => result,
    }
}

/// Write `content` to `dir/name` atomically via a dot-prefixed, owner-only
/// temp file + rename.
/// # Errors
/// Returns an error if the write or rename fails; the temp file is removed best-effort.
pub(super) fn write_atomic(dir: &Path, name: &str, content: &str) -> Result<()> {
    let target = dir.join(name);
    // Independent of `name`: deriving it overflowed NAME_MAX for long names.
    let tmp_path = dir.join(format!(".tmp-{}-{}", std::process::id(), next_tmp_suffix()));

    if let Err(e) =
        create_temp_file(&tmp_path).and_then(|mut file| file.write_all(content.as_bytes()))
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    std::fs::rename(&tmp_path, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::from(e)
    })
}

/// Combine multi-variant write outcomes: any success wins, errors are logged.
/// # Errors
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
const MAX_PREV_BG_CONTEXTS_PER_KEY: usize = 16;

/// Insertion-ordered `prev_bg` contexts of one key (`None` = context-free),
/// with the key's use-recency tick.
struct Contexts {
    list: Vec<Option<String>>,
    tick: u64,
}

/// Contexts per key plus the tick counter that orders keys for eviction.
struct SeenContexts {
    entries: HashMap<String, Contexts>,
    tick: u64,
}

/// Content last written to one cache file, with its write-recency tick.
struct Written {
    content: String,
    tick: u64,
}

/// Write records plus the tick counter that orders them for eviction.
struct LastWritten {
    entries: HashMap<String, Written>,
    tick: u64,
}

impl LastWritten {
    /// Advance and return the write-recency tick.
    const fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }
}

/// Manager for instant-prompt cache files.
pub struct InstantPromptCache {
    cache_dir: PathBuf,
    /// Content and write-recency tick per `{cache_key}:{suffix}`.
    /// Invariant: `content` equals the file's content.
    last_written: Mutex<LastWritten>,
    /// `prev_bg` contexts seen per key; background writers refresh all of them.
    seen_contexts: RwLock<SeenContexts>,
    /// Keys whose filename cannot fit `NAME_MAX` even chunked; logged once each.
    unwritable_keys: Mutex<HashSet<String>>,
    /// Repos whose git caches [`Self::remove_git`] dropped, by cache key, with
    /// the root each key names. Git writes for these keys are skipped until
    /// [`Self::release_git_suppression`] lets them through again, so a write
    /// already in flight when a reload excluded the repo cannot republish it.
    /// Locked only while `last_written` is held.
    suppressed_git: Mutex<HashMap<String, PathBuf>>,
    /// Test-only count of [`Self::write_cache_file`] invocations.
    #[cfg(test)]
    write_calls: AtomicU64,
}

impl InstantPromptCache {
    /// Create a new instant-prompt cache manager.
    /// # Errors
    /// Returns an error naming the cache directory if it cannot be created.
    pub fn new() -> Result<Self> {
        let cache_dir = get_instant_cache_dir()?;
        create_private_dir(&cache_dir).map_err(|e| Error::Agent {
            message: format!(
                "cannot create instant-prompt cache directory {}",
                cache_dir.display()
            ),
            source: Some(Box::new(e)),
        })?;
        // Nothing else removes entries; best-effort, never blocks startup.
        let _ = prune_stale_entries(&cache_dir, SystemTime::now());
        Ok(Self {
            cache_dir,
            last_written: Mutex::new(LastWritten {
                entries: HashMap::new(),
                tick: 0,
            }),
            seen_contexts: RwLock::new(SeenContexts {
                entries: HashMap::new(),
                tick: 0,
            }),
            unwritable_keys: Mutex::new(HashSet::new()),
            suppressed_git: Mutex::new(HashMap::new()),
            #[cfg(test)]
            write_calls: AtomicU64::new(0),
        })
    }

    /// A cache in a fresh, never-cleaned temp directory, for tests (`new` needs
    /// `HOME`/XDG). Ungated so integration tests and `new_test_handle` can use it.
    /// # Panics
    /// Panics if the directory cannot be created.
    #[doc(hidden)]
    #[expect(
        clippy::expect_used,
        reason = "test-only helper; an unusable temp dir is fatal"
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

    /// Ungated because it backs [`Self::new_for_test`].
    /// # Errors
    /// Returns an error if the cache directory cannot be created.
    #[doc(hidden)]
    pub(crate) fn new_in_dir(cache_dir: PathBuf) -> Result<Self> {
        create_private_dir(&cache_dir)?;
        Ok(Self {
            cache_dir,
            last_written: Mutex::new(LastWritten {
                entries: HashMap::new(),
                tick: 0,
            }),
            seen_contexts: RwLock::new(SeenContexts {
                entries: HashMap::new(),
                tick: 0,
            }),
            unwritable_keys: Mutex::new(HashSet::new()),
            suppressed_git: Mutex::new(HashMap::new()),
            #[cfg(test)]
            write_calls: AtomicU64::new(0),
        })
    }

    /// Number of times [`Self::write_cache_file`] has run (test-only).
    #[cfg(all(test, unix))]
    pub(crate) fn write_call_count(&self) -> u64 {
        self.write_calls.load(Ordering::Relaxed)
    }

    /// Write every git variant, dialect and known `prev_bg` context; returns
    /// `true` when any content changed.
    /// # Errors
    /// Returns an error if prompt rendering or file writing fails.
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

        // Background writers pass `None` but must refresh every known context.
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

    /// Write the language cache for one position; returns `true` if content changed.
    /// `languages` are resolved display entries, so writing never probes a version.
    /// # Errors
    /// Returns an error if prompt rendering or file writing fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "independent render inputs; a params struct would only rename them"
    )]
    pub(crate) fn write_language(
        &self,
        path: &Path,
        languages: &[LanguageInfo],
        config: &Config,
        theme: &ThemeConfig,
        pos: SegmentPosition,
        prev_bg: Option<&str>,
        palette: &Palette,
    ) -> Result<bool> {
        let project_root = find_project_root(path);
        let key = path_to_cache_key(&project_root);
        let base = variant_suffix("lang", pos.is_last, pos.is_first);

        // Refresh every known render context, as in `write_git`.
        let mut wrote_any = false;
        let mut first_err = None;
        for ctx_prev_bg in self.known_contexts(&key, prev_bg) {
            let token = prev_bg_token(ctx_prev_bg.as_deref());
            let color = ctx_prev_bg.as_deref().and_then(|s| parse_color(s).ok());
            let prompts =
                match render_language_prompts(languages, config, theme, pos, color, palette) {
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

    /// Write all four language variants; returns `true` if any content changed.
    /// # Errors
    /// Returns an error if prompt rendering or file writing fails.
    pub fn write_language_variants(
        &self,
        path: &Path,
        languages: &[LanguageInfo],
        config: &Config,
        theme: &ThemeConfig,
        prev_bg: Option<&str>,
        palette: &Palette,
    ) -> Result<bool> {
        let mut wrote_any = false;
        let mut first_err = None;
        for pos in SEGMENT_POSITIONS {
            match self.write_language(path, languages, config, theme, pos, prev_bg, palette) {
                Ok(wrote) => wrote_any |= wrote,
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        settle_variant_writes(wrote_any, first_err)
    }

    /// Record `prev_bg` for `key` and return all known contexts. Past the caps
    /// the oldest context of the key, and the least recently used key, go first.
    fn known_contexts(&self, key: &str, prev_bg: Option<&str>) -> Vec<Option<String>> {
        let requested = prev_bg.map(str::to_owned);
        let mut guard = self.seen_contexts.write_or_recover();
        let seen = &mut *guard;

        seen.tick = seen.tick.wrapping_add(1);
        let tick = seen.tick;
        let contexts = seen.entries.entry(key.to_owned()).or_insert(Contexts {
            list: Vec::new(),
            tick,
        });
        contexts.tick = tick;
        if !contexts.list.contains(&requested) {
            if contexts.list.len() >= MAX_PREV_BG_CONTEXTS_PER_KEY {
                contexts.list.remove(0_usize);
            }
            contexts.list.push(requested);
        }
        let known = contexts.list.clone();

        // Bound the number of repositories tracked; `key` holds the newest tick.
        evict_to_capacity(
            &mut seen.entries,
            INSTANT_PROMPT_LAST_WRITTEN_CAPACITY,
            |contexts| contexts.tick,
        );
        drop(guard);
        known
    }

    /// Remove language and context-specific cache files, which a restarted agent
    /// (contexts are in-memory) would never refresh. Best-effort.
    pub fn clear_unrefreshable_files(&self) {
        let bases = SEGMENT_POSITIONS.map(|pos| variant_suffix("lang", pos.is_last, pos.is_first));

        clear_unrefreshable_files_in(&self.cache_dir, &bases);

        self.last_written
            .lock_or_recover()
            .entries
            .retain(|map_key, _written| {
                // The cache key has no colon, so split on the first one.
                let suffix = map_key.split_once(':').map_or("", |(_key, s)| s);
                let is_language = bases.iter().any(|base| {
                    suffix
                        .strip_prefix(base.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
                });
                // The token is the second-to-last component.
                let token = suffix.rsplit('.').nth(1).unwrap_or("");
                !(is_language || token != NO_PREV_BG_TOKEN)
            });
    }

    /// Delete every git cache file of the repository at `path` (each variant,
    /// `prev_bg` token and dialect, including contexts no longer tracked) so
    /// shells stop serving its git segment. Best-effort.
    pub fn remove_git(&self, path: &Path) {
        let key = path_to_cache_key(&find_project_root(path));
        let bases = SEGMENT_POSITIONS.map(|pos| variant_suffix("git", pos.is_last, pos.is_first));
        let (dir, prefix) = cache_file_parts(&self.cache_dir, &key, "");

        // Held across the deletion so the record keeps matching the disk.
        let mut last_written = self.last_written.lock_or_recover();
        last_written.entries.retain(|map_key, _written| {
            // `{key}:{base}.{token}.{ext}`; the cache key has no colon.
            !map_key.split_once(':').is_some_and(|(entry_key, tail)| {
                entry_key == key
                    && tail
                        .split_once('.')
                        .is_some_and(|(base, _rest)| bases.iter().any(|b| b == base))
            })
        });
        remove_git_files_in(&dir, &prefix, &bases);
        self.suppressed_git
            .lock_or_recover()
            .insert(key, find_project_root(path));
        drop(last_written);
    }

    /// Let git writes through again for every repo [`Self::remove_git`]
    /// suppressed that `enabled` now accepts (given the repo root).
    pub fn release_git_suppression(&self, enabled: impl Fn(&Path) -> bool) {
        let _last_written = self.last_written.lock_or_recover();
        self.suppressed_git
            .lock_or_recover()
            .retain(|_key, root| !enabled(root));
    }

    /// Atomically write a cache file unless already current; returns `true` if written.
    /// # Errors
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

        // Held across check, touch, write and record so the record equals disk.
        let mut cache = self.last_written.lock_or_recover();
        if suffix.starts_with("git") && self.suppressed_git.lock_or_recover().contains_key(key) {
            return Ok(false);
        }
        if cache
            .entries
            .get(&map_key)
            .is_some_and(|written| written.content == content)
        {
            // Unchanged: bump mtime for the shells' TTL; a vanished file is rewritten.
            match std::fs::File::options()
                .write(true)
                .open(&cache_file)
                .and_then(|f| f.set_modified(SystemTime::now()))
            {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Ok(()) | Err(_) => {
                    let tick = cache.next_tick();
                    if let Some(written) = cache.entries.get_mut(&map_key) {
                        written.tick = tick;
                    }
                    return Ok(false);
                }
            }
        }

        // Directory may have been removed externally; recreate and retry once
        // under the same guard. Not done up front to avoid a `stat` per write.
        match write_atomic(&dir, &file_name, content) {
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                create_private_dir(&self.cache_dir)?;
                std::fs::create_dir_all(&dir)?;
                write_atomic(&dir, &file_name, content)?;
            }
            result => result?,
        }

        let tick = cache.next_tick();
        cache.entries.insert(
            map_key,
            Written {
                content: content.to_owned(),
                tick,
            },
        );
        // Keep the dedup table bounded; the least recently written go first.
        evict_to_capacity(
            &mut cache.entries,
            INSTANT_PROMPT_LAST_WRITTEN_CAPACITY,
            |written| written.tick,
        );
        drop(cache);

        Ok(true)
    }

    /// Log once per root that `key`'s files exceed `NAME_MAX` and are skipped.
    fn note_unwritable_key(&self, key: &str) {
        let mut seen = self.unwritable_keys.lock_or_recover();
        if seen.len() < INSTANT_PROMPT_LAST_WRITTEN_CAPACITY && seen.insert(key.to_owned()) {
            debug_log!(
                "cache",
                "Skipping instant cache for {key}: its filename exceeds {NAME_MAX_BYTES} bytes"
            );
        }
    }

    /// Cache file path a shell in `dialect` should read for a directory.
    /// # Errors
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
const SEGMENT_POSITIONS: [SegmentPosition; 4] = [
    SegmentPosition::new(IsLast::No, IsFirst::No),
    SegmentPosition::new(IsLast::No, IsFirst::Yes),
    SegmentPosition::new(IsLast::Yes, IsFirst::No),
    SegmentPosition::new(IsLast::Yes, IsFirst::Yes),
];

/// Cache-filename suffix for `(is_last, is_first)`, e.g. `git_last`; shells MUST match.
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
/// # Errors
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

/// Render a git status as a prompt in every dialect.
/// # Errors
/// Returns an error if prompt rendering fails.
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
/// # Errors
/// Returns an error if prompt rendering fails.
fn render_language_prompts(
    languages: &[LanguageInfo],
    config: &Config,
    theme: &ThemeConfig,
    pos: SegmentPosition,
    prev_bg: Option<Color>,
    palette: &Palette,
) -> Result<DialectPrompts> {
    let ctx = RenderContext::new(config, theme, pos)
        .with_palette(palette.clone())
        .with_prev_colors(None, prev_bg);

    let response = Response::Language {
        languages: languages.to_vec(),
    };
    render_all_dialects(&response, &ctx)
}

/// GPY base cache directory (`~/.cache/gpy`, XDG, or `%LOCALAPPDATA%\gpy`).
/// `LOCALAPPDATA` is `cfg(windows)`-only: WSL may inherit a `C:\...` value.
/// # Errors
/// Returns an error if neither `XDG_CACHE_HOME` nor `HOME`/`LOCALAPPDATA` is set.
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

/// Instant-prompt cache directory (`<base>/instant-prompts/`).
/// # Errors
/// Returns an error if the base cache directory cannot be determined.
fn get_instant_cache_dir() -> Result<PathBuf> {
    get_gpy_cache_dir().map(|d| d.join("instant-prompts"))
}

/// Age (by mtime) after which the startup sweep removes an unused entry.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
const CACHE_ENTRY_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Age after which a `.tmp-*` file is an orphan from a killed writer.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
const ORPHAN_TMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Remove stale entries and orphaned temp files under `dir`, ignoring errors.
/// Returns `(entries_removed, orphans_removed)`.
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
            // Future mtime (clock skew or in-flight write): leave it.
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

/// Whether `entry` is a chunk directory ([`CACHE_KEY_CHUNK_CHARS`] chars, not a symlink).
fn is_chunk_dir(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|t| t.is_dir())
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.chars().count() == CACHE_KEY_CHUNK_CHARS)
}

/// One directory level of [`InstantPromptCache::clear_unrefreshable_files`].
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

        // `{key}.{base}.{token}.{ext}`: base, token and ext contain no dot, but
        // the key may, so strip from the right.
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

/// Delete the `{prefix}{base}.{token}.{ext}` files in `dir` whose `base` is one
/// of `bases`. Base, token and ext contain no dot, so a longer key sharing
/// `prefix` leaves extra components and never matches.
fn remove_git_files_in(dir: &Path, prefix: &str, bases: &[String]) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // Nothing was ever cached under this (chunked) key.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            debug_log!(
                "cache",
                "Could not scan {} to remove git caches: {e}",
                dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(tail) = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .and_then(|name| name.strip_prefix(prefix))
        else {
            continue;
        };
        let mut parts = tail.split('.');
        if let (Some(base), Some(_token), Some(ext), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
            && bases.iter().any(|git_base| git_base == base)
            && PromptDialect::ALL
                .iter()
                .any(|dialect| dialect.cache_ext() == ext)
            && let Err(e) = std::fs::remove_file(&path)
        {
            debug_log!(
                "cache",
                "Could not remove git cache {}: {e}",
                path.display()
            );
        }
    }
}

/// Cache-filename token of a context-free render (no `prev_bg`).
const NO_PREV_BG_TOKEN: &str = "none";

/// Filename token for the `prev_bg` an entry was rendered with: `none` when
/// absent, else non-alphanumeric bytes → `_`. Never contains `.`.
/// The shells' `__gpy_prev_bg_token` MUST match.
fn prev_bg_token(prev_bg: Option<&str>) -> String {
    match prev_bg {
        Some(s) if !s.is_empty() => s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
        _ => NO_PREV_BG_TOKEN.to_owned(),
    }
}

/// Max escaped key length on Windows before truncation + hash-tail; leaves
/// headroom under the 255-char filename limit for suffix, token and ext.
const WINDOWS_CACHE_KEY_MAX_LEN: usize = 200;

/// Convert a path to an injective cache key: `_` is escaped first (to `__`), then `/` `_s`, `\` `_b`, `:` `_c`, space `_w`.
/// The Unix output MUST match the shells' `__gpy_path_to_cache_key` byte-for-byte.
/// Long keys are chunked on disk by [`cache_key_path`].
fn path_to_cache_key(path: &Path) -> String {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    path_to_cache_key_for(&path.to_string_lossy(), os)
}

/// Longest Unix cache key stored as one flat filename; longer keys are chunked
/// so the name plus `.{suffix}.{token}.{ext}` stays under `NAME_MAX`.
const FLAT_CACHE_KEY_MAX_CHARS: usize = 200;

/// Characters per key chunk: ≤200 bytes even at 4 bytes/char, leaving room for the tail.
const CACHE_KEY_CHUNK_CHARS: usize = 50;

/// Longest filename component Linux and macOS accept, in bytes.
const NAME_MAX_BYTES: usize = 255;

/// [`cache_key_path_for`] for this platform.
fn cache_key_path(key: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    cache_key_path_for(key, os)
}

/// Relative on-disk stem: the key up to [`FLAT_CACHE_KEY_MAX_CHARS`], else
/// 50-character chunks joined by `/`. The shells' `__gpy_chunk_cache_key` MUST match (see
/// `tests/fixtures/cache_key_vectors.tsv`). Windows keys are already capped and
/// returned unchanged.
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

/// Directory and filename of `{cache_key_path(key)}.{tail}` under `cache_dir`.
fn cache_file_parts(cache_dir: &Path, key: &str, tail: &str) -> (PathBuf, String) {
    let stem = cache_key_path(key);
    match stem.rsplit_once('/') {
        Some((chunk_dirs, last)) => (cache_dir.join(chunk_dirs), format!("{last}.{tail}")),
        None => (cache_dir.to_path_buf(), format!("{stem}.{tail}")),
    }
}

/// [`path_to_cache_key`] for an explicit [`Os`](crate::paths::Os); only the Unix arm must match the shells.
fn path_to_cache_key_for(path_str: &str, os: crate::paths::Os) -> String {
    // `_` MUST be first so later tokens are not re-escaped.
    let escaped = path_str
        .replace('_', "__")
        .replace('/', "_s")
        .replace('\\', "_b")
        .replace(':', "_c")
        .replace(' ', "_w");

    match os {
        crate::paths::Os::Unix => escaped,
        // Verbatim `\\?\` paths require escaping Windows-reserved characters too;
        // done before case-folding.
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

/// Windows-only: case-fold (case-insensitive filesystem) and, above
/// [`WINDOWS_CACHE_KEY_MAX_LEN`], truncate with a hash-tail of the full key.
/// Escape tokens are lowercase, so folding cannot alter them.
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

/// Project root for a path (git root or the path itself), matching the shells.
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
mod tests;
