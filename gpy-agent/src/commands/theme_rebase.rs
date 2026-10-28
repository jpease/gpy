//! `gpy theme rebase`: rewrite a modified full copy of a theme as an overlay
//! that keeps only what differs from its base (ADR-0012 section 4).
//!
//! The copy and the base are compared as normalised typed values,
//! `N(Table::try_from(theme))` (see [`normalised_table`]), never as raw text:
//! a wizard copy holds every serde default and nested language tables, the
//! builtins write few keys and flat language keys. Each key the copy's file
//! sets is dropped when its normalised value equals the base's and kept in the
//! file's own spelling, with its comments, otherwise. Paths where the two
//! differ but the file sets nothing are inherited after the rebase; they are
//! reported at their topmost level, and `--unset-missing` turns each into one
//! `unset` entry. Before anything is written the overlay must load and
//! normalise to the copy (except at the inherited paths it does not unset).

use super::output;
use crate::config::discovery::validate_config_name;
use crate::theme::overlay::{self, nested_language_path, split_path};
use crate::theme::{
    ChainLink, LoadedTheme, normalise, normalised_table, parse, parse_table, user_copy,
};
use crate::{Error, Result};
use std::collections::BTreeSet;
use std::path::Path;

/// What `gpy theme rebase` was asked to do.
#[derive(Debug, Clone, Copy)]
pub struct RebaseOptions<'a> {
    /// The user theme file to rewrite, by theme name.
    pub name: &'a str,
    /// The theme the overlay extends; `None` means the same name's next lower
    /// source (the plugin or builtin theme the copy shadows).
    pub onto: Option<&'a str>,
    /// Write the overlay over the file, after copying it aside, instead of
    /// only printing it.
    pub write: bool,
    /// Add one `unset` entry for each inherited path.
    pub unset_missing: bool,
}

/// A dotted key path, split into its components.
type KeyPath = Vec<String>;

/// Rewrite the user theme file `options.name`, a full theme, as an overlay of
/// its base. Prints the overlay and a report; writes only with `write`, via a
/// `.bak` copy and an atomic replace.
///
/// # Errors
/// Returns an error when there is no user theme file `name`, it is already an
/// overlay or does not load, no base is given or found, the base does not
/// load, the overlay fails the before-write check, or the file can't be
/// backed up or written. On any error the file is left as it was.
pub fn rebase(options: &RebaseOptions<'_>) -> Result<()> {
    let RebaseOptions {
        name,
        onto,
        write,
        unset_missing,
    } = *options;
    validate_config_name("theme", name)?;
    if let Some(base) = onto {
        validate_config_name("theme", base)?;
    }
    let path = user_copy::user_copy_path(name);
    if !path.is_file() {
        return Err(no_file(name, &path));
    }
    let link = overlay::own_user_link(name)?;
    let label = link.path.display().to_string();
    let text = std::fs::read_to_string(&link.path)
        .map_err(|e| Error::io_context(format_args!("read {label}"), &e))?;
    if let Some(extends) = parse_table(&text, &label)?.get("extends") {
        return Err(already_overlay(&link, extends));
    }
    let copy = parse(&text, &label).map_err(|error| does_not_load(&link.name, &error))?;
    let copy_raw = toml::Table::try_from(&copy)
        .map_err(|e| Error::config(format!("failed to serialize theme to TOML: {e}")))?;
    let mut copy_normalised = copy_raw.clone();
    normalise(&mut copy_normalised);

    let base_name = match onto {
        Some(base) => base.to_owned(),
        None => default_base(&link, &copy_normalised)?,
    };
    let base = overlay::load_text(link.clone(), header_only(&base_name))?;
    let base_normalised = normalised_table(&base.theme)?;

    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| Error::config(format!("Failed to parse {label} as TOML:\n  {e}")))?;
    let compare = Compare {
        copy_raw: &copy_raw,
        copy: &copy_normalised,
        base: &base_normalised,
    };
    let mut report = Report::default();
    let mut file = FileKeys::default();
    prune(
        doc.as_table_mut(),
        &mut Vec::new(),
        &compare,
        &mut report,
        &mut file,
    );
    let inherited = inherited_paths(&copy_normalised, &base_normalised, &file);
    let unset = if unset_missing {
        unset_entries(&inherited)?
    } else {
        Vec::new()
    };
    set_header(&mut doc, &base_name, &unset);
    let rebased = doc.to_string();

    let merged = overlay::load_text(link.clone(), rebased.clone())
        .map_err(|error| check_failed(&link.name, &message_of(&error)))?;
    let excluded: &[KeyPath] = if unset_missing { &[] } else { &inherited };
    check(&link.name, &copy_normalised, &merged, excluded)?;

    print_report(&link, &base, &report, &inherited, unset_missing);
    output::blank();
    output::plain(rebased.trim_end());
    output::blank();
    if !write {
        output::plain(format!(
            "Nothing written. Run `{}` to replace {label} (a copy is kept as .bak).",
            command_line(&link.name, onto, unset_missing)
        ));
        return Ok(());
    }
    let backup = user_copy::copy_aside(&link.path)?;
    crate::fs_util::write_atomically(&link.path, rebased.as_bytes())
        .map_err(|e| Error::io_context(format_args!("write {label}"), &e))?;
    output::plain(format!(
        "Rewrote {label} as an overlay of '{base_name}' (original kept at {})",
        backup.display()
    ));
    Ok(())
}

