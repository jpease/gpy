//! Linguist-grade language detection, path-first: a file is read only when
//! its path is ambiguous or unmatched.

use crate::Result;
use crate::language::filters;
use gengo_language::Language;
use std::path::Path;

/// Fallback read cap; matches `hyperpolyglot`'s `MAX_CONTENT_SIZE_BYTES`,
/// which the heuristic test corpus is calibrated against.
const READ_LIMIT: usize = 51_200;

/// Suffixes stripped before matching. `Language::pick` strips these too, so
/// the path-only pre-check must agree with it (e.g. `foo.rs.bak`).
const STRIPPED_SUFFIX_EXTENSIONS: [&str; 2] = [".bak", ".example"];

/// Language detector over `gengo-language`'s matcher tables.
#[derive(Default)]
pub struct Detector;

impl Detector {
    /// Detect languages in a directory, aggregated by normalized name.
    #[must_use]
    pub fn detect_directory(path: &Path) -> Vec<DetectedLanguage> {
        Self::detect_directory_impl(path, true)
    }

    /// [`Self::detect_directory`] without the per-file size lookup, for benchmarks.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn detect_directory_without_metadata(path: &Path) -> Vec<DetectedLanguage> {
        Self::detect_directory_impl(path, false)
    }

    fn detect_directory_impl(path: &Path, include_file_sizes: bool) -> Vec<DetectedLanguage> {
        let detections = walk_directory(path, include_file_sizes);
        languages_from_aggregates(aggregate_detections(detections))
    }

    /// Detect languages using the configured [`DetectionMode`].
    #[must_use]
    pub fn detect_directory_for_mode(
        path: &Path,
        mode: crate::config::types::DetectionMode,
    ) -> Vec<DetectedLanguage> {
        match mode {
            crate::config::types::DetectionMode::Content => Self::detect_directory(path),
            crate::config::types::DetectionMode::Markers => Self::detect_directory_markers(path),
            crate::config::types::DetectionMode::Hybrid => Self::detect_directory_hybrid(path),
        }
    }

    /// Content-scan detection gated to languages whose project marker *file*
    /// is present. Extensions are not used: a stray `.py` script already has
    /// one, so only a real marker distinguishes "a project of X".
    #[must_use]
    pub(crate) fn detect_directory_hybrid(path: &Path) -> Vec<DetectedLanguage> {
        let content = Self::detect_directory(path);
        if content.is_empty() {
            return content;
        }
        let marker_file_languages = Self::directory_marker_file_languages(path);
        content
            .into_iter()
            .filter(|lang| marker_file_languages.contains(lang.name.as_str()))
            .collect()
    }

    /// Languages whose marker file is among `path`'s immediate entries.
    fn directory_marker_file_languages(path: &Path) -> std::collections::HashSet<&'static str> {
        marker_evidence(path, false).keys().copied().collect()
    }

    /// Detect languages for an externally supplied path, falling back from
    /// `Content`/`Hybrid` to `Markers` outside git or above
    /// [`CONTENT_SCAN_FILE_BUDGET`] files, since an unbounded scan of `$HOME`
    /// can take minutes. External paths MUST use this; known repo roots may
    /// call [`Self::detect_directory_for_mode`].
    #[must_use]
    pub fn detect_directory_bounded(
        path: &Path,
        mode: crate::config::types::DetectionMode,
    ) -> Vec<DetectedLanguage> {
        Self::detect_directory_bounded_within(path, mode, CONTENT_SCAN_FILE_BUDGET)
    }

    /// [`Self::detect_directory_bounded`] with an explicit file budget.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn detect_directory_bounded_with_budget(
        path: &Path,
        mode: crate::config::types::DetectionMode,
        budget: usize,
    ) -> Vec<DetectedLanguage> {
        Self::detect_directory_bounded_within(path, mode, budget)
    }

    /// [`Self::detect_directory_bounded`] with a content scan capped at `budget` files.
    fn detect_directory_bounded_within(
        path_ref: &Path,
        mode: crate::config::types::DetectionMode,
        budget: usize,
    ) -> Vec<DetectedLanguage> {
        let needs_fallback = matches!(
            mode,
            crate::config::types::DetectionMode::Content
                | crate::config::types::DetectionMode::Hybrid
        ) && (crate::git::find_git_dir_only(path_ref).is_none()
            || exceeds_file_budget(path_ref, budget));
        let bounded_mode = if needs_fallback {
            crate::config::types::DetectionMode::Markers
        } else {
            mode
        };
        Self::detect_directory_for_mode(path_ref, bounded_mode)
    }

    /// Detect languages by marker files/extensions among immediate entries,
    /// ordered by evidence count then name. Unreadable directories yield none.
    #[must_use]
    pub(crate) fn detect_directory_markers(path: &Path) -> Vec<DetectedLanguage> {
        let evidence = marker_evidence(path, true);

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
    /// If an ambiguous path's fallback content read fails.
    pub fn detect_file(path: &Path) -> Result<Option<DetectedLanguage>> {
        let detected = detect_language(path).map_err(|error| {
            crate::Error::language(format!(
                "language detection failed for {}: {error}",
                path.display()
            ))
        })?;
        let Some(language) = detected else {
            return Ok(None);
        };

        let file_size = std::fs::metadata(path).map_or(0, |metadata| metadata.len());
        Ok(Some(DetectedLanguage {
            name: normalize_language_name(language.name()).to_owned(),
            confidence: 1.0, // Single-file detection has no prevalence to weigh.
            file_count: 1,
            total_bytes: file_size,
        }))
    }
}

