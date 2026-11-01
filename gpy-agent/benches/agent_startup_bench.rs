//! Cold-start benches. `Agent::new(socket)` binds a socket and registers watchers, so
//! its dominant sub-steps (config load, theme load, theme export) and the
//! version-command spawn path are benched individually.
//!
//! Run with: cargo bench --bench `agent_startup_bench`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::cache::theme_export::write_theme_export_to_dir;
use gpy_agent::config::Config;
use gpy_agent::config::loader::load_config_from_file;
use gpy_agent::language::version::{VersionSource, execute_version_command};
use gpy_agent::template::Palette;
use gpy_agent::theme::ThemeManager;
use std::fs;
use std::hint::black_box;
use tempfile::TempDir;

/// Fully populated config mirroring `config/config.example.toml`, not the cheap
/// empty-file default.
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

/// `load_config_from_file` against a tempdir fixture.
fn bench_config_load(c: &mut Criterion) {
    let dir = TempDir::new().expect("create temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(&config_path, CONFIG_FIXTURE_TOML).expect("write fixture config");

    // Confirm the fixture parses to its values, not defaults, so this isn't a
    // no-op parse.
    let parsed = load_config_from_file(&config_path).expect("parse fixture config");
    assert_eq!(
        parsed.git.timeout_seconds.get(),
        45,
        "fixture should override the default git.timeout_seconds (10)"
    );

    c.bench_function("config_load_from_file", |b| {
        b.iter(|| {
            let config = load_config_from_file(black_box(&config_path)).expect("parse config");
            black_box(config);
        });
    });
}

/// `ThemeManager::new` as `Agent::init_theme_manager` calls it. Not hermetic:
/// it consults the host `~/.config` override path, like the real code path.
fn bench_theme_manager_load(c: &mut Criterion) {
    c.bench_function("theme_manager_load", |b| {
        b.iter(|| {
            let theme_manager = ThemeManager::new(black_box("default")).expect("load theme");
            black_box(theme_manager);
        });
    });
}

/// `write_theme_export_cache` via the tempdir seam; after the first iteration
/// the export is unchanged, so this measures render + compare, no write.
fn bench_theme_export(c: &mut Criterion) {
    let theme_manager = ThemeManager::new("default").expect("load theme");
    let config = Config::default();
    let palette = Palette::default();
    let dir = TempDir::new().expect("create temp dir");
    let cache_dir = dir.path().join("gpy");

    c.bench_function("theme_export_write", |b| {
        b.iter(|| {
            write_theme_export_to_dir(
                black_box(&cache_dir),
                black_box(&theme_manager),
                black_box(&config),
                black_box(&palette),
            )
            .expect("write theme export");
        });
    });
}

/// Near-instant no-op command, so only our spawn/parse overhead is measured.
struct FastStubDetector;

impl VersionSource for FastStubDetector {
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

/// `execute_version_command`'s spawn, process-group, reader thread, timeout, and parse.
/// No cache at this layer (it lives in `detect_version_at`), and
/// `cwd: None` skips the mise probe.
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