/// The normalised tables keys are decided against.
struct Compare<'a> {
    /// The copy serialized, before normalisation: a key the file sets that is
    /// missing here is ignored at load.
    copy_raw: &'a toml::Table,
    /// `C`: the copy, normalised.
    copy: &'a toml::Table,
    /// `B`: the merged base, normalised.
    base: &'a toml::Table,
}

impl Compare<'_> {
    /// Decide the file key at `raw` (own spelling), `normalised` (nested
    /// language spelling): `true` drops it. `empty_table` marks a key whose
    /// value is a table with no keys, which is never reported as ignored.
    fn drops(
        &self,
        raw: &[String],
        normalised: &[String],
        empty_table: bool,
        report: &mut Report,
    ) -> bool {
        if !empty_table && lookup(self.copy_raw, normalised).is_none() {
            report.ignored.push(raw.join("."));
            return true;
        }
        if lookup(self.copy, normalised) == lookup(self.base, normalised) {
            report.dropped = report.dropped.saturating_add(1);
            return true;
        }
        report.kept.push(raw.join("."));
        false
    }
}

/// What happened to the keys the copy's file sets.
#[derive(Debug, Default)]
struct Report {
    /// Keys kept because they differ from the base, in the file's spelling.
    kept: Vec<String>,
    /// How many keys were dropped as equal to the base.
    dropped: usize,
    /// Keys dropped because GPY ignores them at load.
    ignored: Vec<String>,
}

/// Every key path the copy's file sets, normalised.
#[derive(Debug, Default)]
struct FileKeys {
    /// Every key, tables included.
    paths: BTreeSet<KeyPath>,
    /// Every path with a key beneath it.
    parents: BTreeSet<KeyPath>,
}

impl FileKeys {
    fn insert(&mut self, path: &[String]) {
        for end in 1..path.len() {
            if let Some(prefix) = path.get(..end) {
                self.parents.insert(prefix.to_vec());
            }
        }
        self.paths.insert(path.to_vec());
    }
}

/// Drop from `table` (at `path`) every key `compare` drops, and every table
/// left empty by that, recording each key the file sets in `file`.
fn prune(
    table: &mut dyn toml_edit::TableLike,
    path: &mut KeyPath,
    compare: &Compare<'_>,
    report: &mut Report,
    file: &mut FileKeys,
) {
    let keys: Vec<String> = table.iter().map(|(key, _)| key.to_owned()).collect();
    for key in keys {
        path.push(key.clone());
        let normalised = nested_language_path(path.clone());
        file.insert(&normalised);
        let remove = match table.get_mut(&key).map(toml_edit::Item::as_table_like_mut) {
            Some(Some(child)) if !child.is_empty() => {
                prune(child, path, compare, report, file);
                child.is_empty()
            }
            Some(child) => compare.drops(path, &normalised, child.is_some(), report),
            None => false,
        };
        if remove {
            table.remove(&key);
        }
        path.pop();
    }
}

/// The value at `path` in `table`.
fn lookup<'a>(table: &'a toml::Table, path: &[String]) -> Option<&'a toml::Value> {
    let (last, parents) = path.split_last()?;
    let mut current = table;
    for key in parents {
        current = current.get(key)?.as_table()?;
    }
    current.get(last)
}

/// The paths where `copy` and `base` differ and the file sets no key at or
/// beneath, each at its topmost level: its parent is the root or holds a key
/// the file sets.
fn inherited_paths(copy: &toml::Table, base: &toml::Table, file: &FileKeys) -> Vec<KeyPath> {
    let mut found = Vec::new();
    inherited_below(Some(copy), Some(base), &mut Vec::new(), file, &mut found);
    found
}

