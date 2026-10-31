//! Instant prompt caching: shells read pre-rendered prompts the daemon keeps fresh.
//!
//! Files live in `~/.cache/gpy/instant-prompts/{key}.{variant}.{token}.{ext}`
//! ([`CacheFileName`]): `{key}` from [`path_to_cache_key`], chunked into
//! directories when long, `{variant}` the segment and its position, `{token}`
//! the [`prev_bg_token`] (the ANSI bakes in the `fg:prev_bg` chevron), `{ext}`
//! the [`PromptDialect::cache_ext`]. The `git_bg` variant holds the colour the git
//! pill leaves for the next chevron, in the dialect's shell spelling.

pub(super) mod files;
mod key;
mod name;
mod sweep;

use crate::cache::bounded::{INSTANT_PROMPT_LAST_WRITTEN_CAPACITY, evict_to_capacity};
use crate::config::Config;
use crate::debug_log;
use crate::formatter::{Formatter, PromptDialect, PromptFormatter, RenderContext, SegmentPosition};
use crate::git::RepositoryStatus;
use crate::ipc::{LanguageInfo, Response};
use crate::shell::Shell;
use crate::sync_ext::{MutexExt, RwLockExt};
use crate::template::{Color, Palette, parse_color};
use crate::theme::export::resolve_shell_color;
use crate::theme::{GitState, ThemeConfig};
use crate::{Error, Result};
use files::{create_private_dir, get_instant_cache_dir, write_atomic};
use key::{NAME_MAX_BYTES, cache_file_parts, find_project_root, path_to_cache_key};
use name::{
    CacheFileName, CacheKind, NO_PREV_BG_TOKEN, SEGMENT_POSITIONS, parse_cache_file_name,
    prev_bg_token,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::SystemTime;
use sweep::{clear_unrefreshable_files, prune_stale_entries, remove_git_files};

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
    /// Content and write-recency tick per [`CacheFileName`] (unchunked key).
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
    /// Temp directory backing `cache_dir` for [`Self::new_for_test`]; removed on drop.
    #[cfg(any(test, feature = "test-support"))]
    _temp_dir: Option<RemoveDirOnDrop>,
}

/// Removes its directory tree when dropped.
#[cfg(any(test, feature = "test-support"))]
struct RemoveDirOnDrop(PathBuf);

#[cfg(any(test, feature = "test-support"))]
impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            crate::debug_log!("cache", "Failed to remove {}: {e}", self.0.display());
        }
    }
}

