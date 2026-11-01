//! Plugin discovery cost as the user plugin count grows (0, 10 and 50).
//!
//! Run with: cargo bench --bench `plugin_bench`

#![allow(
    clippy::expect_used,
    reason = "bench setup fails loudly on a broken fixture"
)]

#[path = "../tests/common/env_guard.rs"]
mod env_guard;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use env_guard::EnvGuard;
use gpy_agent::plugin::discover_plugins;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use tempfile::TempDir;

/// The first-party metadata plugin every discovery includes.
const FIRST_PARTY_PLUGINS: usize = 1;

/// Write `count` minimal file plugins under `root`.
fn write_plugins(root: &Path, count: usize) {
    for i in 0..count {
        let id = format!("demo-{i:02}");
        let plugin_dir = root.join(&id);
        fs::create_dir_all(plugin_dir.join("segments")).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
                 entry_type = \"file\"\nprovided_segments = [\"seg_{i}\"]\n"
            ),
        )
        .expect("write manifest");
    }
}

fn bench_discovery(c: &mut Criterion) {
    let mut group = c.benchmark_group("plugin_discovery");
    for count in [0_usize, 10, 50] {
        let xdg = TempDir::new().expect("xdg tempdir");
        let plugins_root = xdg.path().join("gpy").join("plugins");
        fs::create_dir_all(&plugins_root).expect("create plugins root");
        write_plugins(&plugins_root, count);

        // Criterion runs every benchmark on this thread, so nothing else reads
        // the environment while the guard changes it.
        let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", xdg.path());
        env.set("XDG_DATA_HOME", xdg.path());
        env.remove("GPY_BUNDLED_PLUGIN_DIR");
        assert_eq!(
            discover_plugins().plugins.len(),
            count.saturating_add(FIRST_PARTY_PLUGINS),
            "discovery must find every user plugin plus the first-party one"
        );
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, _| {
            b.iter(|| black_box(discover_plugins()));
        });
        drop(env);
    }
    group.finish();
}

criterion_group!(benches, bench_discovery);
criterion_main!(benches);