fn inherited_below(
    copy: Option<&toml::Table>,
    base: Option<&toml::Table>,
    path: &mut KeyPath,
    file: &FileKeys,
    found: &mut Vec<KeyPath>,
) {
    let keys: BTreeSet<&String> = copy
        .into_iter()
        .chain(base)
        .flat_map(toml::Table::keys)
        .collect();
    for key in keys {
        path.push(key.clone());
        let in_copy = copy.and_then(|table| table.get(key));
        let in_base = base.and_then(|table| table.get(key));
        if file.parents.contains(&*path) {
            inherited_below(
                in_copy.and_then(toml::Value::as_table),
                in_base.and_then(toml::Value::as_table),
                path,
                file,
                found,
            );
        } else if !file.paths.contains(&*path) && in_copy != in_base {
            found.push(path.clone());
        }
        path.pop();
    }
}

/// The leaf paths where `left` and `right` differ; a table on one side only,
/// or against a non-table, is one path.
fn differences(left: &toml::Table, right: &toml::Table) -> Vec<KeyPath> {
    let mut found = Vec::new();
    differences_below(left, right, &mut Vec::new(), &mut found);
    found
}

fn differences_below(
    left: &toml::Table,
    right: &toml::Table,
    path: &mut KeyPath,
    found: &mut Vec<KeyPath>,
) {
    let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    for key in keys {
        path.push(key.clone());
        match (left.get(key), right.get(key)) {
            (Some(toml::Value::Table(left_table)), Some(toml::Value::Table(right_table))) => {
                differences_below(left_table, right_table, path, found);
            }
            (left_value, right_value) if left_value != right_value => found.push(path.clone()),
            _ => {}
        }
        path.pop();
    }
}

/// The `unset` entries for `inherited`, as dotted paths.
///
/// # Errors
/// Returns an error if a path has a component an `unset` entry can't hold.
fn unset_entries(inherited: &[KeyPath]) -> Result<Vec<String>> {
    inherited
        .iter()
        .map(|path| {
            let dotted = path.join(".");
            if split_path(&dotted).as_ref() == Some(path) {
                Ok(dotted)
            } else {
                Err(Error::config(format!(
                    "Cannot unset '{dotted}'\n  \
                     Diagnostic: `unset` paths may hold only ASCII letters, digits, `_` and `-` in each component.\n  \
                     Remediation: Run without --unset-missing to inherit it, or remove it from the base."
                )))
            }
        })
        .collect()
}

/// Put `extends = "<base>"` and, if any, the `unset` entries at the top of `doc`.
fn set_header(doc: &mut toml_edit::DocumentMut, base: &str, unset: &[String]) {
    let root = doc.as_table_mut();
    root.insert("extends", toml_edit::value(base));
    if !unset.is_empty() {
        let entries: toml_edit::Array = unset.iter().map(String::as_str).collect();
        root.insert("unset", toml_edit::value(entries));
    }
}

/// An overlay that only names `base`: it loads as the merged base.
pub(crate) fn header_only(base: &str) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    set_header(&mut doc, base, &[]);
    doc.to_string()
}

/// The default base of `link`: its own name, resolved below the file (the
/// plugin or builtin theme it shadows).
///
/// # Errors
/// Returns an error suggesting the closest builtin when nothing lies below.
fn default_base(link: &ChainLink, copy: &toml::Table) -> Result<String> {
    match overlay::resolve_extends(&link.name, link) {
        Ok(_) => Ok(link.name.clone()),
        Err(reason) => {
            let suggestion = closest_builtin(copy).map_or_else(String::new, |(builtin, count)| {
                format!(
                    " The closest builtin is '{builtin}' ({} differ): \
                     `gpy theme rebase {} --onto {builtin}`.",
                    count_keys(count),
                    link.name
                )
            });
            Err(Error::config(format!(
                "Theme '{}' needs a base to rebase onto\n  \
                 Diagnostic: {reason}.\n  \
                 Remediation: Pass --onto <theme>.{suggestion}",
                link.name
            )))
        }
    }
}

/// The builtin whose normalised table differs from `copy` at the fewest
/// leaf paths, with that count.
fn closest_builtin(copy: &toml::Table) -> Option<(&'static str, usize)> {
    user_copy::builtin_names()
        .filter_map(|builtin| {
            let theme = parse(user_copy::builtin_content(builtin)?, builtin).ok()?;
            let table = normalised_table(&theme).ok()?;
            Some((builtin, differences(copy, &table).len()))
        })
        .min_by_key(|(_, count)| *count)
}

