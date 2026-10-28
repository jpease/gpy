//! Warnings `gpy theme validate` gives for a theme file's `unset` key
//! (ADR-0012 sections 1 and 2). Neither is a load error: both files load.

use super::resolve::{Candidates, OwnName, link_for_path};
use super::{ChainFile, merge, read_chain};
use crate::theme::parse::parse_table;
use std::path::Path;

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
    let Ok(files) = read_chain(first, OwnName::Stem, &mut Candidates::discover()) else {
        return Vec::new();
    };
    let mut links = files.into_iter();
    let Some(top) = links.next() else {
        return Vec::new();
    };
    let mut base = toml::Table::new();
    for file in links.rev() {
        let ChainFile {
            table: mut file_table,
            header,
            ..
        } = file;
        merge::canonicalise(&mut file_table);
        for entry in &header.unset {
            merge::remove_path(&mut base, &entry.path);
        }
        merge::deep_merge(&mut base, file_table);
    }
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
