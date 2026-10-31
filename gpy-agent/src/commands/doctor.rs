//! `gpy doctor`: agent, config, theme, segment, plugin and binary checks.

use super::doctor_bundled::{SETUP_REMEDIATION, is_uninstalled_bundled};
use super::doctor_ipc::check_ipc_client;
use super::doctor_themes::check_theme_overrides;
use super::utils::agent_binary;
use crate::agent::lifecycle::StatusProbe;
use crate::commands::output::{self, Status};
use crate::commands::validation::FailureKind;
use crate::plugin::discover_plugins;
use crate::{Error, Result, config};
use std::collections::{HashMap, HashSet};
use std::process::Command;

/// Command words that suggest a prompt segment spawns a slow process, with
/// the reason reported. `http` matches a URL scheme rather than a word.
const SLOW_PATTERN_HINTS: &[(&str, &str)] = &[
    ("curl", "network call"),
    ("wget", "network call"),
    ("http", "network call"),
    ("kubectl", "external tool invocation"),
    ("docker", "external tool invocation"),
    ("git", "subprocess git call"),
    ("python", "interpreter spawn"),
    ("python3", "interpreter spawn"),
    ("node", "interpreter spawn"),
];

/// Run diagnostics on the GPY installation and configuration.
///
/// # Errors
/// Returns an error if any critical health check fails.
pub fn run() -> Result<()> {
    output::heading("GPY Doctor - System Health Report");

    let mut all_ok = true;

    let loaded_config = config::loader::load_config();
    all_ok &= check_agent_status(loaded_config.as_ref().ok());
    output::blank();

    let discovery = discover_plugins();
    all_ok &= check_config_and_theme(&loaded_config, &discovery);
    check_theme_overrides(loaded_config.as_ref().ok().map(|cfg| cfg.ui.theme.as_str()));
    output::blank();

    all_ok &= check_plugins(&discovery);
    output::blank();

    check_plugin_performance(&loaded_config, &discovery);
    output::blank();

    all_ok &= check_binaries();
    let installed = super::setup::installed_integrations();
    check_ipc_client(&installed, loaded_config.as_ref().ok());
    check_shell_integrations(&installed);

    output::blank();
    output::plain("Summary:");
    if all_ok {
        output::ok("All checks passed.");
        Ok(())
    } else {
        output::fail("Issues were found. Please review the remediation hints above.");
        Err(Error::HealthCheckFailed)
    }
}

fn check_agent_status(cfg: Option<&config::Config>) -> bool {
    output::section("Agent");
    output::step("Checking process");
    if cfg.is_some_and(|loaded| !loaded.agent.enabled) {
        output::plain("Disabled via config (agent.enabled = false)");
        return true;
    }
    match crate::paths::get_socket_path() {
        Ok(socket_path) => {
            let probe = crate::agent::lifecycle::probe_status(&socket_path);
            report_agent_probe(probe, &socket_path)
        }
        Err(err) => {
            output::fail(format_args!("Cannot resolve the agent socket: {err}"));
            false
        }
    }
}

fn check_active_palette(palette_name: &str) -> bool {
    output::step("Checking active palette");
    match crate::commands::palette::validate_by_name(palette_name) {
        Ok(report) => {
            output::ok(format_args!(
                "Loaded successfully ('{}' from {})",
                report.target, report.source
            ));
            true
        }
        Err(failure) => {
            output::fail("Validation failed");
            crate::commands::palette::print_palette_validation_error("palette check", &failure);
            false
        }
    }
}

/// Warn about keys in `path` that `Config` ignores. A file that cannot be
/// read or parsed is reported as such.
fn print_unknown_config_keys(path: &std::path::Path) {
    let raw = match std::fs::read_to_string(path) {
        Ok(text) => match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(err) => {
                output::detail_status(
                    1,
                    Status::Warn,
                    format_args!("Cannot check {} for unknown keys: {err}", path.display()),
                );
                return;
            }
        },
        Err(err) => {
            output::detail_status(
                1,
                Status::Warn,
                format_args!("Cannot check {} for unknown keys: {err}", path.display()),
            );
            return;
        }
    };
    for key in config::loader::unknown_config_keys(&raw) {
        output::detail_status(
            1,
            Status::Warn,
            format_args!("Unknown key in {}: {key} (ignored)", path.display()),
        );
    }
}

