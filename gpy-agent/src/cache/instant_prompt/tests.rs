use super::*;
use crate::sync_ext::MutexExt;

/// A resolved Rust entry, so language writes never probe a real `rustc`.
fn rust_languages() -> Vec<crate::ipc::LanguageInfo> {
    vec![crate::ipc::LanguageInfo {
        name: "rust".to_owned(),
        version: Some("1.99.0".to_owned()),
        color: crate::config::types::ColorSpec::new("#dea584").expect("color"),
    }]
}

impl InstantPromptCache {
    /// Get the cache file path for a given cache key, suffix and dialect
    /// (the writer needs the directory and name separately).
    fn cache_file_path(&self, key: &str, suffix: &str, dialect: PromptDialect) -> PathBuf {
        let ext = dialect.cache_ext();
        let (dir, file_name) = cache_file_parts(&self.cache_dir, key, &format!("{suffix}.{ext}"));
        dir.join(file_name)
    }
}

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
    // A surviving temp file means the writer died between write and rename.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let orphan = aged_file(
        dir.path(),
        ".tmp-1234-5",
        ORPHAN_TMP_MAX_AGE.saturating_add(Duration::from_secs(60)),
    );
    // Past the orphan threshold but not the entry threshold.
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

    let leftover_count = std::fs::read_dir(temp_dir.path())
        .expect("read dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(
        leftover_count, 0_usize,
        "stray temp files remain after atomic write"
    );

    let content =
        std::fs::read_to_string(cache.cache_file_path("repo", "git", PromptDialect::Ansi))
            .expect("read");
    assert_eq!(content, "hello world");
}

#[test]
fn last_written_evicts_least_recently_written() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let write = |key: &str, content: &str| {
        cache
            .write_cache_file(key, "git", PromptDialect::Ansi, content)
            .expect("write")
    };
    // Oldest first: one past capacity evicts exactly the oldest record.
    for i in 0..=INSTANT_PROMPT_LAST_WRITTEN_CAPACITY {
        assert!(write(&format!("k{i:04}"), "c"));
    }
    let newest = format!("k{INSTANT_PROMPT_LAST_WRITTEN_CAPACITY:04}");

    assert!(!write(&newest, "c"), "newest key is still recorded");
    assert!(!write("k0512", "c"), "a middle key is still recorded");
    assert!(write("k0000", "c"), "the oldest key was evicted");
    assert!(write(&newest, "d"), "changed content is written");
}

#[test]
fn known_contexts_evicts_least_recently_used_key() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    for i in 0..INSTANT_PROMPT_LAST_WRITTEN_CAPACITY {
        cache.known_contexts(&format!("k{i:04}"), Some("red"));
    }
    // Using the oldest key makes k0001 the least recently used.
    cache.known_contexts("k0000", Some("blue"));
    cache.known_contexts("new", None);

    assert_eq!(
        cache.known_contexts("k0000", Some("red")),
        vec![Some("red".to_owned()), Some("blue".to_owned())],
        "the recently used key keeps its contexts"
    );
    assert_eq!(
        cache.known_contexts("k1023", Some("green")),
        vec![Some("red".to_owned()), Some("green".to_owned())],
        "the newest key keeps its contexts"
    );
    assert_eq!(
        cache.known_contexts("k0001", Some("green")),
        vec![Some("green".to_owned())],
        "the least recently used key was evicted"
    );
}

/// Permission bits of `path`.
#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
#[test]
fn instant_cache_dir_and_files_are_private() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let parent = temp_dir.path().join("gpy");
    let root = parent.join("instant-prompts");
    std::fs::DirBuilder::new()
        .mode(0o755)
        .recursive(true)
        .create(&root)
        .expect("create root");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let cache = InstantPromptCache::new_in_dir(root.clone()).expect("cache");
    assert_eq!(mode_of(&root), 0o700, "instant-prompts root is owner-only");
    assert_eq!(mode_of(&parent), 0o755, "the parent keeps its mode");

    assert!(
        cache
            .write_cache_file("repo", "git", PromptDialect::Ansi, "content")
            .expect("write")
    );
    let file = cache.cache_file_path("repo", "git", PromptDialect::Ansi);
    assert_eq!(mode_of(&file), 0o600, "cache files are owner-only");
}

