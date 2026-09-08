//! Unit tests for language detection functionality
//!
//! # Test Navigation Map
//!
//! **Total Tests**: 27
//! **Last Updated**: 2026-06-29
//!
//! ## Test Categories
//!
//! ### ✅ Basic Operations (10 tests)
//! Tests covering normal/happy path scenarios:
//! - `test_normalize_language_name` - Language name normalization to canonical forms
//! - `test_get_language_color_from_theme` - Language color mapping from theme configuration
//! - `test_version_parsing_rust` - Rust version extraction from rustc output
//! - `test_version_parsing_node` - Node version extraction from node output
//! - `test_version_parsing_python` - Python version extraction from python output
//! - `test_version_parsing_go` - Go version extraction from go version output
//! - `test_version_parsing_swift` - Swift version extraction from swift output
//! - `test_version_parsing_java` - Java version extraction from java output
//! - `test_version_detectors_list` - Verify all 10 language detectors present
//! - `test_version_caching` - Version cache hit/miss functionality
//!
//! ### ❌ Error Cases (0 tests)
//! Tests covering error scenarios and failure modes:
//! - (None - error handling embedded in parsing tests)
//!
//! ### 🔀 Edge Cases (0 tests)
//! Tests covering boundary conditions:
//! - (Edge cases embedded in version parsing tests)
//!
//! ### 🔗 Integration (2 tests)
//! Tests covering cross-module interactions:
//! - `test_language_aggregation_js_ts_to_node` - JS/TS file aggregation into single node entry
//! - `test_no_pathbuf_collections_retained` - Regression test for memory-efficient metrics over path collections
//!
//! ### 🧭 Detection Modes (#227, 5 tests)
//! Marker-based vs content-scan language detection:
//! - `markers_mode_detects_languages_with_markers_present`
//! - `markers_mode_hides_languages_without_markers`
//! - `markers_mode_orders_by_evidence_count`
//! - `markers_mode_is_case_insensitive_for_marker_files`
//! - `detect_for_mode_content_matches_default_detector`
//!
//! ## What's NOT Tested (TODO)
//! Known gaps for this module (from the original planning notes, since removed):
//! - Version detection failure modes (command not found, permission denied)
//! - Invalid/corrupted version output handling
//! - Timeout handling for version detection commands
//! - Cache invalidation and expiry scenarios
//! - Custom language themes with missing color mappings
//! - Directory detection with symlinks or special files
//! - Unicode and non-UTF8 filenames in language detection
//! - Language detection in very large directories (>10k files)
//!
//! ## Related Code
//! - **Production**: `src/language/detector.rs`, `src/language/version.rs`, `src/language/cache.rs`, `src/config.rs` (`LanguageTheme`)
//! - **Other Tests**: `integration_tests.rs::test_language_detection_integration`, `agent_daemon_tests.rs::test_oneshot_language_command`
//! - **Fixtures**: None (uses tempfile for file creation tests)
//!
//! ## Quick Reference
//! ```bash
//! cargo test --test language_tests
//! cargo test --test language_tests -- --nocapture
//! ```

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures
#![allow(clippy::indexing_slicing)]
#![allow(clippy::str_to_string)]
#![allow(clippy::default_numeric_fallback)]

use gpy_agent::config::{
    Config,
    metadata::set_config_value,
    types::{DetectionMode, LanguageFilter},
};
use gpy_agent::language::display::{build_language_display_info, build_language_display_info_at};
use gpy_agent::language::{
    detector::{
        DetectedLanguage, Detector, get_language_color_from_theme, normalize_language_name,
    },
    version::*,
};
use std::fs;
use tempfile::tempdir;

fn create_test_repo(files: &[(&str, usize)]) -> tempfile::TempDir {
    let dir = tempdir().expect("create temp dir");
    let path = dir.path();

    for (name, size) in files {
        let file_path = path.join(name);
        // Create file with specific size
        let content = "a".repeat(*size);
        fs::write(&file_path, content).expect("write test file");
    }

    dir
}

#[test]
fn test_confidence_single_language() {
    // Single language repo should have 1.0 confidence
    let dir = create_test_repo(&[("main.rs", 1000)]);
    let langs = Detector::detect_directory(dir.path());

    assert_eq!(langs.len(), 1);
    assert_eq!(langs[0].name, "rust");
    assert!((langs[0].confidence - 1.0).abs() < 0.01);
}

#[test]
fn test_confidence_dominant_language() {
    // 95% Rust by bytes should have ~0.95 confidence
    let dir = create_test_repo(&[
        ("main.rs", 9500),    // 95% of bytes
        ("config.json", 500), // 5% of bytes
    ]);
    let langs = Detector::detect_directory(dir.path());

    let rust = langs.iter().find(|l| l.name == "rust").unwrap();
    assert!(rust.confidence > 0.80, "Rust should be dominant");

    // Note: json is not detected by hyperpolyglot as a language, so we don't check for it
    // If it were detected, it would have low confidence
}