fn check_config_and_theme(
    loaded_config: &Result<config::Config>,
    discovery: &crate::plugin::PluginDiscovery,
) -> bool {
    output::section("Configuration");
    output::step("Checking config file");
    let active = config::schema::resolve_active_config(&config::schema::get_config_paths());
    let cfg = match loaded_config {
        Ok(cfg) => {
            output::ok("Valid");
            if let Some((_, Some(existing))) = &active {
                print_unknown_config_keys(existing);
            }
            cfg
        }
        Err(e) => {
            output::fail("Invalid");
            output::detail(format_args!("Diagnostic: {e}"));
            let write_path = active.as_ref().map_or_else(
                || "~/.config/gpy/config.toml".to_owned(),
                |(write_path, _)| write_path.display().to_string(),
            );
            output::detail(format_args!(
                "Remediation: Fix the syntax error in {write_path}"
            ));
            return false;
        }
    };

    output::step("Checking active theme");
    let theme_name = cfg.ui.theme.as_str();
    // Loading the theme also validates its segment templates.
    let theme_result = crate::commands::theme::validate_by_name(theme_name);
    let theme_ok = match &theme_result {
        Ok(report) => {
            output::ok(format_args!(
                "Loaded successfully ('{}' from {})",
                report.target, report.source
            ));
            true
        }
        Err(failure) => {
            output::fail("Validation failed");
            crate::commands::theme::print_theme_validation_error("theme check", failure);
            false
        }
    };

    let palette_ok = check_active_palette(cfg.ui.palette.as_str());

    output::step("Checking segment templates");
    let templates_ok = match &theme_result {
        Ok(_) => {
            output::ok("Valid");
            true
        }
        Err(failure) if failure.kind() == FailureKind::Template => {
            output::fail("Invalid template");
            output::detail(format_args!("Diagnostic: {failure}"));
            output::detail("Remediation: Fix the segment `format` template named above.");
            false
        }
        Err(_) => {
            output::plain("Skipped (theme failed to load)");
            true
        }
    };

    output::step("Checking enabled segments");
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
        output::ok("All segments recognized");
        true
    } else {
        output::fail("Unrecognized segments found");
        let bundled_dir = crate::plugin::bundled_plugins_dir();
        let (uninstalled, unknown): (Vec<_>, Vec<_>) = invalid
            .into_iter()
            .partition(|seg| is_uninstalled_bundled(seg, &bundled_dir));
        for seg in uninstalled.iter().chain(&unknown) {
            output::detail_at(2, format_args!("- '{seg}'"));
        }
        if !uninstalled.is_empty() {
            output::detail(SETUP_REMEDIATION);
        }
        if !unknown.is_empty() {
            output::detail(
                "Remediation: Check for typos in `enabled_segments` or ensure the providing plugin is installed.",
            );
        }
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
    output::section("Plugins");
    if discovery.plugins.is_empty() && discovery.diagnostics.is_empty() {
        output::detail_status(1, Status::Ok, "No plugins discovered");
        return true;
    }

    let mut all_plugins_ok = true;

    if !discovery.plugins.is_empty() {
        output::detail("Discovered Plugins:");
        for plugin in &discovery.plugins {
            output::detail_status(
                2,
                Status::Ok,
                format_args!(
                    "{} {} [{}]",
                    plugin.manifest.id, plugin.manifest.version, plugin.source
                ),
            );
        }
    }

    if !discovery.diagnostics.is_empty() {
        all_plugins_ok = false;
        output::detail("Plugin Issues:");
        for diag in &discovery.diagnostics {
            output::detail_status(
                2,
                Status::Fail,
                format_args!("{} [{}]", diag.plugin_dir.display(), diag.source),
            );
            output::detail_at(3, format_args!("Error: {}", diag.message));
            if is_shared_writable(diag) {
                let dropped = if matches!(diag.source, crate::plugin::PluginSource::Bundled) {
                    "this bundled plugin is dropped, so its segments never render"
                } else {
                    "this plugin is dropped, so its segments never render"
                };
                output::detail_at(
                    3,
                    format_args!(
                        "Warning: {dropped}; run `chmod -R go-w {}` (or reinstall with `gpy setup`)",
                        diag.plugin_dir.display()
                    ),
                );
            }
        }
        output::detail(
            "Remediation: Review the plugin manifests (plugin.toml) for the reported errors.",
        );
    }

    all_plugins_ok
}

