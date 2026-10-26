//! Cache keys: a project root escaped into one filename-safe string, chunked
//! into directories when too long for a single filename.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Max escaped key length on Windows before truncation + hash-tail; leaves
/// headroom under the 255-char filename limit for suffix, token and ext.
pub(super) const WINDOWS_CACHE_KEY_MAX_LEN: usize = 200;

/// Longest Unix cache key stored as one flat filename; longer keys are chunked
/// so the name plus `.{variant}.{token}.{ext}` stays under `NAME_MAX`.
pub(super) const FLAT_CACHE_KEY_MAX_CHARS: usize = 200;

/// Characters per key chunk: ≤200 bytes even at 4 bytes/char, leaving room for the tail.
pub(super) const CACHE_KEY_CHUNK_CHARS: usize = 50;

/// Longest filename component Linux and macOS accept, in bytes.
pub(super) const NAME_MAX_BYTES: usize = 255;

/// Convert a path to an injective cache key: `_` is escaped first (to `__`), then `/` `_s`, `\` `_b`, `:` `_c`, space `_w`.
/// The Unix output MUST match the shells' `__gpy_path_to_cache_key` byte-for-byte.
/// Long keys are chunked on disk by [`cache_key_path`].
pub(super) fn path_to_cache_key(path: &Path) -> String {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    path_to_cache_key_for(&path.to_string_lossy(), os)
}

/// [`path_to_cache_key`] for an explicit [`Os`](crate::paths::Os); only the Unix arm must match the shells.
pub(super) fn path_to_cache_key_for(path_str: &str, os: crate::paths::Os) -> String {
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

/// [`cache_key_path_for`] for this platform.
pub(super) fn cache_key_path(key: &str) -> Cow<'_, str> {
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
pub(super) fn cache_key_path_for(key: &str, os: crate::paths::Os) -> Cow<'_, str> {
    if matches!(os, crate::paths::Os::Windows) || key.chars().count() <= FLAT_CACHE_KEY_MAX_CHARS {
        return Cow::Borrowed(key);
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
    Cow::Owned(stem)
}

/// Directory under `cache_dir` holding the files of the key whose
/// [`cache_key_path`] is `stem`, and the key's final chunk, which starts
/// their names.
pub(super) fn split_key_path<'a>(cache_dir: &Path, stem: &'a str) -> (PathBuf, &'a str) {
    match stem.rsplit_once('/') {
        Some((chunk_dirs, last)) => (cache_dir.join(chunk_dirs), last),
        None => (cache_dir.to_path_buf(), stem),
    }
}

/// Directory and filename of `{cache_key_path(key)}.{tail}` under `cache_dir`.
pub(super) fn cache_file_parts(cache_dir: &Path, key: &str, tail: &str) -> (PathBuf, String) {
    let stem = cache_key_path(key);
    let (dir, last) = split_key_path(cache_dir, &stem);
    (dir, format!("{last}.{tail}"))
}

/// Project root for a path (git root or the path itself), matching the shells.
pub(super) fn find_project_root(path: &Path) -> PathBuf {
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
