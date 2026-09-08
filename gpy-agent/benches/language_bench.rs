//! Benchmarks for language detection operations
//!
//! These benchmarks measure the performance of language detection
//! with and without caching.
//!
//! Run with: cargo bench --bench `language_bench`

#![allow(clippy::unwrap_used)]
// Fixture-generation indices/arithmetic below are all bounded by construction
// (fixed literal indices into a 10-entry table, or `i % REALISTIC_SNIPPETS.len()`)
// and this file is bench-only harness code, never shipped -- matching this
// repo's existing bench-file convention of relaxing test-shaped lints (see
// git_bench.rs's file-level allows).
#![allow(clippy::indexing_slicing)]
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::default_numeric_fallback)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::language::detector::Detector;
use std::fs;
use std::hint::black_box;
use tempfile::TempDir;

// --- Realistic multi-language corpora (issue #521) ---
//
// `setup_test_project` above intentionally stays untouched: it is the
// misleading 2-3-file synthetic corpus that motivated this addition (#517
// found hyperpolyglot vs. gengo comparisons on it point the wrong way vs.
// real source trees). These corpora are generated in-process (no checked-in
// fixture tree, no dependency on `/tmp` state existing) and are
// implementation-agnostic: they bench `Detector::detect_directory` itself,
// so they measured the hyperpolyglot backend before #523 and the
// gengo-language one after it, unchanged across the swap.

/// Realistic multi-language file bodies, roughly one representative snippet
/// per common extension, so a generated corpus looks like real source
/// rather than repeated placeholder text.
const REALISTIC_SNIPPETS: &[(&str, &str)] = &[
    (
        "rs",
        "use std::collections::HashMap;\n\npub fn process(input: &str) -> HashMap<String, usize> {\n    let mut map = HashMap::new();\n    for (i, word) in input.split_whitespace().enumerate() {\n        map.insert(word.to_string(), i);\n    }\n    map\n}\n",
    ),
    (
        "py",
        "import json\nfrom typing import Any\n\n\ndef load_config(path: str) -> dict[str, Any]:\n    with open(path) as f:\n        return json.load(f)\n",
    ),
    (
        "js",
        "'use strict';\n\nfunction debounce(fn, wait) {\n  let timer;\n  return (...args) => {\n    clearTimeout(timer);\n    timer = setTimeout(() => fn(...args), wait);\n  };\n}\n\nmodule.exports = { debounce };\n",
    ),
    (
        "ts",
        "export interface Config {\n  name: string;\n  retries: number;\n}\n\nexport function withDefaults(cfg: Partial<Config>): Config {\n  return { name: 'default', retries: 3, ...cfg };\n}\n",
    ),
    (
        "go",
        "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfor i := 0; i < 5; i++ {\n\t\tfmt.Println(\"iteration\", i)\n\t}\n}\n",
    ),
    (
        "md",
        "# Title\n\nThis is a *markdown* document with a [link](https://example.com) and:\n\n- item one\n- item two\n",
    ),
    (
        "json",
        "{\n  \"name\": \"example\",\n  \"version\": \"1.0.0\"\n}\n",
    ),
    (
        "toml",
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
    ),
    (
        "yaml",
        "name: ci\non:\n  push:\n    branches: [main]\njobs:\n  test:\n    runs-on: ubuntu-latest\n",
    ),
    (
        "css",
        "body {\n  margin: 0;\n  font-family: sans-serif;\n}\n\n.widget {\n  display: flex;\n}\n",
    ),
];

/// Writes `body` (repeated `repeat` times to vary file size) at `dir/rel.ext`.
///
/// # Panics
///
/// Panics if directory creation or the file write fails.
fn write_realistic_file(dir: &std::path::Path, rel: &str, ext: &str, body: &str, repeat: usize) {
    let path = dir.join(format!("{rel}.{ext}"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body.repeat(repeat)).unwrap();
}

/// A small realistic project (< 50 files).
///
/// A Rust crate with a few Python helper scripts, `TypeScript` web components, and the usual
/// config/doc files -- not the 2-3-file synthetic project `setup_test_project` builds.
///
/// # Panics
///
/// Panics if temporary directory creation or file writes fail.
fn setup_realistic_small_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write_realistic_file(root, "Cargo", "toml", REALISTIC_SNIPPETS[7].1, 1);
    write_realistic_file(root, "README", "md", REALISTIC_SNIPPETS[5].1, 2);
    for i in 0..15 {
        write_realistic_file(
            root,
            &format!("src/mod_{i}"),
            "rs",
            REALISTIC_SNIPPETS[0].1,
            1 + i % 3,
        );
    }
    for i in 0..8 {
        write_realistic_file(
            root,
            &format!("scripts/tool_{i}"),
            "py",
            REALISTIC_SNIPPETS[1].1,
            1,
        );
    }
    for i in 0..5 {
        write_realistic_file(
            root,
            &format!("web/component_{i}"),
            "ts",
            REALISTIC_SNIPPETS[3].1,
            1,
        );
    }
    write_realistic_file(root, "package", "json", REALISTIC_SNIPPETS[6].1, 1);
    write_realistic_file(root, "web/style", "css", REALISTIC_SNIPPETS[9].1, 1);
    write_realistic_file(root, "ci", "yaml", REALISTIC_SNIPPETS[8].1, 1);
    dir
}

