//! Shared IPC benchmark fixtures: server setup and ping measurement.

#![allow(
    clippy::expect_used,
    reason = "benchmark fixture helpers panic on setup failure"
)]
#![allow(clippy::missing_panics_doc, reason = "benchmark fixture helpers")]

use criterion::Criterion;
use gpy_agent::ipc::server::{EndpointDeps, EndpointHandle};
use gpy_agent::security::GuardSettings;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Runtime;

/// Build an IPC endpoint handle bound to `socket_path` with high connection caps for benchmarks.
#[must_use]
pub fn build_ipc_server(socket_path: &Path) -> EndpointHandle {
    let security_config = GuardSettings {
        max_connections_per_second: 100_000,
        max_concurrent_connections: 1000,
        ..Default::default()
    };
    EndpointHandle::builder(EndpointDeps {
        client_registry: Arc::new(gpy_agent::ipc::ClientDirectory::new()),
        git_cache: Arc::new(gpy_agent::git::cache::GitStatusCache::new()),
        config_manager: Arc::new(
            gpy_agent::config::manager::ConfigManager::with_defaults()
                .expect("default config manager"),
        ),
        theme_manager: Arc::new(
            gpy_agent::theme::ThemeManager::new("default").expect("default theme manager"),
        ),
        instant_cache: Arc::new(
            gpy_agent::cache::InstantPromptCache::new().expect("instant prompt cache"),
        ),
        language_cache: gpy_agent::language::DetectionCache::new(),
    })
    .socket_path(socket_path.to_path_buf())
    .security_config(security_config)
    .build()
    .expect("build endpoint handle")
}

/// Poll `socket_path` until a Unix stream connection succeeds.
pub fn wait_for_socket(rt: &Runtime, socket_path: &Path) {
    rt.block_on(async {
        while tokio::net::UnixStream::connect(socket_path).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

/// Start an IPC endpoint on a background task and wait until the socket accepts connections.
pub fn start_and_wait(rt: &Runtime, socket_path: &Path) {
    let mut server = build_ipc_server(socket_path);
    rt.spawn(async move {
        server.start().await.expect("server start");
    });
    wait_for_socket(rt, socket_path);
}

/// Measure single-connection ping round-trip latency against `socket_path`.
pub fn bench_ipc_roundtrip(c: &mut Criterion, rt: &Runtime, socket_path: &Path) {
    c.bench_function("ipc_roundtrip", |b| {
        b.to_async(rt).iter(|| async {
            let mut stream = UnixStream::connect(socket_path).await.expect("connect");
            stream
                .write_all(b"{\"op\":\"ping\"}\n")
                .await
                .expect("write ping");
            let mut buf = [0_u8; 1024];
            let n = stream.read(&mut buf).await.expect("read pong");
            assert!(n > 0, "empty response");
        });
    });
}