/// Count marker-file (and optionally extension) hits per language among
/// `path`'s immediate entries; empty if unreadable.
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
fn walk_threads() -> usize {
    std::thread::available_parallelism().map_or(1_usize, std::num::NonZeroUsize::get)
}

/// Walks `path` in parallel, yielding `(language, bytes)` per detected file.
/// Mirrors `hyperpolyglot`'s walk: `ignore` defaults plus linguist exclusions.
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
                    if is_regular_file(&entry)
                        && let Ok(Some(language)) = detect_language(entry_path)
                    {
                        let bytes = if include_file_sizes {
                            entry.metadata().map_or(0_u64, |metadata| metadata.len())
                        } else {
                            0_u64
                        };
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

/// Resolves a file's language: strip [`STRIPPED_SUFFIX_EXTENSIONS`], then
/// filename/glob/extension match; a unique candidate wins without reading,
/// otherwise read up to [`READ_LIMIT`] bytes and call [`Language::pick`].
///
/// The unique-candidate step can't be `pick` with empty contents: that
/// tie-breaks by static priority (`.h` → C) instead of reading. Checking the
/// extension before the shebang matches `hyperpolyglot`.
///
/// # Errors
/// If the fallback read fails.
fn detect_language(path: &Path) -> std::io::Result<Option<Language>> {
    let stripped = strip_suffix_extensions(path);
    let probe = stripped.as_deref().unwrap_or(path);

    if let [only_candidate] = path_candidates(probe).as_slice() {
        return Ok(Some(*only_candidate));
    }

    let contents = crate::fs_util::read_regular_file_capped(path, READ_LIMIT)?;
    Ok(Language::pick(path, &contents, READ_LIMIT))
}

/// Languages a path alone can name (filename, glob, extension; no shebang).
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

/// Strips trailing [`STRIPPED_SUFFIX_EXTENSIONS`]; `None` if none present.
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

/// Whether a walk entry is a regular file, following symlinks. Directory
/// symlinks are skipped (hyperpolyglot parity), and FIFOs, sockets and devices
/// never reach a content read. Only symlinks cost a `stat`.
#[allow(
    clippy::filetype_is_file,
    reason = "only regular files (direct or via a symlink) are content-read; FIFOs, sockets and devices are excluded on purpose"
)]
fn is_regular_file(entry: &ignore::DirEntry) -> bool {
    entry.file_type().is_some_and(|file_type| {
        file_type.is_file()
            || (file_type.is_symlink() && entry.path().metadata().is_ok_and(|meta| meta.is_file()))
    })
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

#[expect(clippy::cast_precision_loss, reason = "display-only ranking")]
#[expect(clippy::cast_possible_truncation, reason = "ratio is in [0.0, 1.0]")]
#[allow(
    clippy::as_conversions,
    reason = "u64/usize to f64 and f64 to f32 have no From impl; the precision loss is covered by the expects above"
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

/// Max non-ignored files before [`Detector::detect_directory_bounded`] falls
/// back from `Content` to `Markers`.
const CONTENT_SCAN_FILE_BUDGET: usize = 20_000;

/// Whether `path` has more than `budget` files, stopping early. Omits the
/// exclusion overrides, so it is an upper bound.
#[allow(
    clippy::filetype_is_file,
    reason = "the budget counts regular files only, matching the files a content scan reads"
)]
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

/// Convert a detector's language name to our standardized name; unknown names
/// pass through.
#[must_use]
pub fn normalize_language_name(detected_name: &str) -> &str {
    crate::language::metadata::get_canonical_name(detected_name).unwrap_or(detected_name)
}

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
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(tmp.path().join("script.py"), "").unwrap();
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