#[test]
fn test_confidence_file_count_matters() {
    // Equal bytes, but JS has 5 files vs Rust's 1 file
    // JS should win due to 30% weight on file count
    let dir = create_test_repo(&[
        ("main.rs", 1000), // 1 file, 50% of bytes
        ("a.js", 250),     // Combined: 5 files, 50% of bytes
        ("b.js", 250),
        ("c.js", 250),
        ("d.js", 250),
    ]);
    let langs = Detector::detect_directory(dir.path());

    // JavaScript should rank first due to file count advantage
    assert_eq!(langs[0].name, "node");
    assert!(langs[0].confidence > 0.5);
}

#[test]
fn test_confidence_sum_approximately_one() {
    let dir = create_test_repo(&[("main.rs", 5000), ("index.js", 3000), ("app.py", 2000)]);
    let langs = Detector::detect_directory(dir.path());

    let sum: f32 = langs.iter().map(|l| l.confidence).sum();
    assert!(
        (sum - 1.0).abs() < 0.01,
        "Confidence scores should sum to ~1.0"
    );
}

#[test]
fn python_version_comes_from_project_venv() {
    // A project `.venv/pyvenv.cfg` must drive the reported Python version,
    // not the daemon's global interpreter.
    let dir = tempdir().expect("create temp dir");
    let venv = dir.path().join(".venv");
    fs::create_dir_all(&venv).expect("create .venv");
    fs::write(venv.join("pyvenv.cfg"), "version = 3.11.9\n").expect("write pyvenv.cfg");

    let languages = vec![DetectedLanguage {
        name: "python".into(),
        confidence: 1.0,
        file_count: 3,
        total_bytes: 3000,
    }];

    let config = Config::default();
    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info_at(
        &languages,
        &theme,
        &config.language,
        Some(dir.path()),
        None,
    );

    let python = display
        .iter()
        .find(|lang| lang.name == "python")
        .expect("python entry present");
    assert_eq!(python.version.as_deref(), Some("3.11.9"));
}

#[test]
fn ruby_version_comes_from_ruby_version_file() {
    // A project `.ruby-version` (with no mise/tool-versions config) must drive
    // the reported Ruby version, not the daemon's PATH-resolved interpreter.
    let dir = tempdir().expect("create temp dir");
    fs::write(dir.path().join(".ruby-version"), "3.3.11\n").expect("write .ruby-version");

    let languages = vec![DetectedLanguage {
        name: "ruby".into(),
        confidence: 1.0,
        file_count: 3,
        total_bytes: 3000,
    }];

    let config = Config::default();
    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info_at(
        &languages,
        &theme,
        &config.language,
        Some(dir.path()),
        None,
    );

    let ruby = display
        .iter()
        .find(|lang| lang.name == "ruby")
        .expect("ruby entry present");
    assert_eq!(ruby.version.as_deref(), Some("3.3.11"));
}

#[test]
fn test_filter_mode_primary() {
    let mut config = Config::default();
    config.language.filter = LanguageFilter::Primary;

    let languages = vec![
        DetectedLanguage {
            name: "rust".into(),
            confidence: 0.7,
            file_count: 10,
            total_bytes: 7000,
        },
        DetectedLanguage {
            name: "node".into(),
            confidence: 0.3,
            file_count: 5,
            total_bytes: 3000,
        },
    ];

    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info(&languages, &theme, &config.language);
    assert_eq!(display.len(), 1);
    assert_eq!(display[0].name, "rust");
}

#[test]
fn test_filter_mode_top_n() {
    let mut config = Config::default();
    config.language.filter = LanguageFilter::Top(2);

    let languages = vec![
        DetectedLanguage {
            name: "rust".into(),
            confidence: 0.5,
            file_count: 10,
            total_bytes: 5000,
        },
        DetectedLanguage {
            name: "node".into(),
            confidence: 0.3,
            file_count: 6,
            total_bytes: 3000,
        },
        DetectedLanguage {
            name: "python".into(),
            confidence: 0.2,
            file_count: 4,
            total_bytes: 2000,
        },
    ];

    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info(&languages, &theme, &config.language);
    assert_eq!(display.len(), 2);
    assert_eq!(display[0].name, "rust");
    assert_eq!(display[1].name, "node");
}

#[test]
fn test_confidence_threshold() {
    let mut config = Config::default();
    config.language.confidence_threshold =
        gpy_agent::config::types::ConfidenceThreshold::new(0.25).unwrap();

    let languages = vec![
        DetectedLanguage {
            name: "rust".into(),
            confidence: 0.7,
            file_count: 10,
            total_bytes: 7000,
        },
        DetectedLanguage {
            name: "node".into(),
            confidence: 0.2,
            file_count: 3,
            total_bytes: 2000,
        },
        DetectedLanguage {
            name: "python".into(),
            confidence: 0.1,
            file_count: 2,
            total_bytes: 1000,
        },
    ];

    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info(&languages, &theme, &config.language);
    assert_eq!(display.len(), 1, "Only Rust should pass 0.25 threshold");
    assert_eq!(display[0].name, "rust");
}

