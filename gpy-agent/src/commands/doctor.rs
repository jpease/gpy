//! Diagnostic checks for a GPY installation.
//!
//! The doctor command validates the pieces users most often need to debug:
//! agent liveness, config parsing, theme loading, segment/plugin discovery,
//! prompt performance risks, and required binaries. It composes lower-level
//! modules but keeps output formatting and remediation text local to the CLI.

use crate::plugin::discover_plugins;
use crate::{Error, Result, config};
use std::collections::{HashMap, HashSet};
use std::process::Command;

const SLOW_PATTERN_HINTS: &[(&str, &str)] = &[
    ("curl", "network call"),
    ("wget", "network call"),
    ("http", "network call"),
    ("kubectl", "external tool invocation"),
    ("docker", "external tool invocation"),
    ("git ", "subprocess git call"),
    ("python", "interpreter spawn"),
    ("node", "interpreter spawn"),
];

/// Run diagnostics to verify the health of the GPY installation and configuration.
///
/// # Errors
///
/// Returns an error if any critical health checks fail.
pub fn run() -> Result<()> {
    println!("GPY Doctor - System Health Report\n");

    let mut all_ok = true;

    all_ok &= check_agent_status();
    println!();

    let discovery = discover_plugins();
    all_ok &= check_config_and_theme(&discovery);
    println!();

    all_ok &= check_plugins(&discovery);
    println!();

    check_plugin_performance(&discovery);
    println!();

    all_ok &= check_binaries();

    println!("\nSummary:");
    if all_ok {
        println!("✅ Your GPY environment is healthy and ready to go!");
        Ok(())
    } else {
        println!("❌ Issues were found. Please review the remediation hints above.");
        Err(Error::config("Health check failed".to_owned()))
    }
}

fn check_agent_status() -> bool {
    println!("[Agent]");
    print!("  Checking process... ");
    if check_agent_running() {
        println!("✅ Running");
        true
    } else {
        println!("⚠️  Not running");
        println!("  Hint: Start the agent with `gpy start` or just use your prompt.");
        false
    }
}

fn check_active_palette(palette_name: &str) -> bool {
    print!("  Checking active palette... ");
    match crate::commands::palette::validate_by_name(palette_name) {
        Ok(report) => {
            println!(
                "✅ Loaded successfully ('{}' from {})",
                report.target, report.source
            );
            true
        }
        Err(e) => {
            println!("❌ Validation failed");
            crate::commands::palette::print_palette_validation_error("palette check", &e);
            false
        }
    }
}

fn check_config_and_theme(discovery: &crate::plugin::PluginDiscovery) -> bool {
    println!("[Configuration]");
    print!("  Checking config file... ");
    let cfg = match config::loader::load_config() {
        Ok(cfg) => {
            println!("✅ Valid");
            cfg
        }
        Err(e) => {
            println!("❌ Invalid");
            println!("  Diagnostic: {e}");
            println!("  Remediation: Fix the syntax error in ~/.config/gpy/config.toml");
            return false;
        }
    };

    print!("  Checking active theme... ");
    let theme_name = cfg.ui.theme.as_str();
    // `validate_by_name` already validates segment format templates as part of
    // loading the theme (see `theme::validate_theme_name`), so a single call
    // covers both the "active theme" and "segment templates" checks below —
    // running it twice under two labels validated nothing extra, it just
    // repeated the same filesystem/parse work (#625).
    let theme_result = crate::commands::theme::validate_by_name(theme_name);
    let theme_ok = match &theme_result {
        Ok(report) => {
            println!(
                "✅ Loaded successfully ('{}' from {})",
                report.target, report.source
            );
            true
        }
        Err(e) => {
            println!("❌ Validation failed");
            crate::commands::theme::print_theme_validation_error("theme check", e);
            false
        }
    };

    let palette_ok = check_active_palette(cfg.ui.palette.as_str());

    print!("  Checking segment templates... ");
    let templates_ok = match &theme_result {
        Ok(_) => {
            println!("✅ Valid");
            true
        }
        Err(e) => {
            println!("❌ Invalid template");
            println!("  Diagnostic: {e}");
            println!("  Remediation: Fix the segment `format` template named above.");
            false
        }
    };

    print!("  Checking enabled segments... ");
    let segments_ok = check_segments(&cfg.ui.enabled_segments, discovery);

    theme_ok && palette_ok && segments_ok && templates_ok
}

fn check_segments(enabled_segments: &[String], discovery: &crate::plugin::PluginDiscovery) -> bool {
    let valid_segments = valid_segment_names(discovery);

    let invalid: Vec<_> = enabled_segments
        .iter()
        .filter(|segment| !valid_segments.contains(segment.as_str()))
        .collect();

    if invalid.is_empty() {
        println!("✅ All segments recognized");
        true
    } else {
        println!("❌ Unrecognized segments found");
        for seg in invalid {
            println!("    - '{seg}'");
        }
        println!(
            "  Remediation: Check for typos in `enabled_segments` or ensure the providing plugin is installed."
        );
        false
    }
}

fn valid_segment_names(discovery: &crate::plugin::PluginDiscovery) -> HashSet<&str> {
    let mut valid_segments: HashSet<&str> = crate::plugin::BUILTIN_ORDER
        .iter()
        .map(|builtin| builtin.as_str())
        .collect();

    for plugin in &discovery.plugins {
        for segment in &plugin.manifest.provided_segments {
            valid_segments.insert(segment.as_str());
        }
    }

    valid_segments
}

