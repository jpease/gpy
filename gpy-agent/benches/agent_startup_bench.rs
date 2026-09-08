//! Benchmarks for GPY agent cold-start hot paths (issue #335).
//!
//! `Agent::new()` (`src/agent/mod.rs`) registers global file watchers and binds
//! a Unix socket, which makes benchmarking the full constructor impractical and
//! non-hermetic in a Criterion harness. Per the issue, this file instead
//! benches its dominant sub-steps individually, plus the version-detection
//! subprocess path that also runs on the first prompt render (via language
//! detection) but is not itself part of `Agent::new`:
//!
//! - `config_load_from_file` - the config-parsing step `ConfigManager::new`
//!   performs, run against a representative fixture in a tempdir.
//! - `theme_manager_load` - `Agent::init_theme_manager`'s `ThemeManager::new`
//!   call.
//! - `theme_export_write` - `write_theme_export_cache`, benched through the
//!   `write_theme_export_to_dir` seam (an explicit-directory inner function)
//!   so it can target a tempdir instead of the real `XDG_CACHE_HOME`/`HOME`.
//! - `version_command_spawn` - the spawn/read/timeout/parse machinery in
//!   `execute_version_command`, using a stub `ReleaseSource` so the bench
//!   measures our overhead rather than a real toolchain's (node, python, ...)
//!   own startup cost.
//!
//! Run with: cargo bench --bench `agent_startup_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::cache::theme_export::write_theme_export_to_dir;
use gpy_agent::config::Config;
use gpy_agent::config::loader::load_config_from_file;
use gpy_agent::language::version::{ReleaseSource, execute_version_command};
use gpy_agent::theme::ThemeManager;
use std::fs;
use std::hint::black_box;
use tempfile::TempDir;

/// Representative `config.toml` fixture.
///
/// Mirrors the shape of `config/config.example.toml` so the parse-cost bench
/// reflects a realistic, fully-populated user config rather than the much
/// cheaper empty-file default path.
const CONFIG_FIXTURE_TOML: &str = r#"
[ui]
show_icons = true
theme = "default"

[ui.directory]
display = "basename"
truncation_length = 3
truncation_symbol = ""
max_length = 80

[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 30
max_restart_attempts = 5

[git]
enabled = true
show_upstream = true
timeout_seconds = 45
skip_paths = []
max_branch_length = 0

[git.icons]
ahead = "A"
behind = "B"
staged = "S"
unstaged = "U"
untracked = "?"
conflicts = "X"

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"

[language.icons]
rust = "R"
python = "P"
node = "N"
go = "G"
java = "J"
ruby = "Y"
swift = "W"
elixir = "E"
php = "H"
csharp = "S"
cpp = "PP"
c = "C"
erlang = "L"
fish = "F"
"#;

/// Bench: config parsing cost of `ConfigManager::new`'s underlying
/// `load_config_from_file` call, against a fixture written to a tempdir (never
/// the real `XDG_CONFIG_HOME`/`HOME`).
fn bench_config_load(c: &mut Criterion) {
    let dir = TempDir::new().expect("create temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(&config_path, CONFIG_FIXTURE_TOML).expect("write fixture config");
    let path_str = config_path.to_str().expect("path is valid utf8");

    // Confirm the fixture really parses into the values it sets, rather than
    // silently falling back to defaults, so a benchmark number cannot be a
    // no-op parse in disguise.
    let parsed = load_config_from_file(path_str).expect("parse fixture config");
    assert_eq!(
        parsed.git.timeout_seconds.get(),
        45,
        "fixture should override the default git.timeout_seconds (10)"
    );

    c.bench_function("config_load_from_file", |b| {
        b.iter(|| {
            let config = load_config_from_file(black_box(path_str)).expect("parse config");
            black_box(config);
        });
    });
}

/// Bench: `ThemeManager::new`, the exact call `Agent::init_theme_manager`
/// makes for the configured theme name (default: `"default"`).
///
/// Unlike the other benches here, this is not hermetic to a tempdir: theme
/// resolution consults the host's `~/.config` override path before falling
/// back to embedded builtin content, matching the existing
/// `perf_canary_bench` precedent for the same call. That is intentional --
/// this canary exists to measure the real `Agent::new` code path, not an
/// idealized one.
fn bench_theme_manager_load(c: &mut Criterion) {
    c.bench_function("theme_manager_load", |b| {
        b.iter(|| {
            let theme_manager = ThemeManager::new(black_box("default")).expect("load theme");
            black_box(theme_manager);
        });
    });
}

/// Bench: `write_theme_export_cache`'s real rendering + atomic-write work,
/// via the `write_theme_export_to_dir` seam so the target directory is a
/// tempdir rather than the real cache directory.
fn bench_theme_export(c: &mut Criterion) {
    let theme_manager = ThemeManager::new("default").expect("load theme");
    let config = Config::default();
    let dir = TempDir::new().expect("create temp dir");
    let cache_dir = dir.path().join("gpy");

    c.bench_function("theme_export_write", |b| {
        b.iter(|| {
            write_theme_export_to_dir(
                black_box(&cache_dir),
                black_box(&theme_manager),
                black_box(&config),
            )
            .expect("write theme export");
        });
    });
}

/// Stub `ReleaseSource` whose command is a near-instant no-op process.
///
/// So the bench measures spawn/reader-thread/timeout/parse overhead rather
/// than a real toolchain's (node, python, ...) own startup cost.
struct FastStubDetector;

impl ReleaseSource for FastStubDetector {
    fn language_name(&self) -> &'static str {
        "bench-stub"
    }

    fn version_command(&self) -> &[&str] {
        #[cfg(unix)]
        {
            &["true"]
        }
        #[cfg(not(unix))]
        {
            &["cmd", "/C", "exit", "0"]
        }
    }

    fn parse_version(&self, output: &str) -> Option<String> {
        Some(output.trim().to_owned())
    }
}

/// Bench: `execute_version_command`'s spawn + own-process-group placement +
/// dedicated reader thread + timeout wait + output parsing.
///
/// Uses [`FastStubDetector`] so no real toolchain (node, python, ...) is ever
/// shelled out to.
///
/// `execute_version_command` has no cache of its own -- the 24h cache lives
/// one layer up, in `detect_language_release_at` -- so calling it directly
/// here guarantees a real subprocess spawn on every iteration, never a cache
/// hit. Passing `cwd: None` also skips the mise probe (which only runs when a
/// directory is supplied), keeping the measured path to exactly spawn + wait,
/// not project-file discovery.
fn bench_version_command_spawn(c: &mut Criterion) {
    let detector = FastStubDetector;

    let result = execute_version_command(&detector, None).expect("spawn stub command");
    assert_eq!(
        result.as_deref(),
        Some(""),
        "stub command produces empty stdout, confirming it ran rather than erroring"
    );

    c.bench_function("version_command_spawn", |b| {
        b.iter(|| {
            let version = execute_version_command(black_box(&detector), black_box(None))
                .expect("spawn stub command");
            black_box(version);
        });
    });
}

criterion_group!(
    benches,
    bench_config_load,
    bench_theme_manager_load,
    bench_theme_export,
    bench_version_command_spawn
);
criterion_main!(benches);