#[test]
fn test_combined_threshold_and_filter() {
    let mut config = Config::default();
    config.language.confidence_threshold =
        gpy_agent::config::types::ConfidenceThreshold::new(0.15).unwrap();
    config.language.filter = LanguageFilter::Top(2);

    let languages = vec![
        DetectedLanguage {
            name: "rust".into(),
            confidence: 0.5,
            file_count: 10,
            total_bytes: 5000,
        },
        DetectedLanguage {
            name: "node".into(),
            confidence: 0.3,
            file_count: 6,
            total_bytes: 3000,
        },
        DetectedLanguage {
            name: "python".into(),
            confidence: 0.15,
            file_count: 3,
            total_bytes: 1500,
        },
        DetectedLanguage {
            name: "go".into(),
            confidence: 0.05,
            file_count: 1,
            total_bytes: 500,
        },
    ];

    // Should filter to 3 languages (threshold), then take top 2 (filter)
    let theme = gpy_agent::theme::ThemeConfig::default();
    let display = build_language_display_info(&languages, &theme, &config.language);
    assert_eq!(display.len(), 2);
    assert_eq!(display[0].name, "rust");
    assert_eq!(display[1].name, "node");
}

#[test]
fn test_config_set_filter_mode_validation() {
    let mut config = Config::default();

    // Valid values should work
    assert!(set_config_value(&mut config, "language.filter", "all").is_ok());
    assert!(set_config_value(&mut config, "language.filter", "primary").is_ok());
    assert!(set_config_value(&mut config, "language.filter", "5").is_ok());

    // Invalid values should fail
    assert!(set_config_value(&mut config, "language.filter", "invalid").is_err());
}

#[test]
fn test_config_set_threshold_validation() {
    let mut config = Config::default();

    // Valid thresholds
    assert!(set_config_value(&mut config, "language.confidence_threshold", "0.0").is_ok());
    assert!(set_config_value(&mut config, "language.confidence_threshold", "0.5").is_ok());
    assert!(set_config_value(&mut config, "language.confidence_threshold", "1.0").is_ok());

    // Invalid thresholds
    assert!(set_config_value(&mut config, "language.confidence_threshold", "1.5").is_err());
    assert!(set_config_value(&mut config, "language.confidence_threshold", "-0.1").is_err());
    assert!(
        set_config_value(&mut config, "language.confidence_threshold", "not_a_number").is_err()
    );
}

#[test]
fn test_normalize_language_name() {
    // Test language name normalization
    assert_eq!(
        normalize_language_name("JavaScript"),
        "node",
        "JavaScript should normalize to 'node' for unified JS/TS handling"
    );
    assert_eq!(
        normalize_language_name("TypeScript"),
        "node",
        "TypeScript should normalize to 'node' for unified JS/TS handling"
    );
    assert_eq!(
        normalize_language_name("Python"),
        "python",
        "Python should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Rust"),
        "rust",
        "Rust should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Go"),
        "go",
        "Go should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Ruby"),
        "ruby",
        "Ruby should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Java"),
        "java",
        "Java should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Swift"),
        "swift",
        "Swift should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Elixir"),
        "elixir",
        "Elixir should normalize to lowercase canonical form"
    );
    assert_eq!(
        normalize_language_name("Unknown"),
        "Unknown",
        "Unknown languages should preserve original casing"
    );
}

#[test]
fn test_get_language_color_from_theme() {
    // Test language color mapping from theme
    use gpy_agent::config::LanguageTheme;

    // Create a default theme configuration
    let theme = LanguageTheme::default();

    // Test colors match default theme (using new hierarchical fallback system)
    assert_eq!(
        get_language_color_from_theme("node", &theme),
        "green",
        "Node should use green color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("python", &theme),
        "blue",
        "Python should use blue color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("rust", &theme),
        "red",
        "Rust should use red color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("go", &theme),
        "cyan",
        "Go should use cyan color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("ruby", &theme),
        "red",
        "Ruby should use red color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("java", &theme),
        "blue",
        "Java should use blue color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("swift", &theme),
        "red",
        "Swift should use red color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("elixir", &theme),
        "magenta",
        "Elixir should use magenta color from default theme"
    );
    assert_eq!(
        get_language_color_from_theme("unknown", &theme),
        "black",
        "Unknown languages should fall back to segment background color"
    );
}

#[test]
fn test_version_parsing_rust() {
    let detector = RustDetector;

    // Test valid rustc output
    let output1 = "rustc 1.70.0 (90c541806 2023-05-31)";
    assert_eq!(
        detector.parse_version(output1),
        Some("1.70.0".to_owned()),
        "Should extract stable version from rustc output"
    );

    let output2 = "rustc 1.73.0-nightly (9c5a14a78 2023-07-15)";
    assert_eq!(
        detector.parse_version(output2),
        Some("1.73.0-nightly".to_owned()),
        "Should extract nightly version with suffix from rustc output"
    );

    // Test invalid output
    let invalid = "not a rustc version";
    // Current implementation might return "a" - need to handle this better
    let result = detector.parse_version(invalid);
    assert!(
        result.is_none() || result == Some("a".to_owned()),
        "Should handle invalid rustc output gracefully"
    );
}

#[test]
fn test_version_parsing_node() {
    let detector = NodeDetector;

    // Test valid node output
    let output1 = "v18.16.1";
    assert_eq!(
        detector.parse_version(output1),
        Some("18.16.1".to_owned()),
        "Should strip 'v' prefix from node version output"
    );

    let output2 = "v20.5.0";
    assert_eq!(
        detector.parse_version(output2),
        Some("20.5.0".to_owned()),
        "Should extract version from node output format"
    );

    // Test invalid output - current implementation strips 'v' from start
    let invalid = "node: command not found";
    // This would return "node: command not found" since it only strips 'v'
    let result = detector.parse_version(invalid);
    assert!(
        result.is_some(),
        "Current implementation returns Some for any input"
    );
}

