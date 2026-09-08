//! Benchmarks for Git status operations
//!
//! Measures the performance of scanning real repositories.
//!
//! Run with: cargo bench --bench `git_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::status::load_repository_state;
use std::process::Command;
use tempfile::TempDir;

/// Set up a git repository for benchmarking
fn setup_git_repo(num_files: usize) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");
    let path = dir.path();

    Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "bench@test.com"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Bench"])
        .current_dir(path)
        .output()
        .unwrap();

    for i in 0..num_files {
        std::fs::write(path.join(format!("file_{i}.txt")), "content").unwrap();
    }

    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(path)
        .output()
        .unwrap();

    dir
}

fn bench_git_status(c: &mut Criterion) {
    let repo = setup_git_repo(100);
    let path = repo.path().to_str().unwrap();

    c.bench_function("git_status_cold", |b| {
        b.iter(|| {
            let result = load_repository_state(path, None, 0, true);
            std::hint::black_box(result).unwrap();
        });
    });
}

criterion_group!(benches, bench_git_status);
criterion_main!(benches);
