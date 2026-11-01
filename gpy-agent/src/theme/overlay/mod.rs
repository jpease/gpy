//! Theme overlays (ADR-0012): a theme file with a top-level `extends` string
//! names a base theme and holds only the keys it changes.
//!
//! Every theme load from a name or a path goes through this module
//! (`ThemeManager`, `theme::load::load_theme*`, `gpy theme validate`);
//! [`ThemeManager::builtin`](crate::theme::ThemeManager::builtin) stays full-only.
//!
//! - A file without `extends` is a full theme: its table, parsed once, goes
//!   straight to [`from_table`], and only a failure re-parses the text for spans.
//! - An overlay's chain is resolved file by file (at most [`MAX_EXTENDS_HOPS`]
//!   hops, cycles rejected), each file's `[segments.language]` is rewritten to
//!   the nested spelling, and from the bottom up each overlay's `unset` paths
//!   are removed from the merged base before the overlay is merged over it.
//!   [`from_table`] then checks the merged table.
//! - A failure of an overlay chain is attributed to the file and line that
//!   caused it (see `attribute`); a successful load pays nothing for that.

mod attribute;
mod header;
mod lint;
mod merge;
mod resolve;

pub(crate) use header::split_path;
pub(crate) use lint::merged_table;
pub use lint::unset_warnings;
pub(crate) use merge::nested_language_path;
pub use resolve::resolve_top;

use super::ThemeConfig;
use super::parse::{from_table, full_theme_result, parse_table};
use crate::config::discovery::Source;
use crate::config::validation::ValidationError;
use crate::theme::user_copy::{builtin_content, user_copy_path};
use crate::watcher::hot_reload::bytes_fingerprint;
use crate::{Error, Result};
use header::Header;
use resolve::{Candidates, Identity, OwnName};
use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

/// Most `extends` hops one chain may follow: an overlay of a user overlay of a
/// plugin overlay of a builtin needs 3, and a 5th hop is an error.
pub const MAX_EXTENDS_HOPS: usize = 4;

/// One theme file of a resolved [`ThemeChain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLink {
    /// The file's own theme name: its on-disk stem, or the builtin's name.
    pub name: String,
    /// Where the file came from (user, plugin, or the embedded builtin).
    pub source: Source,
    /// The file's path. An embedded builtin carries the path where no file
    /// was found: its user theme path (`<config>/themes/<name>.toml`) when
    /// resolved by name, the requested path when loaded by path.
    pub path: PathBuf,
    /// Whether the theme is read from `path`; `false` means the embedded builtin.
    pub on_disk: bool,
}

impl ChainLink {
    /// The embedded builtin `name`, at its user theme path. Loading it fails as
    /// "not found" when `name` is not a builtin.
    #[must_use]
    pub fn embedded(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            source: Source::Builtin,
            path: user_copy_path(name),
            on_disk: false,
        }
    }
}

impl fmt::Display for ChainLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.source)
    }
}

/// The files a theme was merged from: the requested file first, then each
/// base its `extends` named, down to the full theme at the bottom. A full
/// theme is a chain of one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeChain {
    top: ChainLink,
    bases: Vec<ChainLink>,
}

impl ThemeChain {
    /// The requested file.
    #[must_use]
    pub const fn top(&self) -> &ChainLink {
        &self.top
    }

    /// Every link, the requested file first and the full theme last.
    pub fn iter(&self) -> impl Iterator<Item = &ChainLink> {
        std::iter::once(&self.top).chain(&self.bases)
    }

    /// Whether the requested file is an overlay (the chain has a base).
    #[must_use]
    pub const fn is_overlay(&self) -> bool {
        !self.bases.is_empty()
    }
}

impl fmt::Display for ThemeChain {
    /// `mine (user) → starship (user) → starship (builtin)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, link) in self.iter().enumerate() {
            if index > 0 {
                f.write_str(" → ")?;
            }
            write!(f, "{link}")?;
        }
        Ok(())
    }
}

/// A loaded theme with the chain it was merged from.
#[derive(Debug)]
pub struct LoadedTheme {
    /// The merged, validated theme.
    pub theme: ThemeConfig,
    /// The files it was merged from.
    pub chain: ThemeChain,
    /// Fingerprint of the bytes of every on-disk link, in chain order; `None`
    /// when every link is an embedded builtin. For a single file it is that
    /// file's bytes fingerprint, as before overlays.
    pub fingerprint: Option<u64>,
    /// Each link's text, in chain order, for attributing later failures.
    texts: Vec<Cow<'static, str>>,
}

impl LoadedTheme {
    /// The error for a template validation failure of this theme (`theme use`,
    /// `theme validate`). For an overlay chain it names the file and line that
    /// set the failing key and the chain; a full theme's error is unchanged.
    /// The [`ValidationError`] stays the returned error's source.
    #[must_use]
    pub fn template_error(&self, error: ValidationError) -> Error {
        if self.chain.is_overlay() {
            attribute::template_error(&self.chain, &self.texts, error)
        } else {
            error.into()
        }
    }
}