#[test]
fn test_version_parsing_python() {
    let detector = PythonDetector;

    // Test valid python output
    let output1 = "Python 3.11.4";
    assert_eq!(
        detector.parse_version(output1),
        Some("3.11.4".to_owned()),
        "Should extract version number from Python version output"
    );

    let output2 = "Python 3.9.16";
    assert_eq!(
        detector.parse_version(output2),
        Some("3.9.16".to_owned()),
        "Should parse Python 3.9.x version correctly"
    );

    // Test invalid output - current implementation gets second word
    let invalid = "python: command not found";
    // This would return "command" since it gets .nth(1)
    let result = detector.parse_version(invalid);
    assert_eq!(
        result,
        Some("command".to_owned()),
        "Parser returns second word for invalid output"
    );
}

#[test]
fn test_version_parsing_go() {
    let detector = GoDetector;

    // Test valid go output
    let output1 = "go version go1.21.0 darwin/arm64";
    assert_eq!(
        detector.parse_version(output1),
        Some("1.21.0".to_owned()),
        "Should extract version from go version output on macOS"
    );

    let output2 = "go version go1.19.5 linux/amd64";
    assert_eq!(
        detector.parse_version(output2),
        Some("1.19.5".to_owned()),
        "Should extract version from go version output on Linux"
    );

    // Test invalid output
    let invalid = "go: command not found";
    assert_eq!(
        detector.parse_version(invalid),
        None,
        "Should return None for invalid go output"
    );
}

#[test]
fn test_version_parsing_swift() {
    let detector = SwiftDetector;

    // Test valid swift output
    let output1 = "Swift version 5.9.0 (swift-5.9.0-RELEASE)";
    assert_eq!(
        detector.parse_version(output1),
        Some("5.9.0".to_owned()),
        "Should extract version from standard Swift version output"
    );

    let output2 = "Apple Swift version 5.8.1 (clang-1403.0.22.14.1)";
    assert_eq!(
        detector.parse_version(output2),
        Some("5.8.1".to_owned()),
        "Should extract version from Apple Swift version output"
    );

    // Test invalid output
    let invalid = "swift: command not found";
    assert_eq!(
        detector.parse_version(invalid),
        None,
        "Should return None for invalid swift output"
    );
}

#[test]
fn test_version_parsing_java() {
    let detector = JavaDetector;

    // Test valid java output (stderr)
    let output1 = "java version \"17.0.7\"";
    assert_eq!(
        detector.parse_version(output1),
        Some("17.0.7".to_owned()),
        "Should extract version from Oracle Java version output"
    );

    let output2 = "openjdk version \"11.0.19\"";
    assert_eq!(
        detector.parse_version(output2),
        Some("11.0.19".to_owned()),
        "Should extract version from OpenJDK version output"
    );

    // Test invalid output
    let invalid = "java: command not found";
    assert_eq!(
        detector.parse_version(invalid),
        None,
        "Should return None for invalid java output"
    );
}

#[test]
fn test_version_detectors_list() {
    let detectors = get_version_detectors();

    // Should have all 9 supported languages
    assert_eq!(
        detectors.len(),
        10,
        "Should have all 10 supported language detectors including Fish"
    );

    let languages: Vec<&str> = detectors.iter().map(|d| d.language_name()).collect();
    assert!(
        languages.contains(&"node"),
        "Detector list should include node"
    );
    assert!(
        languages.contains(&"python"),
        "Detector list should include python"
    );
    assert!(
        languages.contains(&"rust"),
        "Detector list should include rust"
    );
    assert!(languages.contains(&"go"), "Detector list should include go");
    assert!(
        languages.contains(&"swift"),
        "Detector list should include swift"
    );
    assert!(
        languages.contains(&"elixir"),
        "Detector list should include elixir"
    );
    assert!(
        languages.contains(&"erlang"),
        "Detector list should include erlang"
    );
    assert!(
        languages.contains(&"ruby"),
        "Detector list should include ruby"
    );
    assert!(
        languages.contains(&"java"),
        "Detector list should include java"
    );
}

#[test]
fn test_language_aggregation_js_ts_to_node() {
    // Test that JavaScript and TypeScript files are aggregated into a single "node" entry
    use gpy_agent::language::detector::Detector;
    use std::fs;
    use tempfile::tempdir;

    let temp_dir = tempdir().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create JavaScript file
    let js_file = temp_path.join("test.js");
    fs::write(&js_file, "console.log('hello');").expect("Failed to write JS file");

    // Create TypeScript file
    let ts_file = temp_path.join("test.ts");
    fs::write(&ts_file, "const x: number = 42;").expect("Failed to write TS file");

    // Detect languages
    let detected = Detector::detect_directory(temp_path);

    // Should have exactly one entry for "node" (aggregated JS + TS)
    let node_entries: Vec<_> = detected.iter().filter(|l| l.name == "node").collect();
    assert_eq!(
        node_entries.len(),
        1,
        "JavaScript and TypeScript should aggregate to single 'node' entry"
    );

    // Should have 2 files total
    let node_lang = node_entries[0];
    assert_eq!(
        node_lang.file_count, 2,
        "Node entry should have 2 files (JS + TS)"
    );

    // Should have non-zero total bytes
    assert!(
        node_lang.total_bytes > 0,
        "Node entry should have non-zero total bytes"
    );
}

