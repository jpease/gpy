//! Cache keys: a project root escaped into one filename-safe string, chunked
//! into directories when too long for a single filename.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Max escaped key length on Windows before truncation + hash-tail; leaves
/// headroom under the 255-char filename limit for suffix, token and ext.
pub(super) const WINDOWS_CACHE_KEY_MAX_LEN: usize = 200;

/// Longest Unix cache key stored as one flat filename, in bytes; longer keys
/// are chunked so the name plus `.{variant}.{token}.{ext}` stays under
/// [`NAME_MAX_BYTES`].
pub(super) const FLAT_CACHE_KEY_MAX_BYTES: usize = 200;

/// Bytes per key chunk, leaving room under [`NAME_MAX_BYTES`] for the tail.
pub(super) const CACHE_KEY_CHUNK_BYTES: usize = 50;

/// Longest filename component Linux and macOS accept, in bytes.
pub(super) const NAME_MAX_BYTES: usize = 255;

/// Convert a path to an injective cache key over its bytes. Long keys are
/// chunked on disk by [`cache_key_path`].
pub(super) fn path_to_cache_key(path: &Path) -> String {
    #[cfg(windows)]
    let os = crate::paths::Os::Windows;
    #[cfg(not(windows))]
    let os = crate::paths::Os::Unix;
    path_to_cache_key_for(path.as_os_str().as_encoded_bytes(), os)
}

/// [`path_to_cache_key`] for an explicit [`Os`](crate::paths::Os), over the
/// path's bytes ([`std::ffi::OsStr::as_encoded_bytes`]).
///
/// Both arms escape `_` as `__`, `/` as `_s`, `\` as `_b`, `:` as `_c`, space
/// as `_w`, and each byte of a sequence that is not UTF-8 as `_x` plus two
/// lowercase hex digits.
///
/// The Unix arm also escapes an ASCII capital as `_u` plus its lowercase
/// letter and every byte of a non-ASCII character as `_x` hex. Every token
/// starts with `_`, no two share a prefix, and no other byte is `_`, so the
/// key decodes back to the path. Its keys are lowercase ASCII, so two roots
/// never share a file on a cache volume that folds case or normalizes
/// Unicode, as macOS does by default.
///
/// The Windows arm escapes the reserved `? * < > " |` as `_q _a _l _g _d _p`
/// instead, keeps other characters, and is case-folded and capped by
/// [`windows_harden_cache_key`].
pub(super) fn path_to_cache_key_for(path: impl AsRef<[u8]>, os: crate::paths::Os) -> String {
    let bytes = path.as_ref();
    let mut key = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            push_escaped_char(&mut key, c, os);
        }
        for &byte in chunk.invalid() {
            push_byte_escape(&mut key, byte);
        }
    }
    match os {
        crate::paths::Os::Unix => key,
        crate::paths::Os::Windows => windows_harden_cache_key(&key),
    }
}

/// Append `c` to `key`, escaped for `os` as [`path_to_cache_key_for`] says.
fn push_escaped_char(key: &mut String, c: char, os: crate::paths::Os) {
    use crate::paths::Os;
    match (c, os) {
        ('_', _) => key.push_str("__"),
        ('/', _) => key.push_str("_s"),
        ('\\', _) => key.push_str("_b"),
        (':', _) => key.push_str("_c"),
        (' ', _) => key.push_str("_w"),
        ('?', Os::Windows) => key.push_str("_q"),
        ('*', Os::Windows) => key.push_str("_a"),
        ('<', Os::Windows) => key.push_str("_l"),
        ('>', Os::Windows) => key.push_str("_g"),
        ('"', Os::Windows) => key.push_str("_d"),
        ('|', Os::Windows) => key.push_str("_p"),
        (capital, Os::Unix) if capital.is_ascii_uppercase() => {
            key.push_str("_u");
            key.push(capital.to_ascii_lowercase());
        }
        (wide, Os::Unix) if !wide.is_ascii() => {
            for &byte in wide.encode_utf8(&mut [0_u8; 4]).as_bytes() {
                push_byte_escape(key, byte);
            }
        }
        (plain, _) => key.push(plain),
    }
}

/// Append `byte` to `key` as `_x` and two lowercase hex digits.
fn push_byte_escape(key: &mut String, byte: u8) {
    // Writing to a `String` cannot fail.
    let _ = write!(key, "_x{byte:02x}");
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

/// Relative on-disk stem: the key up to [`FLAT_CACHE_KEY_MAX_BYTES`], else
/// chunks of at most [`CACHE_KEY_CHUNK_BYTES`] joined by `/`, never splitting
/// a character. A Unix key is ASCII, so each chunk but the last is exactly
/// that long. Pinned by `test_path_to_cache_key_shared_vectors`
/// (`tests/fixtures/cache_key_vectors.tsv`). Windows keys are already capped
/// and returned unchanged.
pub(super) fn cache_key_path_for(key: &str, os: crate::paths::Os) -> Cow<'_, str> {
    if matches!(os, crate::paths::Os::Windows) || key.len() <= FLAT_CACHE_KEY_MAX_BYTES {
        return Cow::Borrowed(key);
    }
    let separators = key.len().div_ceil(CACHE_KEY_CHUNK_BYTES);
    let mut stem = String::with_capacity(key.len().saturating_add(separators));
    let mut chunk_len = 0_usize;
    for c in key.chars() {
        let width = c.len_utf8();
        if chunk_len > 0 && chunk_len.saturating_add(width) > CACHE_KEY_CHUNK_BYTES {
            stem.push('/');
            chunk_len = 0;
        }
        stem.push(c);
        chunk_len = chunk_len.saturating_add(width);
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

/// Project root for a path (git root or the path itself). The shells' copy is
/// pinned by `git_root_shared_vectors`.
pub(super) fn find_project_root(path: &Path) -> PathBuf {
    crate::git::find_worktree_root(path).unwrap_or_else(|| path.to_path_buf())
}
