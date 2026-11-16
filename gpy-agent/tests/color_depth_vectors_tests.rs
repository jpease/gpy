//! The 24-bit colour downgrade must match the shared vectors that the Bash,
//! Zsh and Fish renderers also read (`tests/fixtures/color_depth_vectors.tsv`).

use gpy_agent::formatter::{ColorDepth, PromptDialect};
use gpy_agent::template::{Color, Span, SpanKind, Style};
use std::path::Path;

fn encode(rgb: [u8; 3], background: bool, depth: ColorDepth) -> String {
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
    PromptDialect::Ansi.encode_depth(&[span], depth)
}

fn expected(param: &str) -> String {
    if param.is_empty() {
        "x".to_owned()
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
        let cols: Vec<&str> = line.split('\t').collect();
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
        assert_eq!(encode(rgb, false, depth), expected(fg_param), "fg {line}");
        assert_eq!(encode(rgb, true, depth), expected(bg_param), "bg {line}");
        rows += 1;
    }
    assert!(rows > 0, "no vectors read");
}
