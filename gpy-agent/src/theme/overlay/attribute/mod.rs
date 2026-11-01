//! Error attribution for an overlay chain (ADR-0012).
//!
//! Runs only after a check on the merged table failed, so a successful load
//! pays nothing for it. Three steps:
//!
//! 1. Find the failing key on the merged table (`find`).
//! 2. Find the file: replay the merge, recording which file last wrote or
//!    unset each path (`replay`), and map the failing paths to those records.
//! 3. Find the line: re-parse each named file with `toml_edit` and take the
//!    span of the key, or of the `unset` element.
//!
//! The error names the user and plugin files among the mapped records, and an
//! embedded builtin only when none of them is; only then is the failure called
//! a GPY bug. With no mapped record, it names the chain alone.

mod find;
mod replay;

use super::header::Header;
use super::merge::{self, flat_language_path};
use super::{ChainLink, ThemeChain};
use crate::Error;
use crate::config::discovery::Source;
use crate::config::validation::ValidationError;
use crate::theme::locate::{key_offset, line_column};
use crate::theme::parse::FromTableError;
use replay::{Provenance, Record, ReplayFile};
use std::borrow::Cow;
use toml_edit::{Document, TableLike};

/// What failed: the paths of the merged table it maps to, and why.
struct Finding {
    paths: Vec<Vec<String>>,
    reason: String,
}

