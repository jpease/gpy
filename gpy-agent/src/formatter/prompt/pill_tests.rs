#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "test code: failures should panic"
)]

use super::*;
use crate::config::Config;
use crate::config::types::ColorSpec;
use crate::formatter::SegmentPosition;
use crate::template::Color;
use crate::theme::ThemeConfig;

fn pill(name: &str, version: &str, color: &str) -> LanguageInfo {
    LanguageInfo {
        name: name.to_owned(),
        version: Some(version.to_owned()),
        color: ColorSpec::new(color).unwrap(),
    }
}

/// Pill N's `prev_bg` is pill N-1's background, not the segment before the language segment.
#[test]
fn language_pills_chain_prev_bg() {
    let formatter = PromptFormatter::default();
    let (config, mut theme) = (Config::default(), ThemeConfig::default());
    theme.segments.language.format = Some("[$version](fg:prev_bg)".to_owned());
    let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST).with_prev_colors(
        None,
        Some(Color::Rgb {
            r: 10,
            g: 20,
            b: 30,
        }),
    );
    let response = Response::Language {
        languages: vec![
            pill("Rust", "1.0.0", "#111111"),
            pill("Go", "2.0.0", "#222222"),
            pill("Python", "3.0.0", "#333333"),
        ],
    };
    let got = formatter.render(&response, &rc).expect("render");
    assert!(got.contains("38;2;10;20;30m1.0.0"), "{got:?}");
    assert!(got.contains("38;2;17;17;17m2.0.0"), "{got:?}");
    assert!(got.contains("38;2;34;34;34m3.0.0"), "{got:?}");
}
