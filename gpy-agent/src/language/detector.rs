//! Native `GitHub` Linguist-grade language detection.
//!
//! Detection is path-first: a filename, path glob or extension that names
//! exactly one language settles the answer without opening the file. Only an
//! ambiguous or unmatched path pays for a capped read. See
//! [`detect_language`] for the full resolution order and
//! [`filters`](super::filters) for the exclusion globs the directory walk
//! applies.

use crate::Result;
use crate::language::filters;
use gengo_language::Language;
use std::path::Path;

/// Upper bound on how much of a file the fallback read pulls in.
///
/// This is `hyperpolyglot`'s `MAX_CONTENT_SIZE_BYTES`, matched deliberately:
/// #523 swapped the detector out from under an existing corpus of behavioural
/// tests, and a different read window would change which heuristics fire.
const READ_LIMIT: usize = 51_200;

/// Suffixes appended *after* a file's real extension, stripped before any
/// matcher runs.
///
/// `gengo_language::Language::pick` strips these itself, so the cheap
/// path-only pre-check in [`detect_language`] has to strip them too — otherwise
/// the two disagree on `foo.rs.bak`, and the pre-check would answer for a file
/// `pick` would have classified differently.
const STRIPPED_SUFFIX_EXTENSIONS: [&str; 2] = [".bak", ".example"];

/// Language detector over `gengo-language`'s matcher tables.
#[derive(Default)]
pub struct Detector {
    // Detection is table-driven and needs no configuration.
}

impl Detector {
    /// Detect languages in a directory
    ///
    /// Aggregates results by normalized language name (e.g., `JavaScript` and `TypeScript`
    /// are both mapped to "node") and returns lightweight metrics instead of file paths.
    #[must_use]
    pub fn detect_directory<P: AsRef<Path>>(path: P) -> Vec<DetectedLanguage> {
        Self::detect_directory_impl(path, true)
    }

    /// Benchmark-only variant that skips the per-file size lookup.
    ///
    /// Empirically, sampled Criterion runs show a meaningful amount of time in
    /// file-metadata lookups during directory detection. This variant keeps the
    /// walk, matching and normalization work identical while isolating the cost
    /// of stat-ing every detected file. Since #523 the size comes from the
    /// walk's own [`ignore::DirEntry`] rather than a second `std::fs::metadata`
    /// pass, so the saving is smaller than it was — but the call is still made
    /// per detected file, so the flag still measures something real.
    #[must_use]
    #[doc(hidden)]
    pub fn detect_directory_without_metadata<P: AsRef<Path>>(path: P) -> Vec<DetectedLanguage> {
        Self::detect_directory_impl(path, false)
    }

    fn detect_directory_impl<P: AsRef<Path>>(
        path: P,
        include_file_sizes: bool,
    ) -> Vec<DetectedLanguage> {
        let detections = walk_directory(path.as_ref(), include_file_sizes);
        languages_from_aggregates(aggregate_detections(detections))
    }

    /// Detect languages using the configured [`DetectionMode`].
    ///
    /// `Content` runs the recursive content scan ([`Self::detect_directory`]);
    /// `Markers` shows a language only when its project markers/extensions are
    /// present in the directory ([`Self::detect_directory_markers`]); `Hybrid`
    /// runs the content scan gated by marker *files*
    /// ([`Self::detect_directory_hybrid`]).
    #[must_use]
    pub fn detect_directory_for_mode<P: AsRef<Path>>(
        path: P,
        mode: crate::config::types::DetectionMode,
    ) -> Vec<DetectedLanguage> {
        match mode {
            crate::config::types::DetectionMode::Content => Self::detect_directory(path),
            crate::config::types::DetectionMode::Markers => Self::detect_directory_markers(path),
            crate::config::types::DetectionMode::Hybrid => Self::detect_directory_hybrid(path),
        }
    }

