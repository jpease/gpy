//! Small, fast benchmarks for the pre-push perf canary. Language detection is
//! budgeted separately via `./scripts/bench.sh --ci`.
//! Run with: cargo bench --bench `perf_canary_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

#[path = "support/git.rs"]
mod git;
#[path = "support/ipc.rs"]
mod ipc;

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::status::load_repository_state;
use std::hint::black_box;
use std::str::FromStr;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Read `name` from the environment, falling back to `default` when unset or unparsable.
fn env_or<T: FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn canary_config() -> Criterion {
    // Early-warning canary: a smaller sample window than the full suite.
    Criterion::default()
        .sample_size(env_or("GPY_PERF_CANARY_SAMPLE_SIZE", 20))
        .warm_up_time(Duration::from_secs(env_or(
            "GPY_PERF_CANARY_WARMUP_SECONDS",
            1,
        )))
        .measurement_time(Duration::from_secs(env_or(
            "GPY_PERF_CANARY_MEASUREMENT_SECONDS",
            1,
        )))
}

fn bench_perf_canary(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = tempfile::Builder::new()
        .prefix("gpy-perf-canary-ipc")
        .tempdir()
        .expect("tempdir");
    let socket_path = temp_dir.path().join("gpy.sock");
    ipc::start_and_wait(&rt, &socket_path);
    let repo = git::committed_repo(100);
    let repo_path = repo.path().to_str().expect("repo path to str");

    ipc::bench_ipc_roundtrip(c, &rt, &socket_path);
    c.bench_function("git_status_cold", |b| {
        b.iter(|| {
            let result = load_repository_state(repo_path, None, 0, true);
            black_box(result).expect("load repository state");
        });
    });
}

criterion_group! {
    name = benches;
    config = canary_config();
    targets = bench_perf_canary
}
criterion_main!(benches);
