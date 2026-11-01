//! Merge replay: which file last changed each path of the merged table.

use super::super::header::UnsetEntry;
use super::super::merge;
use std::collections::BTreeMap;

/// One theme file of the chain, ready to replay: its canonicalised table and
/// its `unset` entries.
#[derive(Debug)]
pub(in crate::theme::overlay) struct ReplayFile {
    /// Index of the file in the chain, the requested file being 0.
    pub(in crate::theme::overlay) file: usize,
    /// The file's table, overlay keys removed, language table nested.
    pub(in crate::theme::overlay) table: toml::Table,
    /// The file's `unset` entries.
    pub(in crate::theme::overlay) unset: Vec<UnsetEntry>,
}

/// What a file did at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::theme::overlay) struct Record {
    /// Index of the file in the chain, the requested file being 0.
    pub(in crate::theme::overlay) file: usize,
    /// `Some(i)` for the file's `unset` entry `i`; `None` for a write.
    pub(in crate::theme::overlay) unset: Option<usize>,
}

/// The records of a replayed merge, keyed by path.
#[derive(Debug, Default)]
pub(in crate::theme::overlay) struct Provenance {
    records: BTreeMap<Vec<String>, Record>,
}

impl Provenance {
    /// Replay the merge of `files`, bottom (the full theme) first, returning the
    /// merged table, equal to the one the loader built, and its records.
    ///
    /// - A write records the file for the path it puts a value at and every
    ///   path inside that value, tables included, after clearing the records
    ///   at and beneath it. Where the file's table meets a base table the
    ///   replay recurses instead, recording nothing for the table itself.
    /// - An `unset` entry clears the records at and beneath its path and is
    ///   recorded there.
    pub(in crate::theme::overlay) fn replay(files: Vec<ReplayFile>) -> (toml::Table, Self) {
        let mut merged = toml::Table::new();
        let mut provenance = Self::default();
        for file in files {
            for entry in &file.unset {
                provenance.clear(&entry.path);
                provenance.records.insert(
                    entry.path.clone(),
                    Record {
                        file: file.file,
                        unset: Some(entry.index),
                    },
                );
                merge::remove_path(&mut merged, &entry.path);
            }
            provenance.write(&mut merged, file.table, &mut Vec::new(), file.file);
        }
        (merged, provenance)
    }

    fn write(
        &mut self,
        base: &mut toml::Table,
        overlay: toml::Table,
        prefix: &mut Vec<String>,
        file: usize,
    ) {
        for (key, value) in overlay {
            prefix.push(key.clone());
            let meets_table = matches!(base.get(&key), Some(toml::Value::Table(_)));
            match value {
                toml::Value::Table(inner) if meets_table => {
                    if let Some(toml::Value::Table(existing)) = base.get_mut(&key) {
                        self.write(existing, inner, prefix, file);
                    }
                }
                other => {
                    self.replace(prefix, &other, file);
                    base.insert(key, other);
                }
            }
            prefix.pop();
        }
    }

    /// Record `file` for `path` and every path inside `value`, replacing the
    /// records at and beneath `path`.
    fn replace(&mut self, path: &mut Vec<String>, value: &toml::Value, file: usize) {
        self.clear(path);
        self.record_all(path, value, file);
    }

    fn record_all(&mut self, path: &mut Vec<String>, value: &toml::Value, file: usize) {
        self.records
            .insert(path.clone(), Record { file, unset: None });
        if let toml::Value::Table(table) = value {
            for (key, inner) in table {
                path.push(key.clone());
                self.record_all(path, inner, file);
                path.pop();
            }
        }
    }

    fn clear(&mut self, path: &[String]) {
        self.records.retain(|key, _| !key.starts_with(path));
    }

    /// The records a failing `path` maps to: every record at or beneath it, in
    /// path order (the path itself first), then the nearest record above it.
    pub(in crate::theme::overlay) fn lookup(&self, path: &[String]) -> Vec<(Vec<String>, Record)> {
        let mut found: Vec<(Vec<String>, Record)> = self
            .records
            .range(path.to_vec()..)
            .take_while(|(key, _)| key.starts_with(path))
            .map(|(key, record)| (key.clone(), *record))
            .collect();
        let above = (0..path.len()).rev().find_map(|len| {
            let ancestor = path.get(..len)?;
            self.records
                .get(ancestor)
                .map(|record| (ancestor.to_vec(), *record))
        });
        found.extend(above);
        found
    }
}
