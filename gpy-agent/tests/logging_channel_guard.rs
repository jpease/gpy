//! Guards the unified logging channel (#627): every runtime diagnostic under
//! the IPC server/handlers, the watcher, `security.rs`, `profiling.rs` and
//! `debug.rs` itself must go through `debug_log!` (opt-in trace) or
//! `warn_log!` (always-visible warning) rather than a raw `eprintln!` call.
//! A raw `eprintln!` reaching around the unified surface would silently
//! reopen the gap #627 closed: a daemonized agent's stderr is redirected to
//! `/dev/null`, so a diagnostic that bypasses `warn_log!` (and therefore
//! never reaches the `GPY_DEBUG_LOG` file either) is lost for good.
//!
//! Test-only diagnostics are exempt. Matches `eprintln!(` specifically
//! (the macro invocation), not bare mentions of the word in comments or
//! doc examples, so this guard doesn't trip over its own explanatory prose
//! or the historical mention in `formatter/fish_ansi.rs`.

#![allow(clippy::missing_panics_doc)]

use std::path::{Path, PathBuf};

/// Strip a source file's test module(s) before scanning.
///
/// Once a line is exactly `#[cfg(test)]` and the next non-blank line starts a
/// `mod tests` declaration, everything from that `#[cfg(test)]` line onward
/// is dropped. Everything before it is kept and scanned normally.
fn strip_test_modules(source: &str) -> String {
    let mut kept = Vec::new();
    let mut lines = source.lines();

    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            let starts_test_mod = lines
                .clone()
                .find(|l| !l.trim().is_empty())
                .is_some_and(|l| l.trim_start().starts_with("mod tests"));
            if starts_test_mod {
                break;
            }
        }
        kept.push(line);
    }

    kept.join("\n")
}

/// Find every `eprintln!(` macro-invocation line, returning 1-based line numbers.
fn find_eprintln_invocation_lines(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("eprintln!("))
        .map(|(i, _)| i.saturating_add(1))
        .collect()
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Recursively collect every `.rs` file under `dir` into `out`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The files and directories the unified logging channel covers.
fn guarded_targets() -> Vec<PathBuf> {
    let src = src_dir();
    let mut targets = Vec::new();
    collect_rs_files(&src.join("ipc"), &mut targets);
    collect_rs_files(&src.join("watcher"), &mut targets);
    targets.push(src.join("security.rs"));
    targets.push(src.join("profiling.rs"));
    targets.push(src.join("debug.rs"));
    targets
}

#[test]
fn no_raw_eprintln_in_the_unified_logging_paths() {
    let targets = guarded_targets();
    assert!(
        !targets.is_empty(),
        "guard found no files to scan - src/ layout must have changed"
    );

    let mut violations = Vec::new();

    for path in &targets {
        let contents = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let scanned = strip_test_modules(&contents);
        for line in find_eprintln_invocation_lines(&scanned) {
            violations.push(format!("{}:{line}", path.display()));
        }
    }

    assert!(
        violations.is_empty(),
        "raw eprintln! found outside #[cfg(test)] modules under the unified \
         logging surface (route these through debug_log!/warn_log! instead):\n{}",
        violations.join("\n")
    );
}
