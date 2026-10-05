//! Benchmarks for formatter hot paths
//!
//! These benchmarks measure the performance of the formatter code paths that are
//! executed on every prompt render. The goal is to ensure that formatting remains
//! fast (<5ms) even with complex git states and long branch names.
//!
//! Run with: cargo bench --bench `formatter_bench`

#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::config::Config;
use gpy_agent::formatter::{
    FishAnsiFormatter, FishSourceFormatter, Formatter, RenderContext, SegmentPosition,
};
use gpy_agent::ipc::{LanguageInfo, Response};
use gpy_agent::theme::ThemeConfig;
use std::hint::black_box;

#[allow(clippy::too_many_lines)]
fn bench_git_status_rendering(c: &mut Criterion) {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let formatter = FishAnsiFormatter::default();

    c.bench_function("git_status_ansi_short_branch", |b| {
        let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 1,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });

    c.bench_function("git_status_ansi_long_branch", |b| {
        let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch:
                "feature/implement-advanced-kubernetes-integration-with-monitoring-and-alerting"
                    .to_owned(),
            ahead: 2,
            behind: 1,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });

    c.bench_function("git_status_ansi_truncated_branch", |b| {
        let mut config_truncated = Config::default();
        config_truncated.git.max_branch_length =
            gpy_agent::config::types::MaxBranchLength::new(30).expect("Valid length");

        let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch:
                "feature/implement-advanced-kubernetes-integration-with-monitoring-and-alerting"
                    .to_owned(),
            ahead: 2,
            behind: 1,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });
        let ctx = RenderContext::new(&config_truncated, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });
}

fn bench_fish_source_rendering(c: &mut Criterion) {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let formatter = FishSourceFormatter;

    c.bench_function("git_status_source_short_branch", |b| {
        let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 1,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });
}

fn bench_language_rendering(c: &mut Criterion) {
    let config = Config::default();
    let theme = ThemeConfig::default();
    let formatter = FishAnsiFormatter::default();

    c.bench_function("language_ansi_single", |b| {
        let response = Response::Language {
            languages: vec![LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.75.0".to_owned()),
                color: gpy_agent::config::types::ColorSpec::new("#dea584").expect("Valid color"),
            }],
        };
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });

    c.bench_function("language_ansi_multiple", |b| {
        let response = Response::Language {
            languages: vec![
                LanguageInfo {
                    name: "Rust".to_owned(),
                    version: Some("1.75.0".to_owned()),
                    color: gpy_agent::config::types::ColorSpec::new("#dea584")
                        .expect("Valid color"),
                },
                LanguageInfo {
                    name: "Python".to_owned(),
                    version: Some("3.11.0".to_owned()),
                    color: gpy_agent::config::types::ColorSpec::new("#3776ab")
                        .expect("Valid color"),
                },
                LanguageInfo {
                    name: "Node".to_owned(),
                    version: Some("20.0.0".to_owned()),
                    color: gpy_agent::config::types::ColorSpec::new("#68a063")
                        .expect("Valid color"),
                },
            ],
        };
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        b.iter(|| {
            let result = formatter.render(black_box(&response), black_box(&ctx));
            black_box(result)
        });
    });
}

criterion_group!(
    benches,
    bench_git_status_rendering,
    bench_fish_source_rendering,
    bench_language_rendering
);
criterion_main!(benches);
