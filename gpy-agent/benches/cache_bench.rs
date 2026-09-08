//! Benchmarks for cache operations
//!
//! These benchmarks measure the performance of cache read/write operations
//! which are critical for responsiveness.
//!
//! Run with: cargo bench --bench `cache_bench`

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use std::collections::HashMap;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn sample_status() -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
}

fn bench_cache_write(c: &mut Criterion) {
    let cache = Arc::new(GitStatusCache::new());
    let repo_path = PathBuf::from("/tmp/test-repo");
    let repo_status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    c.bench_function("cache_write_new_entry", |b| {
        b.iter(|| {
            cache.set(
                black_box(&repo_path),
                black_box(repo_status.clone()),
                HashMap::new(),
            );
        });
    });
}

fn bench_cache_read(c: &mut Criterion) {
    let cache = Arc::new(GitStatusCache::new());
    let repo_path = PathBuf::from("/tmp/test-repo");
    let repo_status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    // Pre-populate cache
    cache.set(&repo_path, repo_status, HashMap::new());

    c.bench_function("cache_read_hit", |b| {
        b.iter(|| {
            let result = cache.get(black_box(&repo_path));
            black_box(result)
        });
    });

    c.bench_function("cache_read_miss", |b| {
        let missing_path = PathBuf::from("/tmp/nonexistent-repo");
        b.iter(|| {
            let result = cache.get(black_box(&missing_path));
            black_box(result)
        });
    });
}

fn bench_cache_invalidate(c: &mut Criterion) {
    let cache = Arc::new(GitStatusCache::new());
    let repo_path = PathBuf::from("/tmp/test-repo");
    let repo_status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    c.bench_function("cache_invalidate_single", |b| {
        b.iter(|| {
            cache.set(&repo_path, repo_status.clone(), HashMap::new());
            cache.invalidate(black_box(&repo_path));
        });
    });
}

/// Hot-path comparison on a *real* canonical directory (issue #143).
///
/// Both benches read an identical cache hit keyed by a real, already-canonical
/// path. `cache_read_hit_canonicalizing` pays the redundant `fs::canonicalize`
/// syscall that the warm request path used to incur; `cache_read_hit_fast_path`
/// uses `get_canonical` and skips it. The delta is the overhead this change
/// removes, and acts as the regression guard for the warm path.
fn bench_cache_read_canonical_hotpath(c: &mut Criterion) {
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let Ok(canonical_path) = temp_dir.path().canonicalize() else {
        return;
    };

    let cache = Arc::new(GitStatusCache::new());
    cache.set_canonical(&canonical_path, sample_status(), HashMap::new());

    c.bench_function("cache_read_hit_canonicalizing", |b| {
        b.iter(|| {
            let result = cache.get(black_box(&canonical_path));
            black_box(result)
        });
    });

    c.bench_function("cache_read_hit_fast_path", |b| {
        b.iter(|| {
            let result = cache.get_canonical(black_box(&canonical_path));
            black_box(result)
        });
    });
}

criterion_group!(
    benches,
    bench_cache_write,
    bench_cache_read,
    bench_cache_invalidate,
    bench_cache_read_canonical_hotpath
);
criterion_main!(benches);
