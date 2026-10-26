//! Cache file names, `{key}.{variant}.{token}.{ext}`: the contract with the
//! shells (`tests/fixtures/cache_key_vectors.tsv`). Variant, token and
//! extension never contain `.`, so a name parses from the right; the key may.

use crate::formatter::{IsFirst, IsLast, PromptDialect, SegmentPosition};

/// All four segment positions, in one fixed order.
pub(super) const SEGMENT_POSITIONS: [SegmentPosition; 4] = [
    SegmentPosition::new(IsLast::No, IsFirst::No),
    SegmentPosition::new(IsLast::No, IsFirst::Yes),
    SegmentPosition::new(IsLast::Yes, IsFirst::No),
    SegmentPosition::new(IsLast::Yes, IsFirst::Yes),
];

/// Cache-filename token of a context-free render (no `prev_bg`).
pub(super) const NO_PREV_BG_TOKEN: &str = "none";

/// Filename token for the `prev_bg` an entry was rendered with: `none` when
/// absent, else non-alphanumeric bytes → `_`. Never contains `.`.
/// The shells' `__gpy_prev_bg_token` MUST match.
pub(super) fn prev_bg_token(prev_bg: Option<&str>) -> String {
    match prev_bg {
        Some(s) if !s.is_empty() => s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
        _ => NO_PREV_BG_TOKEN.to_owned(),
    }
}

/// Segment a cache file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CacheKind {
    /// The git status segment.
    Git,
    /// The language segment.
    Lang,
}

impl CacheKind {
    /// Every kind.
    const ALL: [Self; 2] = [Self::Git, Self::Lang];

    /// Leading part of the variant, e.g. `git`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Lang => "lang",
        }
    }
}

/// Rest of the variant after the kind, e.g. `_first_last`; with the kind it
/// MUST match the shells' `__gpy_cache_variant_suffix`.
const fn position_suffix(position: SegmentPosition) -> &'static str {
    match (position.is_first, position.is_last) {
        (IsFirst::No, IsLast::No) => "",
        (IsFirst::No, IsLast::Yes) => "_last",
        (IsFirst::Yes, IsLast::No) => "_first",
        (IsFirst::Yes, IsLast::Yes) => "_first_last",
    }
}

/// One cache file's name, before the key is chunked into directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CacheFileName<'a> {
    /// Cache key; a name parsed inside a chunk directory has the final chunk.
    pub(super) key: &'a str,
    /// Segment rendered.
    pub(super) kind: CacheKind,
    /// Position rendered for.
    pub(super) position: SegmentPosition,
    /// [`prev_bg_token`] of the render context.
    pub(super) token: &'a str,
    /// Shell dialect, named by its extension.
    pub(super) dialect: PromptDialect,
}

impl CacheFileName<'_> {
    /// The name after `{key}.`: `{variant}.{token}.{ext}`.
    pub(super) fn tail(&self) -> String {
        format!(
            "{}{}.{}.{}",
            self.kind.as_str(),
            position_suffix(self.position),
            self.token,
            self.dialect.cache_ext()
        )
    }

    /// Whether a restarted agent, which tracks `prev_bg` contexts in memory
    /// only, would never refresh this file: every language file, and any
    /// rendered for a specific `prev_bg`.
    pub(super) fn is_unrefreshable(&self) -> bool {
        self.kind == CacheKind::Lang || self.token != NO_PREV_BG_TOKEN
    }
}

/// Parse `{key}.{variant}.{token}.{ext}`; `None` for any other name, such as
/// a temp file.
pub(super) fn parse_cache_file_name(name: &str) -> Option<CacheFileName<'_>> {
    let (without_ext, ext) = name.rsplit_once('.')?;
    let dialect = PromptDialect::ALL
        .into_iter()
        .find(|dialect| dialect.cache_ext() == ext)?;
    let (key_variant, token) = without_ext.rsplit_once('.')?;
    let (key, variant) = key_variant.rsplit_once('.')?;
    let (kind, suffix) = CacheKind::ALL
        .into_iter()
        .find_map(|kind| variant.strip_prefix(kind.as_str()).map(|s| (kind, s)))?;
    let position = SEGMENT_POSITIONS
        .into_iter()
        .find(|&position| position_suffix(position) == suffix)?;
    Some(CacheFileName {
        key,
        kind,
        position,
        token,
        dialect,
    })
}