/// The before-write check: the merged overlay normalises to `copy`, except at
/// and beneath the `excluded` (inherited) paths.
///
/// # Errors
/// Returns an error naming the paths that differ.
fn check(name: &str, copy: &toml::Table, merged: &LoadedTheme, excluded: &[KeyPath]) -> Result<()> {
    let merged_normalised = normalised_table(&merged.theme)?;
    let differing: Vec<String> = differences(copy, &merged_normalised)
        .into_iter()
        .filter(|path| !excluded.iter().any(|inherited| path.starts_with(inherited)))
        .map(|path| path.join("."))
        .collect();
    if differing.is_empty() {
        Ok(())
    } else {
        Err(check_failed(
            name,
            &format!(
                "the overlay's merged result differs from the copy at {}",
                differing.join(", ")
            ),
        ))
    }
}

/// Print where the overlay's keys came from and what it inherits.
fn print_report(
    link: &ChainLink,
    base: &LoadedTheme,
    report: &Report,
    inherited: &[KeyPath],
    unset_missing: bool,
) {
    let base_chain: Vec<String> = base.chain.iter().skip(1).map(ToString::to_string).collect();
    output::plain(format!(
        "Rebasing {} onto {}",
        link.path.display(),
        base_chain.join(" → ")
    ));
    if report.kept.is_empty() {
        output::detail("No key differs from the base; the overlay only names it.");
    } else {
        output::detail(format!(
            "Kept, as different from the base: {}",
            report.kept.join(", ")
        ));
    }
    output::detail(format!(
        "Dropped as equal to the base: {}",
        count_keys(report.dropped)
    ));
    if !report.ignored.is_empty() {
        output::detail(format!(
            "Dropped as ignored by GPY: {}",
            report.ignored.join(", ")
        ));
    }
    if inherited.is_empty() {
        return;
    }
    let paths: Vec<String> = inherited.iter().map(|path| path.join(".")).collect();
    if unset_missing {
        output::detail(format!(
            "Unset (your copy does not have them): {}",
            paths.join(", ")
        ));
    } else {
        output::detail(format!(
            "Inherited from the base (your copy does not have them): {}",
            paths.join(", ")
        ));
        output::detail(
            "If you removed them on purpose, run again with --unset-missing to unset them.",
        );
    }
}

/// `1 key`, `2 keys`.
fn count_keys(count: usize) -> String {
    format!("{count} {}", if count == 1 { "key" } else { "keys" })
}

/// The command that writes what this run printed.
fn command_line(name: &str, onto: Option<&str>, unset_missing: bool) -> String {
    let mut command = format!("gpy theme rebase {name}");
    if let Some(base) = onto {
        command.push_str(" --onto ");
        command.push_str(base);
    }
    if unset_missing {
        command.push_str(" --unset-missing");
    }
    command.push_str(" --write");
    command
}