/// Whether a diagnostic comes from the ownership check that refuses a script
/// or directory others can write (see `check_owned_and_not_shared`).
fn is_shared_writable(diag: &crate::plugin::PluginDiagnostic) -> bool {
    diag.message.contains("group- or world-writable")
}

fn check_plugin_performance(
    loaded_config: &Result<config::Config>,
    discovery: &crate::plugin::PluginDiscovery,
) {
    output::section("Plugin Performance");

    let cfg = match loaded_config {
        Ok(cfg) => cfg,
        Err(err) => {
            output::detail(format_args!("Skipped: config unavailable ({err})."));
            return;
        }
    };

    let enabled_plugin_segments = enabled_plugin_segments(cfg, discovery);

    if enabled_plugin_segments.is_empty() {
        output::detail("No enabled community plugin segments detected.");
        return;
    }

    output::detail("Enabled community plugin segments:");
    for (segment, plugin) in enabled_plugin_segments {
        print_plugin_performance_hint(segment, plugin, cfg);
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

/// The file whose contents `segment` runs: the `.fish` file of a `file`
/// plugin, or the script a `script` plugin's segment manifest names.
fn plugin_segment_file(
    segment: &str,
    plugin: &crate::plugin::DiscoveredPlugin,
) -> std::result::Result<std::path::PathBuf, String> {
    let root = plugin
        .root
        .as_deref()
        .ok_or_else(|| "built-in segments have no file".to_owned())?;
    match plugin.manifest.entry_type {
        crate::plugin::EntryType::File => Ok(root.join("segments").join(format!("{segment}.fish"))),
        crate::plugin::EntryType::Script => {
            let manifest = plugin
                .script_segments
                .iter()
                .find_map(|(name, manifest)| (name.as_str() == segment).then_some(manifest))
                .ok_or_else(|| format!("no segment manifest for '{segment}'"))?;
            manifest
                .command
                .resolve(root)
                .map_err(|err| err.to_string())
        }
        crate::plugin::EntryType::Command => {
            Err("command plugins have no inspectable script".to_owned())
        }
    }
}

fn print_plugin_performance_hint(
    segment: &str,
    plugin: &crate::plugin::DiscoveredPlugin,
    cfg: &config::Config,
) {
    let segment_path = match plugin_segment_file(segment, plugin) {
        Ok(path) => path,
        Err(reason) => {
            output::detail_at(
                2,
                format_args!("- {segment} (plugin: {})", plugin.manifest.id),
            );
            output::detail_at(
                3,
                format_args!("heuristic: unable to inspect file ({reason})"),
            );
            return;
        }
    };
    output::detail_at(
        2,
        format_args!(
            "- {segment} (plugin: {}, file: {})",
            plugin.manifest.id,
            segment_path.display()
        ),
    );

    match std::fs::read_to_string(&segment_path) {
        Ok(content) => print_suspect_patterns(segment, &content),
        Err(error) => {
            output::detail_at(
                3,
                format_args!("heuristic: unable to inspect file ({error})"),
            );
        }
    }
    if matches!(plugin.manifest.entry_type, crate::plugin::EntryType::Script) {
        print_script_trial(segment, plugin, &segment_path, cfg);
    }
}

/// Run a script segment once with the runner's rules and report the time.
fn print_script_trial(
    segment: &str,
    plugin: &crate::plugin::DiscoveredPlugin,
    script: &std::path::Path,
    cfg: &config::Config,
) {
    use super::script_run::{Outcome, run_trial};
    use crate::plugin::SegmentName;

    let Some(root) = plugin.root.as_deref() else {
        return;
    };
    let Ok(name) = SegmentName::new(segment) else {
        return;
    };
    let Some(manifest) = plugin.script_segments.get(&name) else {
        return;
    };
    let resolved = match config::plugin_segments::resolve(&name, manifest, cfg.plugin.get(&name)) {
        Ok(resolved) => resolved,
        Err(error) => {
            output::detail_status(3, Status::Fail, format_args!("trial: {error}"));
            return;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| root.to_path_buf());
    let timeout_ms = resolved.timeout_ms.get();
    let trial = match run_trial(script, root, manifest, &resolved, &cwd, cfg.ui.show_icons) {
        Ok(trial) => trial,
        Err(error) => {
            output::detail_status(3, Status::Fail, format_args!("trial: cannot run ({error})"));
            return;
        }
    };
    let millis = trial.elapsed.as_millis();
    let (status, message) = match &trial.outcome {
        Outcome::Shown { text, variant } => (
            Status::Ok,
            format!(
                "trial: ran in {millis} ms (timeout_ms {timeout_ms}); shows {:?}{}",
                crate::template::sanitize_control_chars(text),
                variant
                    .as_ref()
                    .map_or_else(String::new, |v| format!(" with variant {v}"))
            ),
        ),
        Outcome::Hidden(reason) => (
            Status::Warn,
            format!(
                "trial: ran in {millis} ms (timeout_ms {timeout_ms}); segment hidden: {reason}"
            ),
        ),
        Outcome::TimedOut => (
            Status::Fail,
            format!("trial: killed after timeout_ms {timeout_ms}; the segment is hidden"),
        ),
    };
    output::detail_status(3, status, message);
    if matches!(trial.outcome, Outcome::Shown { .. })
        && trial.elapsed.as_millis().saturating_mul(2) > u128::from(timeout_ms)
    {
        output::detail_at(
            3,
            "hint: more than half of timeout_ms; speed the script up or lower its cost",
        );
    }
}

/// Reasons the command words of `content` suggest a slow prompt segment.
/// Comment lines and trailing comments are ignored, and a pattern matches a
/// whole word (or a URL scheme), not a substring of an identifier.
fn slow_pattern_reasons(content: &str) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    for line in content.lines() {
        let code = line
            .split_once(" #")
            .map_or(line, |(code, _comment)| code)
            .trim_start();
        if code.starts_with('#') {
            continue;
        }
        for word in code
            .split(|c: char| {
                c.is_whitespace()
                    || matches!(c, ';' | '|' | '&' | '(' | ')' | '`' | '"' | '\'' | '$')
            })
            .filter(|word| !word.is_empty())
        {
            for (pattern, reason) in SLOW_PATTERN_HINTS {
                let hit = if *pattern == "http" {
                    word.starts_with("http://") || word.starts_with("https://")
                } else {
                    word == *pattern
                };
                if hit && !reasons.contains(reason) {
                    reasons.push(*reason);
                }
            }
        }
    }
    reasons
}

fn print_suspect_patterns(segment: &str, content: &str) {
    let suspects = slow_pattern_reasons(content);
    if suspects.is_empty() {
        output::detail_at(3, "heuristic: no obvious slow patterns detected");
    } else {
        output::detail_at(
            3,
            format_args!(
                "heuristic: possible slowdown source ({})",
                suspects.join(", ")
            ),
        );
        output::detail_at(
            3,
            format_args!(
                "hint: if your prompt feels slow, disable `{segment}` first to confirm whether this plugin segment is responsible"
            ),
        );
    }
}

/// Warn about installed shell scripts whose version stamp differs from this
/// `gpy`: `gpy setup` for one shell leaves the other shells' scripts as they
/// were. Not a failure, since they still work against the same protocol.
fn check_shell_integrations(installed: &[super::setup::InstalledIntegration]) {
    for integration in installed {
        let shell = integration.shell;
        output::step(format_args!("Checking {shell} integration"));
        match integration.version.as_deref() {
            Some(version) if version == crate::VERSION => {
                output::ok(format_args!("{version} ({})", integration.dir.display()));
            }
            found => {
                let from = found.map_or_else(
                    || "an older gpy".to_owned(),
                    |version| format!("gpy {version}"),
                );
                output::warn(format_args!(
                    "installed by {from}, but this is gpy {} ({})",
                    crate::VERSION,
                    integration.dir.display()
                ));
                output::detail(format_args!(
                    "Remediation: run `gpy setup --shell {shell}` to install the current scripts."
                ));
            }
        }
    }
}

fn check_binaries() -> bool {
    output::section("System");
    output::step("Checking gpy-agent binary");
    let agent = agent_binary();
    match Command::new(&agent).arg("--version").output() {
        Ok(probe) if probe.status.success() => {
            output::ok(format_args!("Found ({})", agent.display()));
            true
        }
        Ok(probe) => {
            output::fail(format_args!(
                "Found ({}), but `gpy-agent --version` failed ({})",
                agent.display(),
                probe.status
            ));
            let stderr = String::from_utf8_lossy(&probe.stderr);
            if let Some(line) = stderr.lines().map(str::trim).find(|line| !line.is_empty()) {
                output::detail(format_args!("Diagnostic: {line}"));
            }
            output::detail(
                "Remediation: Reinstall gpy-agent, and check that the first gpy-agent in your PATH (`command -v gpy-agent`) is GPY's agent.",
            );
            false
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            output::fail("Not found in PATH");
            output::detail(
                "Remediation: Ensure gpy-agent is installed and your PATH includes its location.",
            );
            false
        }
        Err(err) => {
            output::fail(format_args!("Could not run gpy-agent: {err}"));
            output::detail(
                "Remediation: Reinstall gpy-agent, and check that the first gpy-agent in your PATH (`command -v gpy-agent`) is executable.",
            );
            false
        }
    }
}

/// The agent check's verdict from a probe of its socket. A refused or absent
/// socket and a load-shedding agent only warn; an agent that holds its socket
/// without answering is wedged, and every shell prompt would time out on it.
fn report_agent_probe(probe: StatusProbe, socket_path: &std::path::Path) -> bool {
    match probe {
        StatusProbe::Answered(_) => {
            output::ok("Running");
            true
        }
        StatusProbe::NotListening(_) => {
            output::warn("Not running");
            output::detail("Hint: Start the agent with `gpy start` or just use your prompt.");
            true
        }
        StatusProbe::Busy => {
            output::warn("Running but too busy to answer");
            true
        }
        StatusProbe::NoReply(err) => {
            output::fail(format_args!("Not answering on {}", socket_path.display()));
            output::detail(format_args!("Diagnostic: {err}"));
            output::detail("Remediation: run `gpy restart`.");
            false
        }
        StatusProbe::Failed(err) => {
            output::warn(format_args!("Could not check the agent: {err}"));
            true
        }
    }
}

#[cfg(test)]
mod tests {

    use super::{check_segments, slow_pattern_reasons};

    #[test]
    fn check_segments_accepts_builtins_and_rejects_unknown_names() {
        let discovery = crate::plugin::PluginDiscovery::default();
        let builtins = vec!["git".to_owned(), "directory".to_owned()];
        assert!(check_segments(&builtins, &discovery));

        let with_typo = vec!["git".to_owned(), "gti".to_owned()];
        assert!(!check_segments(&with_typo, &discovery));
    }

    #[test]
    fn slow_patterns_ignore_comments_and_identifiers() {
        let script = "# no network here: curl, wget and http are not used\n\
                      node_count=3 # docker is not called\n\
                      echo \"$node_count\"\n";
        assert_eq!(slow_pattern_reasons(script), Vec::<&str>::new());
    }

    #[test]
    fn slow_patterns_flag_command_words_once_per_reason() {
        let script = "out=$(curl -s https://example.com | wget -q -)\n\
                      git status; python3 -c 'pass'\n";
        assert_eq!(
            slow_pattern_reasons(script),
            vec!["network call", "subprocess git call", "interpreter spawn"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_script_diagnostic_is_recognised() {
        use super::is_shared_writable;
        use crate::plugin::{PluginDiagnostic, PluginSource, ScriptSegmentManifest, SegmentName};
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new().expect("tempdir");
        let plugin = temp.path().join("demo");
        std::fs::create_dir_all(plugin.join("segments")).expect("segments");
        std::fs::write(plugin.join("segments/demo.toml"), "command = \"run.sh\"\n")
            .expect("segment");
        std::fs::write(plugin.join("run.sh"), "echo hi\n").expect("script");
        let mode = |path: &std::path::Path, bits| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).expect("chmod");
        };
        for dir in [temp.path(), &plugin, &plugin.join("segments")] {
            mode(dir, 0o755);
        }
        let segment = SegmentName::new("demo").expect("segment name");
        let diagnostic = |error: crate::Error| PluginDiagnostic {
            plugin_dir: plugin.clone(),
            source: PluginSource::Bundled,
            message: error.to_string(),
        };

        mode(&plugin.join("run.sh"), 0o644);
        ScriptSegmentManifest::load(&plugin, &segment).expect("private script loads");

        mode(&plugin.join("run.sh"), 0o664);
        let error = ScriptSegmentManifest::load(&plugin, &segment).expect_err("shared script");
        assert!(is_shared_writable(&diagnostic(error)));

        let other = PluginDiagnostic {
            plugin_dir: plugin.clone(),
            source: PluginSource::Bundled,
            message: "missing field `id`".to_owned(),
        };
        assert!(!is_shared_writable(&other));
    }
}