    /// Detect languages via the `hybrid` strategy: content-scan prevalence
    /// ([`Self::detect_directory`]), gated to only the languages whose project
    /// marker *file* (e.g. `Cargo.toml`, not just a `.rs` extension) is present
    /// in the directory.
    ///
    /// Gating deliberately uses only
    /// [`marker_files`](crate::language::metadata::LanguageMeta::marker_files),
    /// not `marker_extensions`: a lone helper script already carries the
    /// extension that got it content-detected in the first place, so an
    /// extension check adds no gating signal over content alone (see
    /// [`Self::detect_directory_markers`], which — correctly for `Markers`
    /// mode — treats that same extension as sufficient evidence on its own).
    /// Hybrid's whole point is distinguishing "this repo is a project of X"
    /// from "this repo merely contains an X file", so only a real project
    /// marker counts. Best of both signals — a Rust repo with a stray `.py`
    /// helper script surfaces only `rust`, since `python` has no project
    /// marker file here.
    #[must_use]
    pub fn detect_directory_hybrid<P: AsRef<Path>>(path: P) -> Vec<DetectedLanguage> {
        let path_ref = path.as_ref();
        let content = Self::detect_directory(path_ref);
        if content.is_empty() {
            return content;
        }
        let marker_file_languages = Self::directory_marker_file_languages(path_ref);
        content
            .into_iter()
            .filter(|lang| marker_file_languages.contains(lang.name.as_str()))
            .collect()
    }

    /// Canonical names of languages whose project marker file (not extension)
    /// is present among `path`'s immediate directory entries.
    fn directory_marker_file_languages(path: &Path) -> std::collections::HashSet<&'static str> {
        marker_evidence(path, false).keys().copied().collect()
    }

    /// Detect languages for an externally supplied path (IPC request, CLI
    /// `--cwd`, wizard facts, …), honoring the configured [`DetectionMode`]
    /// but never running the expensive recursive `Content` (or `Hybrid`, which
    /// runs the same scan) scan outside a discoverable git repository, or when
    /// the tree is too large to walk within [`CONTENT_SCAN_FILE_BUDGET`].
    ///
    /// `Content` mode ([`Self::detect_directory`]) walks and matches every
    /// non-ignored file under `path` with no repo-boundary or size cap of its
    /// own. When `path` isn't inside a git repo — a shell sitting at `$HOME`
    /// or another large, non-project directory — that scan can run for minutes
    /// across gigabytes of data (#390). A legitimately huge monorepo can still
    /// trigger the same failure mode from inside a real repo, so the file count
    /// is checked *before* the scan starts rather than bounded mid-walk. GPY
    /// owns that walk outright since #523 and could in principle cancel it
    /// through [`ignore::WalkState::Quit`], but converting this pre-check into
    /// mid-walk cancellation changes the `Content`→`Markers` fallback semantics
    /// the tests below assert, so it stays a pre-check and the conversion is
    /// tracked separately on #391. `Hybrid` starts from the same content scan
    /// ([`Self::detect_directory_hybrid`]) so it falls back to `Markers` under
    /// the identical conditions. Callers handling a real, externally supplied path
    /// MUST use this instead of calling [`Self::detect_directory_for_mode`]
    /// directly. Callers that already know `path` is a real, registered
    /// repository root (e.g. the file watcher) may keep calling
    /// [`Self::detect_directory_for_mode`] directly, since the repo check
    /// here would be a redundant no-op for them.
    #[must_use]
    pub fn detect_directory_bounded<P: AsRef<Path>>(
        path: P,
        mode: crate::config::types::DetectionMode,
    ) -> Vec<DetectedLanguage> {
        Self::detect_directory_bounded_with_budget(path, mode, CONTENT_SCAN_FILE_BUDGET)
    }

    /// Test/benchmark-only variant of [`Self::detect_directory_bounded`] with
    /// an injectable file-count budget, so tests can exercise the fallback
    /// threshold without materializing tens of thousands of files on disk.
    #[must_use]
    #[doc(hidden)]
    pub fn detect_directory_bounded_with_budget<P: AsRef<Path>>(
        path: P,
        mode: crate::config::types::DetectionMode,
        budget: usize,
    ) -> Vec<DetectedLanguage> {
        let path_ref = path.as_ref();
        let needs_fallback =
            matches!(
                mode,
                crate::config::types::DetectionMode::Content
                    | crate::config::types::DetectionMode::Hybrid
            ) && (crate::watcher::multi_repo::MultiRepoWatcher::find_git_root(path_ref).is_none()
                || exceeds_file_budget(path_ref, budget));
        let bounded_mode = if needs_fallback {
            crate::config::types::DetectionMode::Markers
        } else {
            mode
        };
        Self::detect_directory_for_mode(path_ref, bounded_mode)
    }

    /// Detect languages by the presence of project markers and file extensions.
    ///
    /// Scans only the immediate directory entries (not recursively): a language
    /// is surfaced when one of its [`marker_files`](crate::language::metadata::LanguageMeta::marker_files)
    /// exists or a child file carries one of its
    /// [`marker_extensions`](crate::language::metadata::LanguageMeta::marker_extensions).
    /// Results are ordered by evidence count (descending), then name, so the
    /// formatter's "top N" cap is deterministic. Unreadable directories yield an
    /// empty result.
    #[must_use]
    pub fn detect_directory_markers<P: AsRef<Path>>(path: P) -> Vec<DetectedLanguage> {
        let evidence = marker_evidence(path.as_ref(), true);

        let mut languages: Vec<DetectedLanguage> = evidence
            .into_iter()
            .map(|(name, count)| DetectedLanguage {
                name: name.to_owned(),
                // Marker presence is binary; confidence is not prevalence-based.
                confidence: 1.0_f32,
                file_count: count,
                total_bytes: 0_u64,
            })
            .collect();
        languages.sort_by(|a, b| {
            b.file_count
                .cmp(&a.file_count)
                .then_with(|| a.name.cmp(&b.name))
        });
        languages
    }

    /// Detect the language of a single file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file's contents had to be read to resolve an
    /// ambiguous or unmatched path and that read failed.
    pub fn detect_file<P: AsRef<Path>>(path: P) -> Result<Option<DetectedLanguage>> {
        let file_path = path.as_ref();

        let detected = detect_language(file_path).map_err(|error| {
            crate::Error::language(format!(
                "language detection failed for {}: {error}",
                file_path.display()
            ))
        })?;
        let Some(language) = detected else {
            return Ok(None);
        };

        let file_size = std::fs::metadata(file_path).map_or(0, |metadata| metadata.len());
        Ok(Some(DetectedLanguage {
            name: normalize_language_name(language.name()).to_owned(),
            confidence: 1.0, // Single-file detection has no prevalence to weigh.
            file_count: 1,
            total_bytes: file_size,
        }))
    }
}

