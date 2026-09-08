//! Focused performance canary benchmarks for hook-time comparisons.
//!
//! These are intentionally smaller than the full benchmark suite so a pre-push
//! hook can compare the current tree to the last pushed commit on the same
//! machine without turning every push into a long benchmark session.
//!
//! Run with: cargo bench --bench `perf_canary_bench`
//!
//! Previously also carried `language_detect_rust/python/node`, dropped in
//! #537: their 2-3-file synthetic corpus measured as unstable diagnostic
//! noise (a 23% baseline swing between runs with no code change), and the
//! `language_detection` budget in `tests/performance-baselines.json` already
//! gates language detection properly, against a realistic-sized corpus, via
//! `./scripts/bench.sh --ci`. See `scripts/perf-canary.sh`'s `BENCHMARKS`
//! array for the corresponding removal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::status::load_repository_state;
use gpy_agent::ipc::server::EndpointHandle;
use gpy_agent::security::GuardSettings;
use std::fs;
use std::hint::black_box;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Runtime;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn canary_config() -> Criterion {
    // The pre-push canary is an early warning diagnostic, so we intentionally
    // use a smaller sample window than the full benchmark workflow.
    Criterion::default()
        .sample_size(env_usize("GPY_PERF_CANARY_SAMPLE_SIZE", 20))
        .warm_up_time(Duration::from_secs(env_u64(
            "GPY_PERF_CANARY_WARMUP_SECONDS",
            1,
        )))
        .measurement_time(Duration::from_secs(env_u64(
            "GPY_PERF_CANARY_MEASUREMENT_SECONDS",
            1,
        )))
}

fn setup_git_repo(num_files: usize) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("gpy-perf-canary-git")
        .tempdir_in("/tmp")
        .expect("failed to create temp dir");
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
        fs::write(path.join(format!("file_{i}.txt")), "content").unwrap();
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

fn build_ipc_server(socket_path: &std::path::Path) -> EndpointHandle {
    let security_config = GuardSettings {
        max_connections_per_second: 100_000,
        max_concurrent_connections: 1000,
        ..Default::default()
    };

    EndpointHandle::builder()
        .socket_path(socket_path.to_path_buf())
        .client_registry(Arc::new(gpy_agent::ipc::ClientDirectory::new()))
        .git_cache(Arc::new(gpy_agent::git::cache::GitStatusCache::new()))
        .config_manager(Arc::new(
            gpy_agent::config::manager::ConfigManager::with_defaults().unwrap(),
        ))
        .theme_manager(Arc::new(
            gpy_agent::theme::ThemeManager::new("default").unwrap(),
        ))
        .instant_cache(Arc::new(
            gpy_agent::cache::InstantPromptCache::new().unwrap(),
        ))
        .latency_tracker(Arc::new(gpy_agent::ipc::LatencyTracker::new(100)))
        .language_cache(gpy_agent::language::DetectionCache::new())
        .security_config(security_config)
        .build()
        .unwrap()
}

fn wait_for_socket(rt: &Runtime, socket_path: &std::path::Path) {
    rt.block_on(async {
        while tokio::net::UnixStream::connect(socket_path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

fn setup_ipc_server(rt: &Runtime) -> std::path::PathBuf {
    let temp_dir = tempfile::Builder::new()
        .prefix("gpy-perf-canary-ipc")
        .tempdir_in("/tmp")
        .unwrap();
    let socket_path = temp_dir.path().join("gpy.sock");
    let mut server = build_ipc_server(&socket_path);

    let socket_path_clone = socket_path.clone();
    rt.spawn(async move {
        let _temp_dir = temp_dir;
        server.start().await.unwrap();
    });

    wait_for_socket(rt, &socket_path_clone);

    socket_path
}

fn bench_ipc_roundtrip(c: &mut Criterion, rt: &Runtime, socket_path: &std::path::Path) {
    c.bench_function("ipc_roundtrip", |b| {
        b.to_async(rt).iter(|| async {
            let mut stream = UnixStream::connect(socket_path).await.unwrap();
            stream.write_all(b"{\"op\":\"ping\"}\n").await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
        });
    });
}

fn bench_perf_canary(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let socket_path = setup_ipc_server(&rt);
    let repo = setup_git_repo(100);
    let repo_path = repo.path().to_str().unwrap();

    bench_ipc_roundtrip(c, &rt, &socket_path);

    c.bench_function("git_status_cold", |b| {
        b.iter(|| {
            let result = load_repository_state(repo_path, None, 0, true);
            black_box(result).unwrap();
        });
    });
}

criterion_group! {
    name = benches;
    config = canary_config();
    targets = bench_perf_canary
}
criterion_main!(benches);
