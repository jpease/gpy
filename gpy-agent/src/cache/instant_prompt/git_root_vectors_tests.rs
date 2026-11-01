//! `tests/fixtures/git_root_vectors.tsv` against `find_project_root`: Rust is
//! the oracle for every `root` row, so a hand-edited expected value fails here
//! as well as in the shell vector tests. The `key` rows test the shells'
//! `__gpy_instant_cache_key_path` composition, which has no single Rust
//! function, so they are skipped.

use super::key::find_project_root;
use crate::vector_cells::{cell, is_empty_token};
use std::fs;
use std::path::Path;

/// Whether `start` or an ancestor up to and including `scratch` holds `.git`.
fn has_git_entry(start: &Path, scratch: &Path) -> bool {
    start
        .ancestors()
        .take_while(|dir| dir.starts_with(scratch))
        .any(|dir| dir.join(".git").exists())
}

#[test]
fn git_root_shared_vectors() {
    let dir = tempfile::tempdir().expect("temp dir");
    // macOS temp dirs sit behind a `/var` -> `/private/var` symlink.
    let scratch = fs::canonicalize(dir.path()).expect("canonical scratch dir");
    let mut checked = 0_u32;
    for row in crate::vector_cells::rows("git_root_vectors.tsv") {
        match row
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["mkdir", path, ..] => {
                fs::create_dir_all(scratch.join(cell(path))).expect("mkdir");
            }
            ["gitdir", path, ..] => {
                fs::create_dir_all(scratch.join(cell(path)).join(".git")).expect("gitdir");
            }
            ["gitfile", path, ..] => {
                fs::write(scratch.join(cell(path)).join(".git"), "gitdir: elsewhere\n")
                    .expect("gitfile");
            }
            ["symlink", link, target, ..] => {
                std::os::unix::fs::symlink(scratch.join(cell(target)), scratch.join(cell(link)))
                    .expect("symlink");
            }
            ["root", start, expected, ..] => {
                let mut start_path = scratch.join(cell(start));
                if start_path.exists() {
                    start_path = fs::canonicalize(&start_path).expect("canonical start");
                }
                let found = find_project_root(&start_path);
                let actual =
                    (found != start_path || has_git_entry(&start_path, &scratch)).then_some(found);
                let expected_root = (!is_empty_token(expected)).then(|| {
                    fs::canonicalize(scratch.join(cell(expected))).expect("canonical expected")
                });
                assert_eq!(actual, expected_root, "root row {row:?}");
                checked = checked.saturating_add(1);
            }
            ["key", ..] => {}
            _ => panic!("unknown or short row {row:?}"),
        }
    }
    assert!(checked > 0, "no git root vectors were checked");
}