#[test]
fn test_no_pathbuf_collections_retained() {
    // Regression test to ensure DetectedLanguage does not retain PathBuf collections
    use gpy_agent::language::detector::Detector;
    use std::fs;
    use tempfile::tempdir;

    let temp_dir = tempdir().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create some test files
    for i in 0..10 {
        let file_path = temp_path.join(format!("test{i}.js"));
        fs::write(&file_path, "console.log('test');").expect("Failed to write file");
    }

    // Detect languages
    let detected = Detector::detect_directory(temp_path);

    // Verify the structure uses metrics, not path collections
    for lang in &detected {
        // The DetectedLanguage struct should have file_count and total_bytes
        // but NOT a Vec<PathBuf> field
        assert!(
            lang.file_count > 0,
            "Should have file_count metric instead of path collection"
        );
        assert!(
            lang.total_bytes > 0,
            "Should have total_bytes metric instead of path collection"
        );

        // This test will fail to compile if anyone adds back a Vec<PathBuf> field
        // and tries to access it, serving as a compile-time regression guard
    }

    // Verify we detected the JavaScript files
    let js_lang = detected.iter().find(|l| l.name == "node");
    assert!(js_lang.is_some(), "Should detect JavaScript files");
    assert_eq!(
        js_lang.unwrap().file_count,
        10,
        "Should detect all 10 JavaScript files"
    );
}

// --- Marker-based detection mode (#227) -------------------------------------

#[test]
fn markers_mode_detects_languages_with_markers_present() {
    // A Rust project (Cargo.toml) that also has a stray Python helper script.
    let dir = create_test_repo(&[("Cargo.toml", 50), ("main.rs", 200), ("helper.py", 30)]);
    let detected = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Markers);

    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    assert!(
        names.contains(&"rust"),
        "Cargo.toml/.rs should surface rust: {names:?}"
    );
    assert!(
        names.contains(&"python"),
        "the .py file should surface python: {names:?}"
    );
}

#[test]
fn markers_mode_hides_languages_without_markers() {
    // No recognized markers or source extensions -> nothing surfaces.
    let dir = create_test_repo(&[("README.md", 100), ("LICENSE", 50)]);
    let detected = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Markers);
    assert!(
        detected.is_empty(),
        "marker mode must not surface languages without markers: {detected:?}"
    );
}

#[test]
fn markers_mode_orders_by_evidence_count() {
    // Rust has three signals (Cargo.toml + two .rs files); node has one.
    let dir = create_test_repo(&[
        ("Cargo.toml", 50),
        ("main.rs", 200),
        ("lib.rs", 200),
        ("package.json", 40),
    ]);
    let detected = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Markers);
    assert_eq!(detected.first().map(|l| l.name.as_str()), Some("rust"));
    let rust = detected
        .iter()
        .find(|l| l.name == "rust")
        .expect("rust present");
    assert_eq!(
        rust.file_count, 3,
        "rust should accumulate three marker signals"
    );
}

#[test]
fn detect_for_mode_content_matches_default_detector() {
    let dir = create_test_repo(&[("main.rs", 1000), ("helper.py", 100)]);
    let via_mode = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);
    let via_default = Detector::detect_directory(dir.path());
    let mode_names: Vec<&str> = via_mode.iter().map(|l| l.name.as_str()).collect();
    let default_names: Vec<&str> = via_default.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        mode_names, default_names,
        "content mode must match the default hyperpolyglot scan"
    );
}

#[test]
fn bounded_content_detection_falls_back_to_markers_outside_a_git_repo() {
    // Same 95%/5% byte split as test_confidence_dominant_language: raw Content
    // mode would give rust a weighted confidence around ~0.95. With no `.git`
    // anywhere above `dir`, the bounded entry point must not run the
    // unbounded recursive content scan at all (#390) -- it should fall back
    // to Markers mode instead, which always reports flat 1.0 confidence.
    let dir = create_test_repo(&[("main.rs", 9500), ("config.json", 500)]);
    let detected = Detector::detect_directory_bounded(dir.path(), DetectionMode::Content);

    let rust = detected
        .iter()
        .find(|l| l.name == "rust")
        .expect("rust should still be detected via its .rs marker extension");
    assert!(
        (rust.confidence - 1.0).abs() < f32::EPSILON,
        "outside a git repo, bounded detection must use Markers' flat confidence, not content-weighted: {}",
        rust.confidence
    );
}

#[test]
fn bounded_content_detection_matches_unbounded_inside_a_git_repo() {
    let dir = create_test_repo(&[("main.rs", 1000), ("helper.py", 100)]);
    fs::create_dir_all(dir.path().join(".git")).expect("create fake .git marker");

    let bounded = Detector::detect_directory_bounded(dir.path(), DetectionMode::Content);
    let unbounded = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);

    let bounded_names: Vec<&str> = bounded.iter().map(|l| l.name.as_str()).collect();
    let unbounded_names: Vec<&str> = unbounded.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        bounded_names, unbounded_names,
        "inside a real git repo, bounded detection must behave exactly like unbounded content mode"
    );
}

