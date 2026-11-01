//! Warnings `gpy theme validate` gives for a theme file's `unset` key. The
//! warnings are not load errors: the file still loads.

use super::resolve::{Candidates, OwnName, link_for_path};
use super::{ChainFile, ChainLink, merge, read_chain};
use crate::theme::parse::parse_table;
use std::path::Path;

/// The merged table of the chain `top` heads, as `merge_chain` builds it
/// (each file's language spelling canonicalised and `unset` applied, base
/// first), but without validating or typing it. With `include_top = false`
/// the top file is left out of the merge, which is the base its `unset` is
/// checked against. `None` when a file of the chain can't be read or resolved.
///
/// `top` is a link already resolved to its own name, so `top.on_disk == false`
/// reads the embedded builtin of that name.
pub(crate) fn merged_table(top: &ChainLink, include_top: bool) -> Option<toml::Table> {
    let label = top.path.display().to_string();
    let first = ChainFile::read(top.clone(), label).ok()?;
    let mut files = read_chain(first, OwnName::Stem, &mut Candidates::discover()).ok()?;
    if !include_top && !files.is_empty() {
        files.remove(0);
    }
    Some(merge_files(files))
}

/// Merge `files` (top first), base first, as `merge_chain` does.
fn merge_files(files: Vec<ChainFile>) -> toml::Table {
    let mut merged = toml::Table::new();
    for file in files.into_iter().rev() {
        let ChainFile {
            table: mut file_table,
            header,
            ..
        } = file;
        merge::canonicalise(&mut file_table);
        for entry in &header.unset {
            merge::remove_path(&mut merged, &entry.path);
        }
        merge::deep_merge(&mut merged, file_table);
    }
    merged
}

/// Warnings about the `unset` key of the theme file at `path`: it does
/// nothing in a file without `extends`, and an entry naming a path the base
/// lacks (after the language spelling is canonicalised) is ignored at load.
///
/// A file that can't be read or whose chain doesn't load has no warnings
/// here; the load error is reported on its own.
#[must_use]
pub fn unset_warnings(path: &Path) -> Vec<String> {
    let label = path.display().to_string();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(table) = parse_table(&text, &label) else {
        return Vec::new();
    };
    if !table.contains_key("extends") {
        return if table.contains_key("unset") {
            vec![format!(
                "{label}: `unset` does nothing in a file without `extends`"
            )]
        } else {
            Vec::new()
        };
    }
    let Ok(first) = ChainFile::read(link_for_path(path), label.clone()) else {
        return Vec::new();
    };
    let Ok(mut files) = read_chain(first, OwnName::Stem, &mut Candidates::discover()) else {
        return Vec::new();
    };
    if files.is_empty() {
        return Vec::new();
    }
    let top = files.remove(0);
    let base = merge_files(files);
    top.header
        .unset
        .iter()
        .filter(|entry| !has_path(&base, &entry.path))
        .map(|entry| {
            format!(
                "{label}: `unset` entry '{}' names a key the base theme lacks",
                entry.path.join(".")
            )
        })
        .collect()
}

/// Whether `table` holds a value at the dotted `path`.
fn has_path(table: &toml::Table, path: &[String]) -> bool {
    let Some((first, rest)) = path.split_first() else {
        return false;
    };
    match table.get(first) {
        Some(_) if rest.is_empty() => true,
        Some(toml::Value::Table(inner)) => has_path(inner, rest),
        _ => false,
    }
}
