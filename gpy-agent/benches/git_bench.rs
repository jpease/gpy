//! Git status benchmarks on real repositories. Run: cargo bench --bench `git_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

#[path = "support/git.rs"]
mod git;

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::status::load_repository_state;

fn bench_git_status(c: &mut Criterion) {
    let repo = git::committed_repo(100);
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