/// Scan `path`'s immediate entries once, counting how many match each
/// language's marker files.
///
/// Also counts marker extensions when `include_extensions` is true. Returns
/// a language → hit-count map; an empty map if `path` can't be read.
#[must_use]
fn marker_evidence(
    path: &Path,
    include_extensions: bool,
) -> std::collections::HashMap<&'static str, usize> {
    use crate::language::metadata::LANGUAGES;
    use std::collections::HashMap;

    let Ok(entries) = std::fs::read_dir(path) else {
        return HashMap::new();
    };

    let mut evidence: HashMap<&'static str, usize> = HashMap::new();
    for entry in entries.flatten() {
        let raw_name = entry.file_name();
        let name = raw_name.to_string_lossy();
        let extension = Path::new(name.as_ref())
            .extension()
            .and_then(|ext| ext.to_str());
        for language in LANGUAGES {
            let file_match = language
                .marker_files
                .iter()
                .any(|marker| marker.eq_ignore_ascii_case(name.as_ref()));
            let extension_match = include_extensions
                && extension.is_some_and(|ext| {
                    language
                        .marker_extensions
                        .iter()
                        .any(|marker| marker.eq_ignore_ascii_case(ext))
                });
            if file_match || extension_match {
                let count = evidence.entry(language.canonical).or_insert(0_usize);
                *count = count.saturating_add(1_usize);
            }
        }
    }
    evidence
}

