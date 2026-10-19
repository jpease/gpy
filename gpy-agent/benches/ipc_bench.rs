//! IPC roundtrip latency: connect, send, receive. Every prompt render pays this.
//!
//! Run with: cargo bench --bench `ipc_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::ipc::server::{EndpointDeps, EndpointHandle};
use gpy_agent::security::GuardSettings;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

fn build_server(socket_path: PathBuf) -> EndpointHandle {
    let security_config = GuardSettings {
        max_connections_per_second: 100_000,
        max_concurrent_connections: 1000,
        ..Default::default()
    };
    EndpointHandle::builder(EndpointDeps {
        client_registry: Arc::new(gpy_agent::ipc::ClientDirectory::new()),
        git_cache: Arc::new(gpy_agent::git::cache::GitStatusCache::new()),
        config_manager: Arc::new(
            gpy_agent::config::manager::ConfigManager::with_defaults().unwrap(),
        ),
        theme_manager: Arc::new(gpy_agent::theme::ThemeManager::new("default").unwrap()),
        instant_cache: Arc::new(gpy_agent::cache::InstantPromptCache::new().unwrap()),
        language_cache: gpy_agent::language::DetectionCache::new(),
    })
    .socket_path(socket_path)
    .security_config(security_config)
    .build()
    .unwrap()
}

fn start_and_wait(rt: &Runtime, socket_path: PathBuf) {
    let mut server = build_server(socket_path.clone());
    rt.spawn(async move {
        server.start().await.unwrap();
    });
    rt.block_on(async move {
        while tokio::net::UnixStream::connect(&socket_path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

fn spawn_load_generator(rt: &Runtime, socket_path: PathBuf) -> JoinHandle<()> {
    rt.spawn(async move {
        loop {
            let mut batch = Vec::new();
            for _ in 0_usize..8_usize {
                let socket = socket_path.clone();
                batch.push(tokio::spawn(async move {
                    let dir = tempfile::tempdir().ok()?;
                    let escaped = dir.path().to_string_lossy().replace('\\', "\\\\");
                    let payload = format!(r#"{{"op":"lang","cwd":"{escaped}","format":"json"}}"#);
                    let mut stream = UnixStream::connect(&socket).await.ok()?;
                    stream
                        .write_all(format!("{payload}\n").as_bytes())
                        .await
                        .ok()?;
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf).await.ok()?;
                    Some(())
                }));
            }
            for task in batch {
                let _ = task.await;
            }
        }
    })
}

fn report_tail_latency(rt: &Runtime, socket_path: PathBuf) {
    rt.block_on(async move {
        let mut samples = Vec::with_capacity(500);
        for _ in 0_usize..500_usize {
            let started = std::time::Instant::now();
            let mut stream = UnixStream::connect(&socket_path).await.unwrap();
            stream.write_all(b"{\"op\":\"ping\"}\n").await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let last_index = samples.len().saturating_sub(1);
        // floor(p * (n - 1) / 100): avoids float `as` casts denied by clippy-strict.
        let pct = |p: usize| -> std::time::Duration {
            let idx = (p.saturating_mul(last_index) / 100).min(last_index);
            samples.get(idx).copied().unwrap_or_default()
        };
        #[allow(clippy::print_stderr)]
        {
            eprintln!(
                "ping-under-load latency: p50={}us p90={}us p99={}us max={}us",
                pct(50).as_micros(),
                pct(90).as_micros(),
                pct(99).as_micros(),
                samples.last().copied().unwrap_or_default().as_micros(),
            );
        }
    });
}

fn bench_ipc_ping(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("gpy.sock");

    start_and_wait(&rt, socket_path.clone());

    c.bench_function("ipc_roundtrip", |b| {
        b.to_async(&rt).iter(|| async {
            let mut stream = UnixStream::connect(&socket_path).await.unwrap();
            stream.write_all(b"{\"op\":\"ping\"}\n").await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
        });
    });
}

/// Ping p50/p99 latency while concurrent requests keep the agent busy;
/// cache-miss waits must not block worker threads.
fn bench_ipc_tail_latency_under_load(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("gpy-tail.sock");

    start_and_wait(&rt, socket_path.clone());

    // Concurrent language-detect misses keep the blocking pool busy.
    let load_handle = spawn_load_generator(&rt, socket_path.clone());

    let mut group = c.benchmark_group("ipc_tail_latency");
    group.sample_size(200);
    group.bench_function("ping_under_load", |b| {
        b.to_async(&rt).iter(|| async {
            let mut stream = UnixStream::connect(&socket_path).await.unwrap();
            stream.write_all(b"{\"op\":\"ping\"}\n").await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
        });
    });
    // Criterion reports the mean; the tail is what users feel.
    report_tail_latency(&rt, socket_path);

    load_handle.abort();

    group.finish();
}

criterion_group!(benches, bench_ipc_ping, bench_ipc_tail_latency_under_load);
criterion_main!(benches);