/// Loads the theme a top-level resolution found (see [`resolve_top`]), merging
/// overlays. An overlay's own name is taken from its directory listing.
///
/// # Errors
/// Returns an error if a file of the chain can't be read, parsed or resolved,
/// or the merged theme is invalid.
pub(crate) fn load_link(top: ChainLink) -> Result<LoadedTheme> {
    load_with(top, OwnName::Listed, &mut Candidates::discover())
}

/// Loads the theme file at `path`, falling back to the builtin with its stem
/// when no file exists. The file is a user theme named after its stem, unless
/// it lies in a plugin's `themes/` directory, where the plugin rule applies.
///
/// # Errors
/// As [`load_link`].
pub(crate) fn load_path(path: &Path) -> Result<LoadedTheme> {
    load_with(
        resolve::link_for_path(path),
        OwnName::Stem,
        &mut Candidates::discover(),
    )
}

/// Loads `text` as if it were the content of the user theme file `link`,
/// without reading or writing that file, so `gpy theme rebase` can check an
/// overlay before writing it. Resolution, the own-name rule and the cycle
/// check treat it exactly as the file at `link.path`.
///
/// # Errors
/// As [`load_link`].
pub(crate) fn load_text(link: ChainLink, text: String) -> Result<LoadedTheme> {
    let label = link.path.display().to_string();
    let first = ChainFile::from_text(link, label, Cow::Owned(text))?;
    load_first(first, OwnName::Listed, &mut Candidates::discover())
}

/// The existing user theme file `name` as a chain link, with its own name and
/// path taken from the directory listing as when it loads by name.
///
/// # Errors
/// Returns an error if no directory entry, or more than one, matches `name`.
pub(crate) fn own_user_link(name: &str) -> Result<ChainLink> {
    resolve::own_link(
        ChainLink {
            name: name.to_owned(),
            source: Source::User,
            path: user_copy_path(name),
            on_disk: true,
        },
        OwnName::Listed,
    )
}

/// Where `extends = "<name>"` in the file `from` resolves (ADR-0012 section 3).
///
/// # Errors
/// Returns the diagnostic when `name` resolves to nothing allowed.
pub(crate) fn resolve_extends(
    name: &str,
    from: &ChainLink,
) -> std::result::Result<ChainLink, String> {
    resolve::resolve_base(name, from, &mut Candidates::discover())
}

/// One file of a chain, read and split into its overlay header and table.
struct ChainFile {
    link: ChainLink,
    label: String,
    text: Cow<'static, str>,
    table: toml::Table,
    header: Header,
}

impl ChainFile {
    /// Read and syntax-check `link`. `label` names the file in errors.
    ///
    /// # Errors
    /// Returns an error if the file can't be read, no builtin matches an
    /// absent file, or the TOML or overlay keys are malformed.
    fn read(mut link: ChainLink, label: String) -> Result<Self> {
        let text = if link.on_disk {
            Cow::Owned(std::fs::read_to_string(&link.path).map_err(|e| {
                Error::config(format!(
                    "Failed to read theme file {}: {e}",
                    link.path.display()
                ))
            })?)
        } else if let Some(content) = builtin_content(&link.name) {
            link.source = Source::Builtin;
            Cow::Borrowed(content)
        } else {
            return Err(Error::config(format!(
                "theme '{}' not found (no user theme, plugin theme, or builtin matches this name)",
                link.name
            )));
        };
        Self::from_text(link, label, text)
    }

    /// Syntax-check `text` as the content of `link`.
    ///
    /// # Errors
    /// Returns an error if the TOML or overlay keys are malformed.
    fn from_text(link: ChainLink, label: String, text: Cow<'static, str>) -> Result<Self> {
        let mut table = parse_table(&text, &label)?;
        let header = Header::take(&mut table, &label)?;
        Ok(Self {
            link,
            label,
            text,
            table,
            header,
        })
    }
}

/// Error for a failed overlay check of the file `label`.
fn overlay_error(label: &str, diagnostic: &str, remediation: &str) -> Error {
    Error::config(format!(
        "Invalid theme overlay {label}\n  Diagnostic: {diagnostic}\n  Remediation: {remediation}"
    ))
}

/// Load `top` and, if it is an overlay, every base, resolving through `candidates`.
///
/// # Errors
/// As [`load_link`].
fn load_with(top: ChainLink, own: OwnName, candidates: &mut Candidates) -> Result<LoadedTheme> {
    // The top file keeps the label it has always had, its path, even when embedded.
    let top_label = top.path.display().to_string();
    let first = ChainFile::read(top, top_label)?;
    load_first(first, own, candidates)
}

