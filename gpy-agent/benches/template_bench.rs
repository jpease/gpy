//! Benchmarks for the template engine render hot path.
//!
//! Measures `render()` cost on the Starship default module formats that appear
//! on every prompt. Baseline data for deciding whether compiled-AST caching
//! is worth the complexity (see issue #188).
//!
//! Run with: `cargo bench --bench template_bench`

#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use criterion::{Criterion, criterion_group, criterion_main};
use gpy_agent::template::{MapResolver, RenderContext, render};
use std::hint::black_box;

fn bench_git_segment(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("git_branch", |b| {
        let fmt = "on [$symbol$branch(:$remote_branch)]($style) ";
        let resolver = MapResolver::from_pairs([
            ("symbol", " "),
            ("branch", "main"),
            ("style", "bold purple"),
        ]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

fn bench_git_status(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("git_status_dirty", |b| {
        let fmt = r"([\[$all_status$ahead_behind\]]($style) )";
        let resolver = MapResolver::from_pairs([("all_status", "!?"), ("style", "red bold")]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.bench_function("git_status_clean", |b| {
        let fmt = r"([\[$all_status$ahead_behind\]]($style) )";
        let resolver = MapResolver::default();
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

fn bench_character_segment(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("character", |b| {
        let fmt = "[❯](bold green)";
        let resolver = MapResolver::default();
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

fn bench_language_segment(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("language_with_version", |b| {
        let fmt = "via [$symbol($version )]($style)";
        let resolver = MapResolver::from_pairs([
            ("symbol", "🦀 "),
            ("version", "v1.75.0"),
            ("style", "bold #dea584"),
        ]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.bench_function("language_no_version", |b| {
        let fmt = "via [$symbol($version )]($style)";
        let resolver = MapResolver::from_pairs([("symbol", "🦀 "), ("style", "bold #dea584")]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

fn bench_duration_segment(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("cmd_duration", |b| {
        let fmt = "took [$duration]($style) ";
        let resolver = MapResolver::from_pairs([("duration", "3s"), ("style", "yellow bold")]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

fn bench_full_prompt(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");

    group.bench_function("full_prompt", |b| {
        let fmt = concat!(
            "on [$symbol$branch(:$remote_branch)]($style) ",
            r"([\[$all_status$ahead_behind\]]($style) )",
            "via [$symbol($version )]($style)",
            "took [$duration]($style) ",
            "[❯](bold green)",
        );
        let resolver = MapResolver::from_pairs([
            ("symbol", " "),
            ("branch", "main"),
            ("style", "bold purple"),
            ("all_status", "!"),
            ("version", "v1.75.0"),
            ("duration", "1s"),
        ]);
        let ctx = RenderContext::new(&resolver);
        b.iter(|| render(black_box(fmt), black_box(&ctx)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_git_segment,
    bench_git_status,
    bench_character_segment,
    bench_language_segment,
    bench_duration_segment,
    bench_full_prompt,
);
criterion_main!(benches);