fn check_plugins(discovery: &crate::plugin::PluginDiscovery) -> bool {
    println!("[Plugins]");
    if discovery.plugins.is_empty() && discovery.diagnostics.is_empty() {
        println!("  No plugins discovered (this is fine).");
        return true;
    }

    let mut all_plugins_ok = true;

    if !discovery.plugins.is_empty() {
        println!("  Discovered Plugins:");
        for plugin in &discovery.plugins {
            println!(
                "    ✅ {} v{} [{}]",
                plugin.manifest.id,
                plugin.manifest.version,
                crate::commands::plugin::render_source(plugin.source)
            );
        }
    }

    if !discovery.diagnostics.is_empty() {
        all_plugins_ok = false;
        println!("  Plugin Issues:");
        for diag in &discovery.diagnostics {
            println!(
                "    ❌ {} [{}]",
                diag.plugin_dir.display(),
                crate::commands::plugin::render_source(diag.source)
            );
            println!("       Error: {}", diag.message);
        }
        println!(
            "  Remediation: Review the plugin manifests (plugin.toml) for the reported errors."
        );
    }

    all_plugins_ok
}

fn check_plugin_performance(discovery: &crate::plugin::PluginDiscovery) {
    println!("[Plugin Performance]");

    let Ok(cfg) = config::loader::load_config() else {
        println!("  Skipped: config unavailable.");
        return;
    };

    let enabled_plugin_segments = enabled_plugin_segments(&cfg, discovery);

    if enabled_plugin_segments.is_empty() {
        println!("  No enabled community plugin segments detected.");
        return;
    }

    println!("  Enabled community plugin segments:");
    for (segment, plugin) in enabled_plugin_segments {
        print_plugin_performance_hint(segment, plugin);
    }
}

fn enabled_plugin_segments<'a>(
    cfg: &'a crate::config::Config,
    discovery: &'a crate::plugin::PluginDiscovery,
) -> Vec<(&'a str, &'a crate::plugin::DiscoveredPlugin)> {
    let segment_index = plugin_segment_index(discovery);

    cfg.ui
        .enabled_segments
        .iter()
        .filter_map(|segment| {
            segment_index
                .get(segment.as_str())
                .map(|plugin| (segment.as_str(), *plugin))
        })
        .collect()
}

fn plugin_segment_index(
    discovery: &crate::plugin::PluginDiscovery,
) -> HashMap<&str, &crate::plugin::DiscoveredPlugin> {
    let mut segment_index = HashMap::new();

    for plugin in &discovery.plugins {
        if matches!(plugin.source, crate::plugin::PluginSource::FirstParty) {
            continue;
        }
        for segment in &plugin.manifest.provided_segments {
            segment_index.entry(segment.as_str()).or_insert(plugin);
        }
    }

    segment_index
}

fn print_plugin_performance_hint(segment: &str, plugin: &crate::plugin::DiscoveredPlugin) {
    let segment_path = plugin.root.join("segments").join(format!("{segment}.fish"));
    println!(
        "    - {segment} (plugin: {}, file: {})",
        plugin.manifest.id,
        segment_path.display()
    );

    match std::fs::read_to_string(&segment_path) {
        Ok(content) => print_suspect_patterns(segment, &content),
        Err(error) => {
            println!("      heuristic: unable to inspect file ({error})");
        }
    }
}

fn print_suspect_patterns(segment: &str, content: &str) {
    let suspects = SLOW_PATTERN_HINTS
        .iter()
        .filter_map(|(pattern, reason)| content.contains(pattern).then_some(*reason))
        .collect::<Vec<_>>();
    if suspects.is_empty() {
        println!("      heuristic: no obvious slow patterns detected");
    } else {
        println!(
            "      heuristic: possible slowdown source ({})",
            suspects.join(", ")
        );
        println!(
            "      hint: if your prompt feels slow, disable `{segment}` first to confirm whether this plugin segment is responsible"
        );
    }
}

fn check_binaries() -> bool {
    println!("[System]");
    print!("  Checking gpy-agent binary... ");
    if Command::new("gpy-agent").arg("--version").output().is_ok() {
        println!("✅ Found in PATH");
        true
    } else {
        println!("❌ Not found in PATH");
        println!(
            "  Remediation: Ensure gpy-agent is installed and your PATH includes its location."
        );
        false
    }
}

/// Whether a live agent answered `gpy-agent status`.
///
/// `gpy-agent status` exits `0` only when the agent responded (#636), and
/// the report line is checked as well so an older agent binary on `PATH`
/// that still exits `0` unconditionally cannot make a stopped agent look
/// healthy.
fn check_agent_running() -> bool {
    match Command::new("gpy-agent").arg("status").output() {
        Ok(output) => {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(AGENT_RESPONDING_LINE)
        }
        Err(_) => false,
    }
}

/// The `gpy-agent status` report line that proves the agent answered.
const AGENT_RESPONDING_LINE: &str = "Status: Running and Responding";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    #[test]
    fn doctor_palette_validation_rejects_unknown() {
        assert!(crate::commands::palette::validate_target(Some("definitely-missing")).is_err());
    }
}