/// The message of `error`, without the `Configuration error:` prefix.
pub(crate) fn message_of(error: &Error) -> String {
    match error {
        Error::Config { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn no_file(name: &str, path: &Path) -> Error {
    Error::config(format!(
        "No user theme file for '{name}' to rebase\n  \
         Diagnostic: {} does not exist; rebase rewrites a full theme file in the themes directory.\n  \
         Remediation: Run `gpy theme list` to see themes.",
        path.display()
    ))
}

fn already_overlay(link: &ChainLink, extends: &toml::Value) -> Error {
    let base = extends
        .as_str()
        .map_or_else(|| extends.to_string(), str::to_owned);
    Error::config(format!(
        "Theme '{}' is already an overlay\n  \
         Diagnostic: {} extends '{base}' and holds only its own keys.\n  \
         Remediation: Nothing to rebase; edit the file directly, or see `gpy theme diff {}`.",
        link.name,
        link.path.display(),
        link.name
    ))
}

fn does_not_load(name: &str, error: &Error) -> Error {
    Error::config(format!(
        "Theme '{name}' does not load, so it cannot be rebased\n  \
         Diagnostic: {}\n  \
         Remediation: Run `gpy theme migrate {name}` first: migrate repairs keys, rebase removes duplication.",
        message_of(error)
    ))
}

fn check_failed(name: &str, reason: &str) -> Error {
    Error::config(format!(
        "Rebasing theme '{name}' would change it, so nothing was written\n  \
         Diagnostic: {reason}\n  \
         Remediation: This is a GPY bug; please report it with the theme file. Pass --onto <theme> to try another base."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().expect("test table parses")
    }

    fn path(dotted: &str) -> KeyPath {
        dotted.split('.').map(str::to_owned).collect()
    }

    fn file_keys(text: &str) -> FileKeys {
        let mut doc: toml_edit::DocumentMut = text.parse().expect("test doc parses");
        let empty = toml::Table::new();
        let compare = Compare {
            copy_raw: &empty,
            copy: &empty,
            base: &empty,
        };
        let mut file = FileKeys::default();
        prune(
            doc.as_table_mut(),
            &mut Vec::new(),
            &compare,
            &mut Report::default(),
            &mut file,
        );
        file
    }

    #[test]
    fn inherited_paths_are_taken_at_the_topmost_level() {
        let file = file_keys("[segments.clock]\nbg_color = \"red\"\n");
        let copy = table("[segments.clock]\nbg_color = \"red\"\n");
        let base = table(
            "[segments.clock]\nbg_color = \"red\"\n[segments.clock.open]\nicon = \"x\"\nfg = \"y\"\n\
             [segments.hostname]\nbg_color = \"black\"\n",
        );
        assert_eq!(
            inherited_paths(&copy, &base, &file),
            [path("segments.clock.open"), path("segments.hostname")]
        );
    }

    #[test]
    fn a_path_the_file_sets_is_never_inherited() {
        let file = file_keys("[segments.clock]\nbg_color = \"red\"\n");
        let copy = table("[segments.clock]\nbg_color = \"red\"\n");
        let base = table("[segments.clock]\nbg_color = \"blue\"\n");
        assert_eq!(inherited_paths(&copy, &base, &file), Vec::<KeyPath>::new());
    }

    #[test]
    fn flat_language_keys_count_under_their_nested_map() {
        let file = file_keys("[segments.language]\nrust_symbol = \"R\"\n");
        let copy = table("[segments.language.symbols]\nrust_symbol = \"R\"\n");
        let base = table("[segments.language.symbols]\nrust_symbol = \"R\"\ngo_symbol = \"G\"\n");
        assert_eq!(
            inherited_paths(&copy, &base, &file),
            [path("segments.language.symbols.go_symbol")]
        );
    }

    #[test]
    fn keys_equal_to_the_base_drop_and_others_keep_their_spelling() {
        let text = "[segments.clock]\n# mine\ntime_format = \"24\"\nbg_color = \"black\"\nnot_a_key = 1\n\
                    [segments.language]\nrust_symbol = \"R\"\ngo_symbol = \"G\"\n";
        let copy_raw = table(
            "[segments.clock]\ntime_format = \"24\"\nbg_color = \"black\"\n\
             [segments.language.symbols]\nrust_symbol = \"R\"\ngo_symbol = \"G\"\n",
        );
        let base = table(
            "[segments.clock]\nbg_color = \"black\"\n\
             [segments.language.symbols]\nrust_symbol = \"r\"\ngo_symbol = \"G\"\n",
        );
        let compare = Compare {
            copy_raw: &copy_raw,
            copy: &copy_raw,
            base: &base,
        };
        let mut doc: toml_edit::DocumentMut = text.parse().expect("doc parses");
        let mut report = Report::default();
        prune(
            doc.as_table_mut(),
            &mut Vec::new(),
            &compare,
            &mut report,
            &mut FileKeys::default(),
        );
        assert_eq!(
            doc.to_string(),
            "[segments.clock]\n# mine\ntime_format = \"24\"\n[segments.language]\nrust_symbol = \"R\"\n"
        );
        assert_eq!(
            report.kept,
            [
                "segments.clock.time_format",
                "segments.language.rust_symbol"
            ]
        );
        assert_eq!(report.dropped, 2);
        assert_eq!(report.ignored, ["segments.clock.not_a_key"]);
    }

    #[test]
    fn unset_entries_reject_paths_unset_cannot_hold() {
        assert_eq!(
            unset_entries(&[path("segments.hostname")]).expect("valid path"),
            ["segments.hostname"]
        );
        assert!(unset_entries(&[vec!["segments".to_owned(), "a b".to_owned()]]).is_err());
    }

    #[test]
    fn the_header_goes_before_every_table() {
        let mut doc: toml_edit::DocumentMut = "[segments.clock]\ntime_format = \"24\"\n"
            .parse()
            .expect("doc parses");
        set_header(&mut doc, "starship", &["segments.hostname".to_owned()]);
        let text = doc.to_string();
        assert!(
            text.starts_with("extends = \"starship\"\nunset = [\"segments.hostname\"]\n"),
            "{text}"
        );
        assert!(
            text.contains("[segments.clock]\ntime_format = \"24\"\n"),
            "{text}"
        );
    }
}
