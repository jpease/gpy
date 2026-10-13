//! Benchmarks for Git status operations
//!
//! Measures the performance of scanning real repositories.
//!
//! Run with: cargo bench --bench `git_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

#[path = "support/git_repo.rs"]
mod git_repo;

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::status::load_repository_state;

fn bench_git_status(c: &mut Criterion) {
    let repo = git_repo::committed_repo(100);
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