#[cfg(unix)]
#[test]
fn recreated_cache_dir_is_private() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let root = temp_dir.path().join("instant-prompts");
    let cache = InstantPromptCache::new_in_dir(root.clone()).expect("cache");
    std::fs::remove_dir_all(&root).expect("remove root");

    assert!(
        cache
            .write_cache_file("repo", "git", PromptDialect::Ansi, "content")
            .expect("write")
    );
    assert_eq!(mode_of(&root), 0o700, "re-created root is owner-only");
    let file = cache.cache_file_path("repo", "git", PromptDialect::Ansi);
    assert_eq!(mode_of(&file), 0o600, "cache files are owner-only");
}

/// Direct tests of [`write_atomic`].
mod write_atomic_tests {
    use super::*;

    #[test]
    fn writes_new_content() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        write_atomic(temp_dir.path(), "target.txt", "hello").expect("write");

        let content = std::fs::read_to_string(temp_dir.path().join("target.txt")).expect("read");
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

    /// The target is the only file left: the temp file was renamed, not copied.
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

    /// A stray `.tmp-` file from an earlier crash must not interfere.
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

    /// Writing into a missing directory fails and never creates the target.
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
    // A missing file is rewritten even when the record matches.
    std::fs::remove_file(cache.cache_file_path("repo", "git", PromptDialect::Ansi))
        .expect("remove");
    assert!(
        cache
            .write_cache_file("repo", "git", PromptDialect::Ansi, "beta")
            .expect("write"),
        "rewriting a missing file should report a change"
    );
}

// Invariant: the dir equals a fresh render of the latest inputs, touched files
// have fresh mtimes, and no temp files remain.

/// One step applied to the cache under test.
enum Op {
    /// Write `content` for `(key, suffix)` (Ansi dialect).
    Write(&'static str, &'static str, &'static str),
    /// Backdate every file by this many seconds.
    Age(u64),
    /// `threads` writers released together by a `Barrier`, thread `i`
    /// writing `versions[i % versions.len()]` for `(key, suffix)`.
    Concurrent(&'static str, &'static str, &'static [&'static str], usize),
    /// Remove the whole cache directory, as `rm -rf ~/.cache/gpy` would.
    DeleteDir,
    /// Write all four git variants for a `key_len`-byte key (chunked above 200).
    WriteVariants(usize, &'static str),
    /// Age every file past `CACHE_ENTRY_MAX_AGE` and sweep; the dir must empty.
    #[cfg(unix)]
    Sweep,
}

/// Independent model of the on-disk name of `(key, suffix)`; Windows never chunks.
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
                    .lock_or_recover()
                    .entries
                    .get(&format!("{key}:{suffix}.ansi"))
                    .map(|written| written.content.clone())
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
    // 225-byte key -> 250-byte longest variant name; its temp name must stay under NAME_MAX.
    h.apply(&Op::WriteVariants(225, "x"));
    h.apply(&Op::Age(60));
    h.apply(&Op::WriteVariants(225, "x"));
    h.apply(&Op::WriteVariants(225, "y"));
}