/// A "real repo"-sized corpus: several hundred multi-language files spread across many
/// directories.
///
/// Matches the rough scale of a real mid-size repository (this repository has 726
/// non-ignored files at the time of writing -- see issue #521) without depending on that
/// checked-in tree.
///
/// # Panics
///
/// Panics if temporary directory creation or file writes fail.
fn setup_realistic_medium_project(file_count: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    for i in 0..file_count {
        let (ext, body) = REALISTIC_SNIPPETS[i % REALISTIC_SNIPPETS.len()];
        let subdir = i / 40; // ~40 files per directory, several directories
        let repeat = 1 + (i % 5);
        write_realistic_file(root, &format!("pkg_{subdir}/file_{i}"), ext, body, repeat);
    }
    dir
}

/// A large tree (5k+ files) across many extensions and directories.
///
/// Distinct from `scripts/create-huge-test-repo.sh`'s homogeneous `file_N.txt` generator --
/// this looks like a large multi-language monorepo instead of one flat directory of
/// plain-text files, which is the case this bench needs to exercise (issue #521).
///
/// # Panics
///
/// Panics if temporary directory creation or file writes fail.
fn setup_realistic_large_project(file_count: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    for i in 0..file_count {
        let (ext, body) = REALISTIC_SNIPPETS[i % REALISTIC_SNIPPETS.len()];
        let subdir = i / 200; // ~200 files per directory, many directories
        let repeat = 1 + (i % 7);
        write_realistic_file(root, &format!("pkg_{subdir}/file_{i}"), ext, body, repeat);
    }
    dir
}

/// Set up a test project for benchmarking
///
/// # Panics
///
/// Panics if temporary directory creation or file writes fail
fn setup_test_project(lang: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    match lang {
        "rust" => {
            fs::write(path.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("src/main.rs"), "fn main() {}").unwrap();
        }
        "python" => {
            fs::write(path.join("setup.py"), "from setuptools import setup").unwrap();
            fs::write(path.join("main.py"), "print('hello')").unwrap();
        }
        "node" => {
            fs::write(path.join("package.json"), "{\"name\": \"test\"}").unwrap();
            fs::write(path.join("index.js"), "console.log('hello')").unwrap();
        }
        _ => {}
    }

    dir
}

fn bench_language_detection(c: &mut Criterion) {
    let rust_dir = setup_test_project("rust");
    let python_dir = setup_test_project("python");
    let node_dir = setup_test_project("node");

    c.bench_function("language_detect_rust", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(rust_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_python", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(python_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_node", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(node_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_rust_no_metadata", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory_without_metadata(black_box(rust_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_python_no_metadata", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory_without_metadata(black_box(python_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_node_no_metadata", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory_without_metadata(black_box(node_dir.path()));
            black_box(langs)
        });
    });
}

fn bench_language_with_file(c: &mut Criterion) {
    let rust_dir = setup_test_project("rust");
    let python_dir = setup_test_project("python");
    let node_dir = setup_test_project("node");

    c.bench_function("language_detect_rust_file", |b| {
        let rust_file = rust_dir.path().join("src/main.rs");
        b.iter(|| {
            let lang = Detector::detect_file(black_box(&rust_file));
            black_box(lang)
        });
    });

    c.bench_function("language_detect_python_file", |b| {
        let python_file = python_dir.path().join("main.py");
        b.iter(|| {
            let lang = Detector::detect_file(black_box(&python_file));
            black_box(lang)
        });
    });

    c.bench_function("language_detect_node_file", |b| {
        let node_file = node_dir.path().join("index.js");
        b.iter(|| {
            let lang = Detector::detect_file(black_box(&node_file));
            black_box(lang)
        });
    });
}

/// Realistic-corpus benches for `Detector::detect_directory` (issue #521).
///
/// Implementation-agnostic on purpose: this benches whatever backend
/// `detect_directory` uses internally (hyperpolyglot before #523,
/// gengo-language after), so it survived that swap unchanged and gives an
/// exact before/after comparison instead of the misleadingly tiny corpora in
/// `bench_language_detection` above.
fn bench_language_detect_realistic(c: &mut Criterion) {
    let small_dir = setup_realistic_small_project();
    let medium_dir = setup_realistic_medium_project(750);
    let large_dir = setup_realistic_large_project(5000);

    c.bench_function("language_detect_realistic_small", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(small_dir.path()));
            black_box(langs)
        });
    });

    c.bench_function("language_detect_realistic_medium", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(medium_dir.path()));
            black_box(langs)
        });
    });

    // 5k+ files is the corpus that matters most for the `language_detection`
    // budget (tests/performance-baselines.json, target 100ms / max 200ms).
    // Named short deliberately: `scripts/bench.sh`'s `check_budget` greps
    // `^$bench_name ` requiring "time:" on the SAME line, and criterion
    // wraps the "time:" column to its own line once the benchmark id gets
    // much past ~22 characters -- verified empirically against this
    // criterion version (a longer name silently breaks the budget match,
    // since the wrapped "time:" line no longer starts with the bench name).
    c.bench_function("lang_detect_5k", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory(black_box(large_dir.path()));
            black_box(langs)
        });
    });

    // Isolates the cost of `detect_directory`'s per-file size lookup (see
    // `walk_directory` in `src/language/detector.rs`) at realistic scale --
    // the existing `_no_metadata` benches above only cover it on 2-3-file
    // corpora, where it is too cheap to show up.
    c.bench_function("lang_detect_5k_nometa", |b| {
        b.iter(|| {
            let langs = Detector::detect_directory_without_metadata(black_box(large_dir.path()));
            black_box(langs)
        });
    });
}

criterion_group!(
    benches,
    bench_language_detection,
    bench_language_with_file,
    bench_language_detect_realistic
);
criterion_main!(benches);