/// Raw language detection result
#[derive(Debug, Clone)]
pub struct DetectedLanguage {
    /// Language name (normalized, e.g. "node" for JavaScript/TypeScript)
    pub name: String,
    /// Detection confidence (0.0 to 1.0)
    pub confidence: f32,
    /// Number of files that matched this language
    pub file_count: usize,
    /// Total size in bytes of all matched files
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct LanguageMetrics {
    file_count: usize,
    total_bytes: u64,
}

impl LanguageMetrics {
    #[must_use]
    const fn combine(self, other: Self) -> Self {
        Self {
            file_count: self.file_count.saturating_add(other.file_count),
            total_bytes: self.total_bytes.saturating_add(other.total_bytes),
        }
    }
}

/// Number of worker threads the detection walk runs on.
///
/// `hyperpolyglot` read this from `num_cpus::get()` (overridable through its
/// own `HYPLY_THREADS` environment variable, which GPY deliberately does not
/// carry over). `std::thread::available_parallelism` is the standard-library
/// equivalent and costs no dependency.
fn walk_threads() -> usize {
    std::thread::available_parallelism().map_or(1_usize, std::num::NonZeroUsize::get)
}

/// Walks `path` in parallel and yields one `(language, bytes)` pair per file
/// that resolves to a language.
///
/// The walk mirrors the one `hyperpolyglot::get_language_breakdown` owned
/// before #523: `ignore`'s parallel walker, thread count from the host's
/// parallelism, linguist's vendor and documentation exclusions layered on top
/// (see [`filters`](super::filters)), directories skipped, and results
/// funnelled through an `mpsc` channel. `ignore`'s own defaults — hidden files
/// skipped, `.gitignore`, parent gitignores and the global gitignore honored —
/// are left untouched, exactly as `hyperpolyglot` left them.
///
/// When `include_file_sizes` is false the per-file size lookup is skipped
/// entirely and every pair reports zero bytes; see
/// [`Detector::detect_directory_without_metadata`].
fn walk_directory(path: &Path, include_file_sizes: bool) -> Vec<(Language, u64)> {
    let (sender, receiver) = std::sync::mpsc::channel::<(Language, u64)>();
    ignore::WalkBuilder::new(path)
        .threads(walk_threads())
        .overrides(filters::exclusion_overrides(path))
        .build_parallel()
        .run(|| {
            let thread_sender = sender.clone();
            Box::new(move |result| {
                if let Ok(entry) = result {
                    let entry_path = entry.path();
                    // `is_dir()` rather than the walk's own file type, matching
                    // hyperpolyglot: it follows symlinks, so a symlink pointing
                    // at a directory is skipped rather than classified.
                    if !entry_path.is_dir()
                        && let Ok(Some(language)) = detect_language(entry_path)
                    {
                        let bytes = if include_file_sizes {
                            entry.metadata().map_or(0_u64, |metadata| metadata.len())
                        } else {
                            0_u64
                        };
                        // The receiver outlives the walk, so this only fails if
                        // the collector is already gone -- nothing left to do.
                        if thread_sender.send((language, bytes)).is_err() {
                            return ignore::WalkState::Quit;
                        }
                    }
                }
                ignore::WalkState::Continue
            })
        });
    drop(sender);
    receiver.into_iter().collect()
}

/// Resolves a single file's language, opening it only when the path alone
/// cannot decide.
///
/// The order replicates `gengo_language`'s private `find_simple` minus its
/// shebang step, which is what keeps the answer identical to the
/// `hyperpolyglot` detector this replaced (#523):
///
/// 1. Strip [`STRIPPED_SUFFIX_EXTENSIONS`] from the file name, repeatedly.
/// 2. Filename match; else path-glob match; else extension match.
/// 3. Exactly one candidate — emit it, and never open the file.
/// 4. Zero or several candidates — read up to [`READ_LIMIT`] bytes and hand
///    path and contents to [`Language::pick`], which re-runs the same matchers
///    with the shebang check and the content heuristics in play.
///
/// Step 3 is load-bearing and cannot be folded into step 4 by calling
/// `Language::pick(path, b"", READ_LIMIT)`. With empty contents no heuristic
/// matches, so `pick` falls through to a static priority tie-break instead of
/// reporting ambiguity: `.h` would resolve to C (priority 75) over C++ and
/// Objective-C (both 50) without the file ever being read.
///
/// One behavioural edge follows from checking the extension before the shebang:
/// a `.py` file carrying `#!/usr/bin/env ruby` resolves to Python. That is the
/// answer `hyperpolyglot` gave, so it is parity — `find_simple` checks the
/// shebang first and would say Ruby, which makes running full-contents `pick`
/// unconditionally the behaviour *change* and this lazy path the conservative
/// one.
///
/// # Errors
///
/// Returns an error when the fallback read fails. Paths settled at step 3 never
/// touch the filesystem and so never error.
fn detect_language(path: &Path) -> std::io::Result<Option<Language>> {
    let stripped = strip_suffix_extensions(path);
    let probe = stripped.as_deref().unwrap_or(path);

    if let [only_candidate] = path_candidates(probe).as_slice() {
        return Ok(Some(*only_candidate));
    }

    let contents = read_capped(path)?;
    Ok(Language::pick(path, &contents, READ_LIMIT))
}

/// Languages a path alone can name, by filename, then path glob, then
/// extension — `gengo_language`'s own precedence, minus the shebang step that
/// would require reading the file.
fn path_candidates(path: &Path) -> Vec<Language> {
    let by_filename = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(Vec::new, Language::from_filename);
    if !by_filename.is_empty() {
        return by_filename;
    }

    let by_glob = Language::from_glob(path);
    if !by_glob.is_empty() {
        return by_glob;
    }

    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(Vec::new, Language::from_extension)
}

/// Removes every trailing [`STRIPPED_SUFFIX_EXTENSIONS`] entry from `path`'s
/// file name, returning `None` when there was nothing to strip.
fn strip_suffix_extensions(path: &Path) -> Option<std::path::PathBuf> {
    let name = path.file_name().and_then(std::ffi::OsStr::to_str)?;
    let mut stripped = name;
    while let Some(shorter) = STRIPPED_SUFFIX_EXTENSIONS
        .iter()
        .find_map(|suffix| stripped.strip_suffix(suffix))
    {
        stripped = shorter;
    }
    if stripped.len() == name.len() {
        return None;
    }
    Some(path.with_file_name(stripped))
}

/// Reads at most [`READ_LIMIT`] bytes from `path`.
///
/// # Errors
///
/// Returns the underlying `std::io::Error` when the file cannot be opened or
/// read.
fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;

