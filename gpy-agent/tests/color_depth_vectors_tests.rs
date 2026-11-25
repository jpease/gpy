//! The colour-depth downgrade must match the shared vectors
//! (`tests/fixtures/color_depth_vectors.tsv`): a 24-bit colour as the encoder
//! writes it, fitted to each depth by `downgrade_sgr`, the one implementation
//! `gpy prompt`, `gpy-agent oneshot` and the wizard's preview share.

use gpy_agent::formatter::{ColorDepth, PromptDialect, downgrade_sgr};
use gpy_agent::template::{Color, Span, SpanKind, Style};
use std::path::Path;

/// `x` in the 24-bit colour `rgb`, as the encoder writes it, fitted to `depth`.
fn fitted(rgb: [u8; 3], background: bool, depth: ColorDepth) -> String {
    let [red, green, blue] = rgb;
    let color = Some(Color::Rgb {
        r: red,
        g: green,
        b: blue,
    });
    let style = if background {
        Style {
            fg: None,
            bg: color,
            attrs: Vec::new(),
        }
    } else {
        Style {
            fg: color,
            bg: None,
            attrs: Vec::new(),
        }
    };
    let span = Span {
        text: "x".to_owned(),
        style,
        kind: SpanKind::Text,
    };
    downgrade_sgr(&PromptDialect::Ansi.encode(&[span]), depth).into_owned()
}

/// What `x` looks like with SGR parameter `param`; with none, only the reset
/// that ends the span is left.
fn expected(param: &str) -> String {
    if param.is_empty() {
        "x\x1b[0m".to_owned()
    } else {
        format!("\x1b[{param}mx\x1b[0m")
    }
}

#[test]
fn rgb_downgrade_matches_shared_vectors() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/color_depth_vectors.tsv");
    let text = std::fs::read_to_string(&path).expect("read color_depth_vectors.tsv");
    let mut rows = 0_usize;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols: Vec<&str> = line.split('\t').collect();
        // Whitespace hooks trim the empty trailing columns of `none` rows.
        assert!((4..=6).contains(&cols.len()), "bad row: {line}");
        while cols.len() < 6 {
            cols.push("");
        }
        let [red, green, blue, depth_name, fg_param, bg_param] = cols.as_slice() else {
            panic!("bad row: {line}");
        };
        let channel = |digits: &str| digits.parse::<u8>().expect("channel");
        let rgb = [channel(red), channel(green), channel(blue)];
        let depth = match *depth_name {
            "truecolor" => ColorDepth::TrueColor,
            "256" => ColorDepth::Ansi256,
            "16" => ColorDepth::Ansi16,
            "none" => ColorDepth::Mono,
            other => panic!("unknown depth {other}"),
        };
        assert_eq!(fitted(rgb, false, depth), expected(fg_param), "fg {line}");
        assert_eq!(fitted(rgb, true, depth), expected(bg_param), "bg {line}");
        rows += 1;
    }
    assert!(rows > 0, "no vectors read");
}