/// The error for a chain whose merged table failed `from_table`.
pub(super) fn table_error(
    chain: &ThemeChain,
    texts: &[Cow<'static, str>],
    error: FromTableError,
) -> Error {
    let rebuilt = rebuild(texts);
    let merged = rebuilt.as_ref().map(|(table, _)| table);
    let (finding, source): (Finding, Option<Box<dyn std::error::Error + Send + Sync>>) = match error
    {
        FromTableError::Missing(table) => (
            Finding {
                paths: vec![vec![table.key().to_owned()]],
                reason: format!("missing required [{}] table", table.key()),
            },
            None,
        ),
        FromTableError::Deserialize(failure) => {
            let found = merged.and_then(find::deserialize_failure);
            let at_key = found.map_or_else(
                || Finding {
                    paths: Vec::new(),
                    reason: find::reason(&failure),
                },
                |(path, reason)| Finding {
                    paths: vec![path],
                    reason,
                },
            );
            (at_key, None)
        }
        FromTableError::Validation(invalid) => (
            Finding {
                paths: merged
                    .map(|table| find::validation_paths(table, &invalid))
                    .unwrap_or_default(),
                reason: invalid.to_string(),
            },
            Some(Box::new(invalid)),
        ),
    };
    let provenance = rebuilt.as_ref().map(|(_, provenance)| provenance);
    Error::Config {
        message: report(chain, texts, provenance, &finding),
        source,
    }
}

/// The error for a template validation failure of a merged chain.
pub(super) fn template_error(
    chain: &ThemeChain,
    texts: &[Cow<'static, str>],
    error: ValidationError,
) -> Error {
    let rebuilt = rebuild(texts);
    let finding = Finding {
        paths: rebuilt
            .as_ref()
            .map(|(merged, _)| find::validation_paths(merged, &error))
            .unwrap_or_default(),
        reason: error.to_string(),
    };
    let provenance = rebuilt.as_ref().map(|(_, provenance)| provenance);
    Error::Config {
        message: report(chain, texts, provenance, &finding),
        source: Some(Box::new(error)),
    }
}

/// Re-parse every file of the chain (`texts`, requested file first) and replay
/// the merge. `None` if a file no longer parses, which a loaded chain cannot.
pub(super) fn rebuild(texts: &[Cow<'static, str>]) -> Option<(toml::Table, Provenance)> {
    let mut files = Vec::with_capacity(texts.len());
    for (file, text) in texts.iter().enumerate().rev() {
        let mut table: toml::Table = toml::from_str(text).ok()?;
        let header = Header::take(&mut table, "").ok()?;
        merge::canonicalise(&mut table);
        files.push(ReplayFile {
            file,
            table,
            unset: header.unset,
        });
    }
    Some(Provenance::replay(files))
}

/// One file of the chain as attribution reads it.
struct Linked<'a> {
    link: &'a ChainLink,
    text: &'a str,
    document: Option<Document<&'a str>>,
}

impl Linked<'_> {
    const fn is_builtin(&self) -> bool {
        !self.link.on_disk && matches!(self.link.source, Source::Builtin)
    }
}

/// The attributed message.
fn report(
    chain: &ThemeChain,
    texts: &[Cow<'static, str>],
    provenance: Option<&Provenance>,
    finding: &Finding,
) -> String {
    let files: Vec<Linked<'_>> = chain
        .iter()
        .zip(texts)
        .map(|(link, text)| Linked {
            link,
            text,
            document: Document::parse(&**text).ok(),
        })
        .collect();
    let builtin = |file: usize| files.get(file).is_none_or(Linked::is_builtin);
    let mapped: Vec<Vec<(Vec<String>, Record)>> = finding
        .paths
        .iter()
        .map(|path| {
            provenance
                .map(|records| records.lookup(path))
                .unwrap_or_default()
        })
        .collect();
    let any_mapped = mapped.iter().any(|records| !records.is_empty());
    let user_mapped = mapped
        .iter()
        .flatten()
        .any(|(_, record)| !builtin(record.file));
    let gpy_bug = any_mapped && !user_mapped;

    let mut subjects = Vec::with_capacity(finding.paths.len());
    let mut locations: Vec<String> = Vec::new();
    let mut any_unset = false;
    for (path, records) in finding.paths.iter().zip(&mapped) {
        let mut named: Vec<usize> = Vec::new();
        let mut spelled = None;
        for (record_path, record) in records {
            if (user_mapped && builtin(record.file)) || named.contains(&record.file) {
                continue;
            }
            let Some(file) = files.get(record.file) else {
                continue;
            };
            named.push(record.file);
            any_unset |= record.unset.is_some();
            if spelled.is_none() {
                spelled = Some(spelling(file.document.as_ref(), path));
            }
            let location = locate(file, record_path, *record);
            if !locations.contains(&location) {
                locations.push(location);
            }
        }
        subjects.push(spelled.unwrap_or_else(|| path.clone()).join("."));
    }

    let remediation = if gpy_bug {
        "This is a GPY bug: only a builtin theme sets the failing keys. Please report it; meanwhile switch themes with `gpy theme use <name>`."
    } else if any_unset {
        "Remove the `unset` entry, or set the keys the table still needs after it."
    } else if locations.is_empty() {
        "Check the files of the chain; the failure needs keys from more than one of them."
    } else {
        "Fix or remove the key at the reported location. Ensure colors are valid (hex, named, or 'transparent') and icons contain no control characters."
    };
    let subject = if subjects.is_empty() {
        String::new()
    } else {
        format!("{}: ", subjects.join(", "))
    };
    let at = if locations.is_empty() {
        String::new()
    } else {
        format!(", {}", locations.join(", "))
    };
    format!(
        "Invalid theme '{name}'\n  Diagnostic: {subject}{reason}{at} (chain: {chain})\n  Remediation: {remediation}",
        name = chain.top().name,
        reason = finding.reason,
    )
}

/// `path` as the file `parsed` spells it: a nested language path the file
/// writes flat is shown flat.
fn spelling(parsed: Option<&Document<&str>>, path: &[String]) -> Vec<String> {
    if let Some(document) = parsed
        && path_offset(document, path).is_none()
        && let Some(flat) = flat_language_path(path)
        && path_offset(document, &flat).is_some()
    {
        return flat;
    }
    path.to_vec()
}

/// `set at <path>:<line>:<column>`, or `unset at …` for an `unset` entry.
fn locate(file: &Linked<'_>, record_path: &[String], record: Record) -> String {
    let offset = file.document.as_ref().and_then(|document| {
        record.unset.map_or_else(
            || {
                path_offset(document, record_path).or_else(|| {
                    flat_language_path(record_path).and_then(|flat| path_offset(document, &flat))
                })
            },
            |index| unset_offset(document, index),
        )
    });
    let verb = if record.unset.is_some() {
        "unset at"
    } else {
        "set at"
    };
    let place = offset.map(|start| line_column(file.text, start));
    match (file.link.on_disk, place) {
        (true, Some((line, column))) => {
            format!("{verb} {}:{line}:{column}", file.link.path.display())
        }
        (true, None) => format!("{verb} {}", file.link.path.display()),
        (false, Some((line, column))) => format!(
            "{verb} builtin theme '{}' (line {line}, column {column})",
            file.link.name
        ),
        (false, None) => format!("{verb} builtin theme '{}'", file.link.name),
    }
}

/// Byte offset of the key at `path` in `document`.
fn path_offset(document: &Document<&str>, path: &[String]) -> Option<usize> {
    let (last, parents) = path.split_last()?;
    let mut table: &dyn TableLike = document.as_table();
    for key in parents {
        table = table.get(key)?.as_table_like()?;
    }
    key_offset(table, last)
}

/// Byte offset of element `index` of the document's `unset` array.
fn unset_offset(document: &Document<&str>, index: usize) -> Option<usize> {
    let span = document.get("unset")?.as_array()?.get(index)?.span()?;
    Some(span.start)
}