    let limit = u64::try_from(READ_LIMIT).unwrap_or(u64::MAX);
    let mut contents = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut contents)?;
    Ok(contents)
}

/// Folds per-file detections into per-canonical-name metrics.
fn aggregate_detections(
    detections: Vec<(Language, u64)>,
) -> std::collections::HashMap<String, LanguageMetrics> {
    detections.into_iter().fold(
        std::collections::HashMap::new(),
        |mut aggregated, (language, bytes)| {
            let metrics = LanguageMetrics {
                file_count: 1_usize,
                total_bytes: bytes,
            };
            aggregated
                .entry(normalize_language_name(language.name()).to_owned())
                .and_modify(|existing| *existing = existing.combine(metrics))
                .or_insert(metrics);
            aggregated
        },
    )
}

fn languages_from_aggregates(
    aggregated: std::collections::HashMap<String, LanguageMetrics>,
) -> Vec<DetectedLanguage> {
    let totals = aggregate_totals(aggregated.values());
    let mut languages: Vec<DetectedLanguage> = aggregated
        .into_iter()
        .map(|(name, metrics)| DetectedLanguage {
            name,
            confidence: confidence_for(metrics, totals),
            file_count: metrics.file_count,
            total_bytes: metrics.total_bytes,
        })
        .collect();

    sort_by_confidence(&mut languages);
    languages
}

fn aggregate_totals<'a>(metrics: impl IntoIterator<Item = &'a LanguageMetrics>) -> LanguageMetrics {
    metrics
        .into_iter()
        .fold(LanguageMetrics::default(), |total, metric| {
            total.combine(*metric)
        })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "confidence is a display-only heuristic ranking; losing precision converting byte/file counts to f64 does not change the resulting order"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the final `as f32` intentionally narrows the f64 ratio for the DetectedLanguage confidence field; the value is always in [0.0, 1.0]"
)]
#[expect(
    clippy::as_conversions,
    reason = "numeric widen/narrow casts for a display-only confidence heuristic, not domain identifiers that need TryFrom validation"
)]
fn confidence_for(metrics: LanguageMetrics, totals: LanguageMetrics) -> f32 {
    if totals.file_count == 0_usize || totals.total_bytes == 0_u64 {
        return 0.0_f32;
    }

    let bytes_ratio = metrics.total_bytes as f64 / totals.total_bytes as f64;
    let files_ratio = metrics.file_count as f64 / totals.file_count as f64;
    bytes_ratio.mul_add(0.7_f64, files_ratio * 0.3_f64) as f32
}