// Chunking is a Unix-only scheme: native Windows keys are capped by
// `windows_harden_cache_key` and a 300-byte key is simply skipped.
#[cfg(unix)]
#[test]
fn instant_cache_writes_and_sweeps_a_chunked_300_byte_key() {
    let mut h = CacheHarness::new();
    // 300-byte key: flat it would be a 325-byte name; chunked it is
    // 5 directories of 50 plus a 75-byte final name.
    h.apply(&Op::WriteVariants(300, "x"));
    h.apply(&Op::Age(60));
    h.apply(&Op::WriteVariants(300, "x"));
    h.apply(&Op::WriteVariants(300, "y"));
    h.apply(&Op::Write("repo", "git.none", "x"));
    // The sweep removes chunked entries and their emptied directories.
    h.apply(&Op::Sweep);
    // Chunk directories are recreated by the write's retry.
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
    let cache = InstantPromptCache::new_in_dir(dir.path().join("instant-prompts")).expect("cache");
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

/// Paths differing only in separator/underscore placement must produce
/// distinct keys.
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

/// The Unix escape scheme is injective: the path is recoverable from the key.
/// (The Windows arm case-folds, so it is not.)
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

/// Pins the exact `Os::Unix` output; it must match the shells byte-for-byte.
#[test]
fn test_path_to_cache_key_unix_characterization() {
    let cases: [(&str, &str); 6] = [
        ("/Users/foo/project", "_sUsers_sfoo_sproject"),
        ("/tmp/what?proj", "_stmp_swhat?proj"),
        ("/a*b<c>d\"e|f", "_sa*b<c>d\"e|f"),
        ("/home/user/my project", "_shome_suser_smy_wproject"),
        ("C:\\Users\\foo\\bar", "C_c_bUsers_bfoo_bbar"),
        (
            "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/track/demo/implementation/work",
            "_sa_svery_sdeeply_snested_sdirectory_sstructure_sthat_sgoes_son_sand_son_sand_son_sand_son_sand_son_sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_slogic_sand_smake_ssure_snothing_schanges_sunexpectedly_swhen_sthis_sfunction_sgains_swindows_shardening_sas_spart_sof_strack_sdemo_simplementation_swork",
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
/// into 50-character chunks, counted in characters.
#[test]
fn test_cache_key_path_unix_characterization() {
    let unix = crate::paths::Os::Unix;
    let flat = "k".repeat(FLAT_CACHE_KEY_MAX_CHARS);
    assert_eq!(cache_key_path_for(&flat, unix), flat.as_str());
    let deep = path_to_cache_key_for(
        "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/track/demo/implementation/work",
        unix,
    );
    assert_eq!(
        cache_key_path_for(&deep, unix),
        "_sa_svery_sdeeply_snested_sdirectory_sstructure_st/hat_sgoes_son_sand_son_sand_son_sand_son_sand_son_/sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_s/logic_sand_smake_ssure_snothing_schanges_sunexpect/edly_swhen_sthis_sfunction_sgains_swindows_sharden/ing_sas_spart_sof_strack_sdemo_simplementation_swo/rk"
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
/// skipped without an error.
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
    assert_eq!(cache.unwritable_keys.lock_or_recover().len(), 1);
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

/// Shared vectors (`tests/fixtures/cache_key_vectors.tsv`) also read by the
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
        let (Some(raw_input), Some(expected), Some(stem)) = (cols.next(), cols.next(), cols.next())
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

/// The shared vector's `stem` is the relative cache-file path the
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

/// Differently-cased paths to one Windows directory collide to one key.
#[test]
fn test_path_to_cache_key_windows_case_fold_collision() {
    let left = path_to_cache_key_for(r"C:\Foo\Bar", crate::paths::Os::Windows);
    let right = path_to_cache_key_for(r"C:\foo\bar", crate::paths::Os::Windows);
    assert_eq!(
        left, right,
        "differently-cased paths to the same Windows directory must share a cache key"
    );

    // The Unix arm must NOT case-fold: distinct directories would collide.
    let unix_left = path_to_cache_key_for(r"C:\Foo\Bar", crate::paths::Os::Unix);
    let unix_right = path_to_cache_key_for(r"C:\foo\bar", crate::paths::Os::Unix);
    assert_ne!(
        unix_left, unix_right,
        "the Unix arm must not case-fold -- it must stay byte-for-byte unmodified"
    );
}

/// Long Windows paths sharing a truncated prefix still get distinct keys.
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

    // Under the cap: no hash-tail.
    let short = path_to_cache_key_for("/Users/foo/project", crate::paths::Os::Windows);
    assert_eq!(short, "_susers_sfoo_sproject");
}

/// `new()` succeeds exactly when `get_gpy_cache_dir` resolves; the env is
/// read, not set (`set_var` needs unsafe).
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
    let languages = rust_languages();

    cache
        .write_language(
            &subdir,
            &languages,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
            None,
            &crate::palette::active_palette(&config),
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
    let languages = rust_languages();

    cache
        .write_language(
            &repo_root,
            &languages,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
            None,
            &crate::palette::active_palette(&config),
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
    let languages = rust_languages();

    cache
        .write_language_variants(
            &repo_root,
            &languages,
            &config,
            &theme,
            None,
            &crate::palette::active_palette(&config),
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

    // Real `{base}.{token}.{ext}` names in every dialect.
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
                "{name} must be cleared like lang/lang_last"
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
    // removed with `remove_file`; it must not abort the sweep.
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
/// directories it empties, but keeps git caches and their dirs.
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

/// `write_git` writes `is_first` variants, and only those drop `$sep_open`.
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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

/// `write_language_variants` writes all four variants; only `is_first` ones drop the cap.
#[test]
fn write_language_variants_creates_is_first_variants_and_suppresses_opening_cap() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let repo_root = temp_dir.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

    let config = Config::default();
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
    let theme = theme_mgr.get();
    let languages = rust_languages();

    cache
        .write_language_variants(
            &repo_root,
            &languages,
            &config,
            &theme,
            None,
            &crate::palette::active_palette(&config),
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

/// Build a git repo whose longest variant name is exactly 250 bytes, so a
/// name-derived temp name would exceed `NAME_MAX`.
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
    let languages = rust_languages();

    cache
        .write_language_variants(
            &repo,
            &languages,
            &config,
            &theme_mgr.get(),
            None,
            &crate::palette::active_palette(&config),
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

/// A ~300-byte key is written at the path `cache_file_for_dir` resolves.
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
    // it by hardening instead, so it never exceeds NAME_MAX there.
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
        let written = cache.cache_file_path(&key, &format!("{suffix}.none"), PromptDialect::Ansi);
        assert!(written.is_file(), "{suffix} must be written");
        // `cache_file_for_dir` resolves the same path under the
        // environment's cache root.
        if let Ok(shell_path) =
            InstantPromptCache::cache_file_for_dir(&canonical, suffix, None, PromptDialect::Ansi)
        {
            let root = get_instant_cache_dir().expect("cache dir");
            assert_eq!(
                shell_path.strip_prefix(&root).expect("under root"),
                written.strip_prefix(&cache.cache_dir).expect("under root"),
            );
        }
    }
}

/// `remove_git` deletes a repo's git caches in every variant, context and
/// dialect, for flat and chunked keys alike, but keeps its language cache and
/// the caches of a repo whose key merely extends it.
#[test]
fn remove_git_deletes_only_that_repos_git_caches() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().join("ip")).expect("cache");
    let config = Config::default();
    let palette = crate::palette::active_palette(&config);
    let write = |repo: &Path, prev_bg: Option<&str>| {
        cache
            .write_git(
                repo,
                &clean_status("main"),
                &config,
                &ThemeConfig::default(),
                prev_bg,
                &palette,
            )
            .expect("write git");
    };
    let git_files = |key: &str| {
        let mut files = Vec::new();
        for base in ["git", "git_last", "git_first", "git_first_last"] {
            for token in ["none", "_112233"] {
                for dialect in PromptDialect::ALL {
                    files.push(cache.cache_file_path(key, &format!("{base}.{token}"), dialect));
                }
            }
        }
        files
    };

    for parent in [
        temp_dir.path().join("flat"),
        temp_dir.path().join("d".repeat(160)),
    ] {
        let repo = parent.join("repo");
        let extended = parent.join("repo.x");
        for root in [&repo, &extended] {
            std::fs::create_dir_all(root.join(".git")).expect("git dir");
            write(root, None);
            write(root, Some("#112233"));
        }
        let key = path_to_cache_key(&std::fs::canonicalize(&repo).expect("canonicalize"));
        let extended_key =
            path_to_cache_key(&std::fs::canonicalize(&extended).expect("canonicalize"));
        let lang = cache.cache_file_path(&key, "lang.none", PromptDialect::Ansi);
        std::fs::write(&lang, "lang").expect("write language cache");
        assert!(
            git_files(&key).iter().all(|file| file.is_file()),
            "control: every git cache of {} is written",
            repo.display()
        );

        cache.remove_git(&repo);

        let left: Vec<_> = git_files(&key).into_iter().filter(|f| f.exists()).collect();
        assert!(
            left.is_empty(),
            "git caches must be deleted, left: {left:?}"
        );
        assert!(lang.is_file(), "the repo's language cache must be kept");
        assert!(
            git_files(&extended_key).iter().all(|file| file.is_file()),
            "a repo whose key extends the removed one must keep its git caches"
        );

        // A write already in flight when the repo was excluded must not
        // republish it; only releasing the suppression lets writes through.
        write(&repo, None);
        assert!(
            git_files(&key).iter().all(|file| !file.exists()),
            "a git write after removal must be skipped while suppressed"
        );
        let lang_before = std::fs::read_to_string(&lang).expect("read language cache");
        cache.release_git_suppression(|root| root != repo.canonicalize().expect("canonicalize"));
        write(&repo, None);
        assert!(
            git_files(&key).iter().all(|file| !file.exists()),
            "a release that still excludes the repo keeps suppressing it"
        );
        cache.release_git_suppression(|_root| true);
        write(&repo, None);
        assert!(
            git_files(&key).first().is_some_and(|file| file.is_file()),
            "a write after the suppression is released must recreate the cache"
        );
        assert_eq!(
            std::fs::read_to_string(&lang).expect("read language cache"),
            lang_before,
            "suppression never touches the language cache"
        );
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

/// The cache contains the branch name styled with ANSI escapes from the palette.
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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

/// Each dialect's cache file escapes the branch for
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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

/// A watcher write (`prev_bg = None`) still produces non-empty ANSI output.
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
    let theme_arc = theme_mgr.get();

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

/// Remapping a color used by the default git format changes the ANSI bytes.
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
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
    let theme_arc = theme_mgr.get();

    // Palette A: empty — "green" falls back to Color::Named("green") → ANSI code 42.
    let palette_a = Palette::default();

    // Palette B: remaps "green" to a specific RGB value.
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

    let formatter = create_formatter(Format::Ansi);

    let ctx_a =
        RenderContext::new(&config, &theme_arc, SegmentPosition::MIDDLE).with_palette(palette_a);
    let ctx_b =
        RenderContext::new(&config, &theme_arc, SegmentPosition::MIDDLE).with_palette(palette_b);

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

/// The `prev_bg` token is filesystem-safe, deterministic, and `none` by default.
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

/// Each `prev_bg` context resolves to its own cache file, so themes that key
/// rendering on `prev_bg` never clobber another context's output.
#[test]
fn git_cache_writes_one_file_per_prev_bg_context() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let repo_root = temp_dir.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

    let status = clean_status("main");
    let config = Config::default();
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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
    assert!(path("git.blue").exists());
    assert!(path("git.green").exists());
    assert!(path("git.none").exists());
}

/// A context-free (`None`) write goes to the `none` token file
/// and must NOT overwrite a previously written context-specific file.
#[test]
fn git_none_write_does_not_clobber_context_file() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let repo_root = temp_dir.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

    let config = Config::default();
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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

/// Language cache writes resolve one file per `prev_bg` context, like git.
#[test]
fn language_cache_writes_one_file_per_prev_bg_context() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let repo_root = temp_dir.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

    let config = Config::default();
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
    let theme = theme_mgr.get();
    let languages = rust_languages();

    cache
        .write_language_variants(
            &repo_root,
            &languages,
            &config,
            &theme,
            Some("blue"),
            &crate::palette::active_palette(&config),
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
}

/// A context-free (`None`) write refreshes every known context.
#[test]
fn none_write_refreshes_all_known_contexts() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(temp_dir.path().to_path_buf()).expect("cache");
    let repo_root = temp_dir.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".git")).expect("git dir");

    let config = Config::default();
    let theme_mgr = crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
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

    // A None write with a changed branch must refresh every context file.
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