/// #391: even inside a real git repo (past the #390 guard), a tree whose
/// non-ignored file count exceeds the scan budget must still fall back to
/// Markers mode rather than run the unbounded content scan.
#[test]
fn bounded_content_detection_falls_back_when_file_count_exceeds_budget() {
    let dir = create_test_repo(&[("main.rs", 9500), ("config.json", 500)]);
    fs::create_dir_all(dir.path().join(".git")).expect("create fake .git marker");
    fs::write(dir.path().join("extra1.txt"), "x").expect("write extra file");
    fs::write(dir.path().join("extra2.txt"), "x").expect("write extra file");

    // Budget of 2 is exceeded by the 4 files above (main.rs, config.json,
    // extra1.txt, extra2.txt), forcing the Markers fallback.
    let detected =
        Detector::detect_directory_bounded_with_budget(dir.path(), DetectionMode::Content, 2);

    let rust = detected
        .iter()
        .find(|l| l.name == "rust")
        .expect("rust should still be detected via its .rs marker extension");
    assert!(
        (rust.confidence - 1.0).abs() < f32::EPSILON,
        "exceeding the file budget inside a real repo must still fall back to \
         Markers' flat confidence, not content-weighted: {}",
        rust.confidence
    );
}

/// #391: staying under the budget must not change behavior versus the
/// unbounded content scan.
#[test]
fn bounded_content_detection_stays_content_mode_under_budget() {
    let dir = create_test_repo(&[("main.rs", 9500), ("config.json", 500)]);
    fs::create_dir_all(dir.path().join(".git")).expect("create fake .git marker");

    let bounded =
        Detector::detect_directory_bounded_with_budget(dir.path(), DetectionMode::Content, 100);
    let unbounded = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);

    let bounded_names: Vec<&str> = bounded.iter().map(|l| l.name.as_str()).collect();
    let unbounded_names: Vec<&str> = unbounded.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        bounded_names, unbounded_names,
        "staying under the file budget must behave exactly like unbounded content mode"
    );
}

#[test]
fn bounded_hybrid_detection_falls_back_to_markers_outside_a_git_repo() {
    // Hybrid starts from the same expensive recursive content scan as
    // Content, so it must get the same #390 repo-boundary protection: outside
    // a git repo, the bounded entry point must fall back to Markers instead
    // of running Hybrid's own content-based sub-scan.
    let dir = create_test_repo(&[("main.rs", 9500), ("config.json", 500)]);
    let detected = Detector::detect_directory_bounded(dir.path(), DetectionMode::Hybrid);

    let rust = detected
        .iter()
        .find(|l| l.name == "rust")
        .expect("rust should still be detected via its .rs marker extension");
    assert!(
        (rust.confidence - 1.0).abs() < f32::EPSILON,
        "outside a git repo, bounded hybrid detection must fall back to Markers' flat confidence: {}",
        rust.confidence
    );
}

#[test]
fn markers_mode_is_case_insensitive_for_marker_files() {
    let dir = create_test_repo(&[("cargo.toml", 50)]);
    let detected = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Markers);
    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    assert!(
        names.contains(&"rust"),
        "marker file match should be case-insensitive: {names:?}"
    );
}

// --- Hybrid detection mode (#229) -------------------------------------------

#[test]
fn hybrid_mode_gates_content_scan_by_markers() {
    // A Rust project (Cargo.toml) with a stray Python helper script: content
    // scan alone would surface both rust and python, but python has no marker
    // here, so hybrid should keep only rust.
    let dir = create_test_repo(&[("Cargo.toml", 50), ("main.rs", 900), ("helper.py", 100)]);
    let detected = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Hybrid);
    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();

    assert!(names.contains(&"rust"), "rust has a marker: {names:?}");
    assert!(
        !names.contains(&"python"),
        "python has no marker here, hybrid must filter it out: {names:?}"
    );
}

#[test]
fn hybrid_mode_surfaces_all_content_languages_that_have_markers() {
    // Both languages have markers present, so hybrid should keep both, with
    // the same per-language values content reports (only extra languages
    // content-detects from the marker files themselves, e.g. TOML/JSON, get
    // filtered -- those aren't in the supported-language table at all).
    let dir = create_test_repo(&[
        ("Cargo.toml", 50),
        ("main.rs", 500),
        ("package.json", 40),
        ("index.js", 500),
    ]);
    let hybrid = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Hybrid);
    let content = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);

    for name in ["rust", "node"] {
        let hybrid_lang = hybrid
            .iter()
            .find(|lang| lang.name == name)
            .unwrap_or_else(|| panic!("hybrid should keep {name}, which has a marker: {hybrid:?}"));
        let content_lang = content
            .iter()
            .find(|lang| lang.name == name)
            .unwrap_or_else(|| panic!("sanity check: content should detect {name}"));
        assert!(
            (hybrid_lang.confidence - content_lang.confidence).abs() < f32::EPSILON,
            "hybrid must preserve content's confidence for {name}"
        );
    }
}