fn sort_by_confidence(languages: &mut [DetectedLanguage]) {
    languages.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Upper bound on the number of non-ignored files [`Detector::detect_directory_bounded`]
/// will tolerate before falling back from `Content` to `Markers` mode (#391).
///
/// [`Detector::detect_directory`] walks the whole tree with no size cap or
/// deadline of its own, so the cheapest way to bound its worst case is to
/// decide *before* starting it. This is comfortably above this project's own
/// tracked file count (~600) while still keeping a worst-case scan to a bounded
/// number of files instead of an entire monorepo.
const CONTENT_SCAN_FILE_BUDGET: usize = 20_000;

/// Cheaply checks whether `path`'s non-ignored file tree exceeds `budget`
/// files, stopping the walk as soon as the budget is crossed so this check
/// never costs more than the budget itself allows.
///
/// Uses the same `ignore` walker [`walk_directory`] does, so a directory this
/// reports as "under budget" is a faithful proxy for the file count the real
/// scan would visit. It deliberately omits the exclusion overrides: skipping
/// them makes this an upper bound, and building 161 globs would cost more than
/// the check itself.
fn exceeds_file_budget(path: &Path, budget: usize) -> bool {
    ignore::WalkBuilder::new(path)
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        })
        .take(budget.saturating_add(1_usize))
        .count()
        > budget
}

/// Convert a detector's language name to our standardized name.
///
/// Unrecognized names pass through unchanged, so a detected `Markdown` or
/// `JSON` still counts toward the totals [`confidence_for`] divides by.
#[must_use]
pub fn normalize_language_name(detected_name: &str) -> &str {
    crate::language::metadata::get_canonical_name(detected_name).unwrap_or(detected_name)
}

/// Get display color for a language from theme configuration (background color)
///
/// Uses the new hierarchical fallback system
#[must_use]
pub fn get_language_color_from_theme(
    language: &str,
    theme: &crate::config::LanguageTheme,
) -> String {
    theme.get_bg_color(language).to_owned()
}

// Design reference (see docs/dev/architecture.md § Language Detection):
// - gengo-language's matcher tables drive both directory breakdown and
//   single-file detection; the directory walk itself is GPY's
// - Results feed into standardized language names and version detection
// - Targets: 5–20 ms detection time, sorted by confidence, graceful handling of polyglot repos

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn equal_confidence_languages_sort_by_name_regardless_of_input_order() {
        let make = |name: &str| DetectedLanguage {
            name: name.to_owned(),
            confidence: 0.5_f32,
            file_count: 3_usize,
            total_bytes: 300_u64,
        };

        for _ in 0_usize..10_usize {
            // Starting order: [Zebra, Alpha] -- must still sort to [Alpha, Zebra].
            let mut languages = vec![make("Zebra"), make("Alpha")];
            sort_by_confidence(&mut languages);
            assert_eq!(
                languages
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Alpha", "Zebra"],
                "equal-confidence languages must tie-break by name"
            );

            // Starting order: [Alpha, Zebra] -- already in the target order, must
            // stay there (proving the fix doesn't just get lucky on one input shape).
            let mut already_ordered = vec![make("Alpha"), make("Zebra")];
            sort_by_confidence(&mut already_ordered);
            assert_eq!(
                already_ordered
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Alpha", "Zebra"],
                "equal-confidence languages must tie-break by name"
            );
        }
    }

    #[test]
    fn marker_evidence_counts_marker_files_and_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        // Marker file match: `Cargo.toml` -> rust.
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        // Marker extension match: `.py` -> python.
        std::fs::write(tmp.path().join("script.py"), "").unwrap();
        // Neither a marker file nor a marker extension.
        std::fs::write(tmp.path().join("README.md"), "").unwrap();

        let with_extensions = marker_evidence(tmp.path(), true);
        assert_eq!(with_extensions.get("rust").copied(), Some(1));
        assert_eq!(
            with_extensions.get("python").copied(),
            Some(1),
            "extension-only match should count when include_extensions is true"
        );
        assert_eq!(with_extensions.get("node"), None);

        let without_extensions = marker_evidence(tmp.path(), false);
        assert_eq!(without_extensions.get("rust").copied(), Some(1));
        assert_eq!(
            without_extensions.get("python"),
            None,
            "extension-only match must be excluded when include_extensions is false"
        );
    }

    #[test]
    fn marker_evidence_unreadable_directory_is_empty() {
        let missing = Path::new("/does/not/exist/at/all");
        assert!(marker_evidence(missing, true).is_empty());
    }
}
