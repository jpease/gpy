//! IPC roundtrip latency: connect, send, receive. Every prompt render pays this.
//!
//! Run with: cargo bench --bench `ipc_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

#[path = "support/ipc.rs"]
mod ipc;

use criterion::{Criterion, criterion_group, criterion_main};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

fn spawn_load_generator(rt: &Runtime, socket_path: PathBuf) -> JoinHandle<()> {
    rt.spawn(async move {
        loop {
            let mut batch = Vec::new();
            for _ in 0_usize..8_usize {
                let socket = socket_path.clone();
                batch.push(tokio::spawn(async move {
                    let dir = tempfile::tempdir().expect("tempdir");
                    let escaped = dir.path().to_string_lossy().replace('\\', "\\\\");
                    let payload = format!(r#"{{"op":"lang","cwd":"{escaped}","format":"json"}}"#);
                    let mut stream = UnixStream::connect(&socket).await.expect("connect");
                    stream
                        .write_all(format!("{payload}\n").as_bytes())
                        .await
                        .expect("write");
                    let mut buf = [0_u8; 4096];
                    let n = stream.read(&mut buf).await.expect("read");
                    assert!(n > 0, "empty response");
                }));
            }
            for task in batch {
                task.await.expect("load generator task");
            }
        }
    })
}

fn report_tail_latency(rt: &Runtime, socket_path: PathBuf) {
    rt.block_on(async move {
        let mut samples = Vec::with_capacity(500);
        for _ in 0_usize..500_usize {
            let started = std::time::Instant::now();
            let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
            stream
                .write_all(b"{\"op\":\"ping\"}\n")
                .await
                .expect("write ping");
            let mut buf = [0_u8; 1024];
            let n = stream.read(&mut buf).await.expect("read pong");
            assert!(n > 0, "empty response");
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
    let rt = Runtime::new().expect("runtime");
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let socket_path = temp_dir.path().join("gpy.sock");
    ipc::start_and_wait(&rt, &socket_path);
    ipc::bench_ipc_roundtrip(c, &rt, &socket_path);
}

/// Ping p50/p99 latency while concurrent requests keep the agent busy;
/// cache-miss waits must not block worker threads.
fn bench_ipc_tail_latency_under_load(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let socket_path = temp_dir.path().join("gpy-tail.sock");

    ipc::start_and_wait(&rt, &socket_path);

    // Concurrent language-detect misses keep the blocking pool busy.
    let load_handle = spawn_load_generator(&rt, socket_path.clone());

    let mut group = c.benchmark_group("ipc_tail_latency");
    group.sample_size(200);
    group.bench_function("ping_under_load", |b| {
        b.to_async(&rt).iter(|| async {
            let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
            stream
                .write_all(b"{\"op\":\"ping\"}\n")
                .await
                .expect("write ping");
            let mut buf = [0_u8; 1024];
            let n = stream.read(&mut buf).await.expect("read pong");
            assert!(n > 0, "empty response");
        });
    });
    // Criterion reports the mean; the tail is what users feel.
    report_tail_latency(&rt, socket_path);

    load_handle.abort();

    group.finish();
}

criterion_group!(benches, bench_ipc_ping, bench_ipc_tail_latency_under_load);
criterion_main!(benches);