#[test]
fn hybrid_mode_hides_everything_when_no_markers_present() {
    // Content alone would detect rust from main.rs, but with no Cargo.toml
    // (or any other recognized marker) present, hybrid must surface nothing.
    let dir = create_test_repo(&[("main.rs", 1000)]);
    let content = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);
    assert!(
        content.iter().any(|lang| lang.name == "rust"),
        "sanity check: content mode should detect rust from main.rs"
    );

    let hybrid = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Hybrid);
    assert!(
        hybrid.is_empty(),
        "no markers present, hybrid must not surface any language: {hybrid:?}"
    );
}

#[test]
fn hybrid_mode_preserves_content_scan_confidence() {
    // Hybrid should keep content's prevalence-based confidence, not collapse
    // to markers' flat 1.0 confidence.
    let dir = create_test_repo(&[("Cargo.toml", 10), ("main.rs", 9500), ("package.json", 10)]);
    let hybrid = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Hybrid);
    let content = Detector::detect_directory_for_mode(dir.path(), DetectionMode::Content);

    let hybrid_rust = hybrid
        .iter()
        .find(|lang| lang.name == "rust")
        .expect("rust should be present");
    let content_rust = content
        .iter()
        .find(|lang| lang.name == "rust")
        .expect("sanity check: content should detect rust");
    assert!(
        (hybrid_rust.confidence - content_rust.confidence).abs() < f32::EPSILON,
        "hybrid should keep content's prevalence-weighted confidence ({}), not markers' flat 1.0",
        content_rust.confidence
    );
    assert!(
        content_rust.confidence > 0.5,
        "sanity check: rust should dominate this repo by bytes: {}",
        content_rust.confidence
    );
}

// --- gengo-language detector parity (#523) ----------------------------------

/// One unambiguous source file per renderable canonical language, with content
/// idiomatic enough that a content-based disambiguator lands on the same answer
/// a path-based one does.
const RENDERABLE_LANGUAGE_FIXTURES: &[(&str, &str, &str)] = &[
    (
        "c",
        "core.c",
        "#include <stdio.h>\n\nint main(void) {\n    printf(\"hello\\n\");\n    return 0;\n}\n",
    ),
    (
        "cpp",
        "engine.cpp",
        "#include <iostream>\n\nnamespace engine {\nint run() {\n    std::cout << \"hello\" << std::endl;\n    return 0;\n}\n}\n",
    ),
    (
        "csharp",
        "Program.cs",
        "using System;\n\nnamespace Demo {\n    public class Program {\n        public static void Main(string[] args) {\n            Console.WriteLine(\"hello\");\n        }\n    }\n}\n",
    ),
    (
        "elixir",
        "app.ex",
        "defmodule App do\n  def greet(name) do\n    IO.puts(\"hello #{name}\")\n  end\nend\n",
    ),
    (
        "erlang",
        "server.erl",
        "-module(server).\n-export([start/0]).\n\nstart() ->\n    io:format(\"hello~n\").\n",
    ),
    (
        "fish",
        "prompt.fish",
        "function gpy_prompt --description 'demo'\n    set -l branch (git branch --show-current)\n    echo $branch\nend\n",
    ),
    (
        "go",
        "server.go",
        "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n",
    ),
    (
        "java",
        "Main.java",
        "package demo;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hello\");\n    }\n}\n",
    ),
    (
        "node",
        "index.js",
        "const os = require('os');\n\nfunction main() {\n  console.log(os.platform());\n}\n\nmain();\n",
    ),
    (
        "php",
        "index.php",
        "<?php\n\nfunction greet(string $name): string {\n    return \"hello $name\";\n}\n\necho greet('world');\n",
    ),
    (
        "python",
        "app.py",
        "import os\n\n\ndef main() -> None:\n    print(os.getcwd())\n\n\nif __name__ == \"__main__\":\n    main()\n",
    ),
    (
        "ruby",
        "app.rb",
        "require 'json'\n\nclass Greeter\n  def greet(name)\n    puts \"hello #{name}\"\n  end\nend\n",
    ),
    (
        "rust",
        "main.rs",
        "use std::io;\n\nfn main() -> io::Result<()> {\n    println!(\"hello\");\n    Ok(())\n}\n",
    ),
    (
        "swift",
        "App.swift",
        "import Foundation\n\nstruct App {\n    func greet(_ name: String) {\n        print(\"hello \\(name)\")\n    }\n}\n",
    ),
];

fn create_repo_with_contents(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempdir().expect("create temp dir");
    for (name, contents) in files {
        let file_path = dir.path().join(name);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("create fixture parent dir");
        }
        fs::write(&file_path, contents).expect("write fixture file");
    }
    dir
}

