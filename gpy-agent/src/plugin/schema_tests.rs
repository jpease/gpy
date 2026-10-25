//! Drift tests for the published plugin manifest JSON schemas.
//!
//! The schemas under `gpy-agent/schemas/` are generated from the Rust types;
//! regenerate them with `GPY_UPDATE_SCHEMAS=1 cargo nextest run schema_tests`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "tests: panics are the assertions"
)]

use super::{PluginManifest, ScriptSegmentManifest};
use std::path::Path;

fn check(file_name: &str, schema: &schemars::Schema) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(file_name);
    let rendered = format!("{}\n", serde_json::to_string_pretty(schema).unwrap());
    if std::env::var_os("GPY_UPDATE_SCHEMAS").is_some() {
        std::fs::write(&path, &rendered).expect("write schema");
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == rendered,
        "{file_name} differs from the Rust types; regenerate with \
         `GPY_UPDATE_SCHEMAS=1 cargo nextest run schema_tests` and review the diff"
    );
}

#[test]
fn plugin_manifest_schema_matches_the_types() {
    check(
        "plugin-manifest.json",
        &schemars::schema_for!(PluginManifest),
    );
}

#[test]
fn script_segment_manifest_schema_matches_the_types() {
    check(
        "script-segment-manifest.json",
        &schemars::schema_for!(ScriptSegmentManifest),
    );
}