impl InstantPromptCache {
    fn with_dir(cache_dir: PathBuf) -> Self {
        Self {
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
            #[cfg(any(test, feature = "test-support"))]
            _temp_dir: None,
        }
    }

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
        let (entries, orphans) = prune_stale_entries(&cache_dir, SystemTime::now());
        if entries > 0 || orphans > 0 {
            crate::debug_log!(
                "cache",
                "Pruned {entries} stale entries and {orphans} orphans from instant-prompt cache"
            );
        }
        Ok(Self::with_dir(cache_dir))
    }

    /// A cache in a fresh temp directory removed when the cache drops (`new`
    /// needs `HOME`/XDG).
    /// # Panics
    /// Panics if the directory cannot be created.
    #[cfg(any(test, feature = "test-support"))]
    #[expect(
        clippy::expect_used,
        reason = "test-only helper; an unusable temp dir is fatal"
    )]
    #[must_use]
    pub fn new_for_test() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("gpy-test-instant-cache-{}-{n}", std::process::id()));
        let guard = RemoveDirOnDrop(dir.clone());
        let cache = Self::new_in_dir(dir).expect("instant cache in a temp directory");
        Self {
            _temp_dir: Some(guard),
            ..cache
        }
    }

    /// A cache rooted at `cache_dir`, created owner-only if missing.
    /// # Errors
    /// Returns an error if the cache directory cannot be created.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new_in_dir(cache_dir: PathBuf) -> Result<Self> {
        create_private_dir(&cache_dir)?;
        Ok(Self::with_dir(cache_dir))
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
        let key = path_to_cache_key(&find_project_root(path));
        let rendered = if config.git.show_upstream {
            status.clone()
        } else {
            status.clone().without_upstream()
        };
        // Before the variants: a shell woken by their write finds the colour.
        let bg_changed = self.write_git_bg(&key, &rendered, theme, palette);
        let response = Response::RepositoryStatus(rendered);
        let variants_changed =
            self.write_variants(&key, CacheKind::Git, prev_bg, |position, prev_bg_color| {
                render_all_dialects(&response, config, theme, palette, position, prev_bg_color)
            })?;
        Ok(bg_changed || variants_changed)
    }

    /// Record the background the git pill of `rendered` is drawn with, per
    /// dialect, for the segment after it; `true` when any file changed. A
    /// failed write is logged: shells fall back to the clean colour.
    fn write_git_bg(
        &self,
        key: &str,
        rendered: &RepositoryStatus,
        theme: &ThemeConfig,
        palette: &Palette,
    ) -> bool {
        let bg = theme
            .segments
            .git
            .get_bg_color(GitState::from_status(rendered));
        let mut changed = false;
        for dialect in PromptDialect::ALL {
            let shell = match dialect {
                PromptDialect::Ansi => Shell::Fish,
                PromptDialect::BashPrompt => Shell::Bash,
                PromptDialect::ZshPrompt => Shell::Zsh,
            };
            let name = CacheFileName {
                key,
                kind: CacheKind::GitBg,
                position: SEGMENT_POSITIONS[0],
                token: NO_PREV_BG_TOKEN,
                dialect,
            };
            match self.write_cache_file(&name, &resolve_shell_color(shell, palette, bg)) {
                Ok(wrote) => changed |= wrote,
                Err(e) => debug_log!("cache", "Could not write git background cache: {e}"),
            }
        }
        changed
    }

    /// Write every language variant, dialect and known `prev_bg` context;
    /// returns `true` when any content changed. `languages` are resolved
    /// display entries, so writing never probes a version.
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
        let response = Response::Language {
            languages: languages.to_vec(),
        };
        let key = path_to_cache_key(&find_project_root(path));
        self.write_variants(&key, CacheKind::Lang, prev_bg, |position, prev_bg_color| {
            render_all_dialects(&response, config, theme, palette, position, prev_bg_color)
        })
    }

    /// Write `kind`'s cache for the project keyed `key` in every position, as
    /// `render` renders it, for each known `prev_bg` context: background
    /// writers pass `None` but must refresh them all. Returns `true` when any
    /// content changed.
    /// # Errors
    /// Returns the first render or write error when no file was written;
    /// errors beside a successful write are logged.
    fn write_variants(
        &self,
        key: &str,
        kind: CacheKind,
        prev_bg: Option<&str>,
        render: impl Fn(SegmentPosition, Option<Color>) -> Result<DialectPrompts>,
    ) -> Result<bool> {
        let mut wrote_any = false;
        let mut first_err = None;
        for context in self.known_contexts(key, prev_bg) {
            let token = prev_bg_token(context.as_deref());
            let color = context.as_deref().and_then(|s| match parse_color(s) {
                Ok(c) => Some(c),
                Err(e) => {
                    crate::debug_log!(
                        "cache",
                        "Failed to parse prev_bg color '{s}' for instant prompt: {e}"
                    );
                    None
                }
            });
            for position in SEGMENT_POSITIONS {
                let prompts = match render(position, color.clone()) {
                    Ok(prompts) => prompts,
                    Err(e) => {
                        first_err.get_or_insert(e);
                        continue;
                    }
                };
                for (dialect, prompt) in prompts {
                    let name = CacheFileName {
                        key,
                        kind,
                        position,
                        token: &token,
                        dialect,
                    };
                    match self.write_cache_file(&name, &prompt) {
                        Ok(wrote) => wrote_any |= wrote,
                        Err(e) => {
                            first_err.get_or_insert(e);
                        }
                    }
                }
            }
        }
        match first_err {
            Some(e) if !wrote_any => Err(e),
            Some(e) => {
                debug_log!("cache", "Some cache variants were not written: {e}");
                Ok(true)
            }
            None => Ok(wrote_any),
        }
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
            |entry| entry.tick,
        );
        drop(guard);
        known
    }

    /// Remove language and context-specific cache files, which a restarted agent
    /// (contexts are in-memory) would never refresh. Best-effort.
    pub fn clear_unrefreshable_files(&self) {
        clear_unrefreshable_files(&self.cache_dir);

        self.last_written
            .lock_or_recover()
            .entries
            .retain(|name, _written| {
                !parse_cache_file_name(name).is_some_and(|parsed| parsed.is_unrefreshable())
            });
    }

    /// Delete every git cache file of the repository at `path` (each variant,
    /// `prev_bg` token and dialect, including contexts no longer tracked) so
    /// shells stop serving its git segment. Best-effort.
    pub fn remove_git(&self, path: &Path) {
        let root = find_project_root(path);
        let key = path_to_cache_key(&root);

        // Held across the deletion so the record keeps matching the disk.
        let mut last_written = self.last_written.lock_or_recover();
        last_written.entries.retain(|name, _written| {
            !parse_cache_file_name(name)
                .is_some_and(|parsed| parsed.kind.is_git() && parsed.key == key)
        });
        remove_git_files(&self.cache_dir, &key);
        self.suppressed_git.lock_or_recover().insert(key, root);
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
    fn write_cache_file(&self, name: &CacheFileName<'_>, content: &str) -> Result<bool> {
        let tail = name.tail();
        let map_key = format!("{}.{tail}", name.key);
        let (dir, file_name) = cache_file_parts(&self.cache_dir, name.key, &tail);
        if file_name.len() > NAME_MAX_BYTES {
            self.note_unwritable_key(name.key);
            return Ok(false);
        }
        let cache_file = dir.join(&file_name);

        // Held across check, touch, write and record so the record equals disk.
        let mut cache = self.last_written.lock_or_recover();
        if name.kind.is_git() && self.suppressed_git.lock_or_recover().contains_key(name.key) {
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

    /// Cache file path a shell in `dialect` should read for a directory's
    /// `variant` (e.g. `git_last`).
    /// # Errors
    /// Returns an error if the cache directory cannot be determined.
    pub fn cache_file_for_dir(
        dir: &Path,
        variant: &str,
        prev_bg: Option<&str>,
        dialect: PromptDialect,
    ) -> Result<PathBuf> {
        let cache_dir = get_instant_cache_dir()?;
        let key = path_to_cache_key(dir);
        let token = prev_bg_token(prev_bg);
        let ext = dialect.cache_ext();
        let (parent, file_name) =
            cache_file_parts(&cache_dir, &key, &format!("{variant}.{token}.{ext}"));
        Ok(parent.join(file_name))
    }
}

/// A rendered cache entry for each [`PromptDialect`].
type DialectPrompts = Vec<(PromptDialect, String)>;

/// Render `response` at `position`, after a segment with background
/// `prev_bg`, once per [`PromptDialect`].
/// # Errors
/// Returns an error if prompt rendering fails.
fn render_all_dialects(
    response: &Response,
    config: &Config,
    theme: &ThemeConfig,
    palette: &Palette,
    position: SegmentPosition,
    prev_bg: Option<Color>,
) -> Result<DialectPrompts> {
    let ctx = RenderContext::new(config, theme, position)
        .with_palette(palette.clone())
        .with_prev_colors(None, prev_bg);
    PromptDialect::ALL
        .iter()
        .map(|&dialect| {
            Ok((
                dialect,
                PromptFormatter::new(dialect).render(response, &ctx)?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod git_bg_tests;
#[cfg(test)]
mod tests;