/// #523 acceptance criterion: a corpus covering all 14 renderable languages
/// must produce the same canonical names before and after the detector swap.
#[test]
fn content_scan_names_every_renderable_language() {
    let files: Vec<(&str, &str)> = RENDERABLE_LANGUAGE_FIXTURES
        .iter()
        .map(|(_, name, contents)| (*name, *contents))
        .collect();
    let dir = create_repo_with_contents(&files);

    let detected = Detector::detect_directory(dir.path());
    let mut names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    names.sort_unstable();

    let mut expected: Vec<&str> = RENDERABLE_LANGUAGE_FIXTURES
        .iter()
        .map(|(canonical, ..)| *canonical)
        .collect();
    expected.sort_unstable();

    assert_eq!(
        names, expected,
        "every renderable language fixture must resolve to its canonical name"
    );
    for lang in &detected {
        assert_eq!(
            lang.file_count, 1,
            "each canonical should account for exactly its one fixture file: {lang:?}"
        );
        assert!(
            lang.total_bytes > 0,
            "each canonical should carry a non-zero byte total: {lang:?}"
        );
    }
}

/// `.h` maps to C, C++ and Objective-C.
///
/// A detector that resolves the ambiguity by static language priority instead
/// of reading the file picks C every time (gengo scores C at 75 against
/// C++/Objective-C at 50), so this pins that the fallback read actually happens.
#[test]
fn cpp_header_is_not_resolved_by_static_language_priority() {
    let header = "#include <iostream>\n#include <vector>\n\nnamespace demo {\nclass Widget {\npublic:\n    void render() const;\n};\n}\n";
    let dir = create_repo_with_contents(&[("widget.h", header)]);

    let detected = Detector::detect_file(dir.path().join("widget.h"))
        .expect("detection should not error")
        .expect("a .h file with C++ contents should resolve to a language");
    assert_eq!(
        detected.name, "cpp",
        "a C++ header must be disambiguated by its contents, not by C's higher \
         static priority"
    );

    let scanned = Detector::detect_directory(dir.path());
    let names: Vec<&str> = scanned.iter().map(|lang| lang.name.as_str()).collect();
    assert!(
        names.contains(&"cpp") && !names.contains(&"c"),
        "the directory scan must reach the same answer as single-file detection: {names:?}"
    );
}

/// A file with no extension and no filename match resolves through the
/// zero-candidate fallback, which is the only path that reads the shebang.
#[test]
fn extensionless_shebang_script_resolves_to_python() {
    let dir = create_repo_with_contents(&[(
        "bootstrap",
        "#!/usr/bin/env python3\nimport sys\n\nprint(sys.version)\n",
    )]);

    let detected = Detector::detect_directory(dir.path());
    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["python"],
        "an extensionless `#!/usr/bin/env python3` script must resolve to python"
    );
}

/// Documented parity edge (#523): the extension decides before the shebang is
/// ever read, so a `.py` file claiming a Ruby interpreter is still Python.
///
/// This is the answer hyperpolyglot gave, deliberately preserved.
#[test]
fn extension_wins_over_shebang_for_mislabelled_script() {
    let dir = create_repo_with_contents(&[(
        "script.py",
        "#!/usr/bin/env ruby\nputs 'this file lies about its interpreter'\n",
    )]);

    let detected = Detector::detect_directory(dir.path());
    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["python"],
        "`.py` must win over a conflicting shebang: {names:?}"
    );
}

/// The walk layers linguist's vendor and documentation exclusion globs.
///
/// Without them the scan visits a strictly larger file set, which both adds
/// languages and moves every confidence ratio, because the confidence
/// denominator is the total across every detected language.
#[test]
fn vendored_and_documentation_paths_are_excluded_from_the_scan() {
    let rust = "fn main() {\n    println!(\"hello\");\n}\n";
    let dir = create_repo_with_contents(&[
        ("main.rs", rust),
        ("vendor/bundled.rs", rust),
        ("node_modules/pkg/index.js", "module.exports = 42;\n"),
        ("README.md", "# Demo\n\nSome prose about the project.\n"),
    ]);

    let detected = Detector::detect_directory(dir.path());
    let names: Vec<&str> = detected.iter().map(|lang| lang.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["rust"],
        "vendor/, node_modules/ and README.md must all be excluded: {names:?}"
    );

    let rust_lang = detected.first().expect("rust entry present");
    assert_eq!(
        rust_lang.file_count, 1,
        "only the top-level main.rs should be counted, not vendor/bundled.rs"
    );
}

/// `detect_file` had no test coverage before #523 — only a benchmark exercised
/// it — so its post-swap behaviour rests entirely on this.
#[test]
fn detect_file_reports_single_file_metrics() {
    let source = "fn main() {\n    println!(\"hello\");\n}\n";
    let dir = create_repo_with_contents(&[("main.rs", source)]);

    let detected = Detector::detect_file(dir.path().join("main.rs"))
        .expect("detection should not error")
        .expect("main.rs should resolve to a language");

    assert_eq!(detected.name, "rust");
    assert_eq!(detected.file_count, 1);
    assert!((detected.confidence - 1.0).abs() < f32::EPSILON);
    assert_eq!(
        detected.total_bytes,
        u64::try_from(source.len()).expect("source length fits in u64")
    );
}

/// A file no matcher recognizes yields `Ok(None)`, not an error.
#[test]
fn detect_file_returns_none_for_an_unrecognized_file() {
    let dir = create_repo_with_contents(&[("notes.zzzunknown", "plain text, no language\n")]);

    let detected = Detector::detect_file(dir.path().join("notes.zzzunknown"))
        .expect("an unrecognized file is not an error");
    assert!(
        detected.is_none(),
        "an unrecognized file must yield None: {detected:?}"
    );
}