/// Load the already read `first` and, if it is an overlay, every base.
///
/// # Errors
/// As [`load_link`].
fn load_first(first: ChainFile, own: OwnName, candidates: &mut Candidates) -> Result<LoadedTheme> {
    if first.header.extends.is_none() {
        return load_full(first);
    }
    merge_chain(read_chain(first, own, candidates)?)
}

/// Every file of the overlay `first`'s chain, `first` at the front.
///
/// # Errors
/// As [`load_link`], short of merging.
fn read_chain(
    mut first: ChainFile,
    own: OwnName,
    candidates: &mut Candidates,
) -> Result<Vec<ChainFile>> {
    first.link = resolve::own_link(first.link, own)?;

    let mut identities = vec![Identity::of(&first.link)];
    let mut files = vec![first];
    while let Some(last) = files.last()
        && let Some(extends) = &last.header.extends
    {
        let links = || files.iter().map(|file| &file.link);
        if files.len() > MAX_EXTENDS_HOPS {
            return Err(overlay_error(
                &last.label,
                &format!(
                    "the chain {} → {extends} needs more than {MAX_EXTENDS_HOPS} `extends` hops",
                    join_links(links())
                ),
                "Extend a theme closer to the full theme the chain is based on.",
            ));
        }
        let link = resolve::resolve_base(extends, &last.link, candidates).map_err(|reason| {
            overlay_error(
                &last.label,
                &reason,
                "Set `extends` to the name of an existing theme (see `gpy theme list`).",
            )
        })?;
        let identity = Identity::of(&link);
        if identities.contains(&identity) {
            return Err(overlay_error(
                &last.label,
                &format!("`extends` cycle: {} → {link}", join_links(links())),
                "Break the cycle: one theme of the chain must extend a theme outside it.",
            ));
        }
        let base_label = if link.on_disk {
            link.path.display().to_string()
        } else {
            format!("builtin theme '{}'", link.name)
        };
        let file = ChainFile::read(link, base_label)?;
        identities.push(identity);
        files.push(file);
    }
    Ok(files)
}

/// A full theme: `from_table` on the table already parsed; only a failure
/// re-parses the text for spans.
///
/// # Errors
/// Returns the full theme's diagnostics, unchanged from before overlays.
fn load_full(file: ChainFile) -> Result<LoadedTheme> {
    let ChainFile {
        link,
        label,
        text,
        table,
        ..
    } = file;
    let theme =
        from_table(table, &label).or_else(|error| full_theme_result(&text, &label, error))?;
    let fingerprint = link.on_disk.then(|| bytes_fingerprint(text.as_bytes()));
    Ok(LoadedTheme {
        theme,
        chain: ThemeChain {
            top: link,
            bases: Vec::new(),
        },
        fingerprint,
        texts: vec![text],
    })
}

/// Merge `files` (top first; the last is a full theme) and check the result.
///
/// # Errors
/// Returns the attributed error if the merged table fails `from_table`.
fn merge_chain(files: Vec<ChainFile>) -> Result<LoadedTheme> {
    let top_label = files
        .first()
        .map_or_else(String::new, |file| file.label.clone());
    let mut links = Vec::with_capacity(files.len());
    let mut texts = Vec::with_capacity(files.len());
    let mut merged = toml::Table::new();
    for file in files.into_iter().rev() {
        let ChainFile {
            link,
            text,
            mut table,
            header,
            ..
        } = file;
        merge::canonicalise(&mut table);
        for entry in &header.unset {
            merge::remove_path(&mut merged, &entry.path);
        }
        merge::deep_merge(&mut merged, table);
        links.push(link);
        texts.push(text);
    }
    links.reverse();
    texts.reverse();
    let mut ordered = links.into_iter();
    let Some(top) = ordered.next() else {
        return Err(Error::config("theme chain is empty"));
    };
    let chain = ThemeChain {
        top,
        bases: ordered.collect(),
    };
    let fingerprint = chain_fingerprint(&chain, &texts);
    let theme = from_table(merged, &top_label)
        .map_err(|error| attribute::table_error(&chain, &texts, error))?;
    Ok(LoadedTheme {
        theme,
        chain,
        fingerprint,
        texts,
    })
}

/// `a (user) → b (builtin)`.
fn join_links<'a>(links: impl Iterator<Item = &'a ChainLink>) -> String {
    links
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" → ")
}

/// Fingerprint over every on-disk link's bytes; a single file keeps its own.
fn chain_fingerprint(chain: &ThemeChain, texts: &[Cow<'static, str>]) -> Option<u64> {
    use std::hash::{Hash as _, Hasher as _};

    let prints: Vec<u64> = chain
        .iter()
        .zip(texts)
        .filter(|(link, _)| link.on_disk)
        .map(|(_, text)| bytes_fingerprint(text.as_bytes()))
        .collect();
    match prints.as_slice() {
        [] => None,
        [one] => Some(*one),
        many => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            many.hash(&mut hasher);
            Some(hasher.finish())
        }
    }
}

#[cfg(test)]
mod attribution_tests;
#[cfg(test)]
mod tests;
