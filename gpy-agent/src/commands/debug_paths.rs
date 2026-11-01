//! `gpy debug paths` — dump every path GPY resolves from the environment.
//!
//! Rust half of the cross-shell path parity harness
//! (`tests/fish/path_parity.test.fish`), which diffs this output against each
//! shell's `__gpy_debug_paths`. Every value MUST come from the resolver
//! production calls; re-deriving a precedence rule here would make the
//! harness compare an implementation with itself.

use super::output;
use std::path::Path;

/// Emitted when a resolver ran but produced no path (unset environment, or an
/// error return). Shell implementations emit the same token.
pub const UNRESOLVED: &str = "<unresolved>";

/// Separator for list-valued keys; `:` and `,` occur in real paths, `|` in
/// none that GPY constructs.
pub const LIST_SEPARATOR: &str = "|";

/// The ordered key set. Shell implementations emit these keys, in this order.
pub const KEYS: [&str; 9] = [
    "runtime_root",
    "socket",
    "shell_registry_dir",
    "cache_root",
    "instant_prompts_dir",
    "theme_export_file",
    "config_path",
    "config_candidates",
    "theme_dir",
];

/// Render an optional path, falling back to [`UNRESOLVED`].
///
/// Empty counts as unresolved (`GPY_AGENT_SOCKET_PATH=`), matching the shells.
fn show(path: Option<&Path>) -> String {
    let rendered = path.map(|p| p.to_string_lossy().into_owned());
    match rendered {
        Some(value) if !value.is_empty() => value,
        _ => UNRESOLVED.to_owned(),
    }
}

/// Resolve every GPY path for the current process environment.
///
/// Returns pairs in [`KEYS`] order; failures carry [`UNRESOLVED`] so the key
/// set is stable.
#[must_use]
pub fn resolved_paths() -> Vec<(&'static str, String)> {
    let runtime_root = Some(crate::paths::runtime_dir());
    // Unvalidated: this is the diagnostic for when `SocketPath::resolve` rejects the length.
    let socket = crate::paths::SocketPath::resolve_unchecked(None);
    let shell_registry_dir = runtime_root.as_ref().map(|root| root.join("shells"));

    // `get_instant_cache_dir` is private; derive it from a real cache file's
    // parent instead of re-deriving the precedence.
    let instant_prompt_file = crate::cache::InstantPromptCache::cache_file_for_dir(
        Path::new("/"),
        "git",
        None,
        crate::formatter::PromptDialect::Ansi,
    )
    .ok();
    let instant_prompts_dir = instant_prompt_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let cache_root = instant_prompts_dir
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);

    let theme_export_file = crate::cache::theme_export::get_theme_export_cache_path().ok();
    let config_path = super::utils::active_config_path().ok();
    let config_candidates = config_candidates().join(LIST_SEPARATOR);
    let theme_dir = crate::theme::ThemeManager::user_themes_dir();

    vec![
        ("runtime_root", show(runtime_root.as_deref())),
        ("socket", show(Some(socket.path()))),
        ("shell_registry_dir", show(shell_registry_dir.as_deref())),
        ("cache_root", show(cache_root.as_deref())),
        ("instant_prompts_dir", show(instant_prompts_dir.as_deref())),
        ("theme_export_file", show(theme_export_file.as_deref())),
        ("config_path", show(config_path.as_deref())),
        (
            "config_candidates",
            if config_candidates.is_empty() {
                UNRESOLVED.to_owned()
            } else {
                config_candidates
            },
        ),
        ("theme_dir", show(Some(theme_dir.as_path()))),
    ]
}

/// The config-file candidates, in resolution order.
fn config_candidates() -> Vec<String> {
    crate::config::schema::get_config_paths()
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

/// Print the resolved path map as JSON or as the `key=value` lines the
/// shells emit. JSON writes the list-valued `config_candidates` as an array;
/// `kv` joins it with [`LIST_SEPARATOR`].
pub fn run(as_json: bool) {
    let pairs = resolved_paths();

    if as_json {
        let candidates = serde_json::Value::Array(
            config_candidates()
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        );
        // serde_json's Map is a BTreeMap without `preserve_order`; build the
        // text from the ordered pairs to keep KEYS order.
        let body = pairs
            .iter()
            .map(|(key, value)| {
                let rendered = if *key == "config_candidates" {
                    candidates.to_string()
                } else {
                    serde_json::Value::String(value.clone()).to_string()
                };
                format!(
                    "  {}: {rendered}",
                    serde_json::Value::String((*key).to_owned())
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        output::plain(format_args!("{{\n{body}\n}}"));
    } else {
        for (key, value) in pairs {
            output::plain(format_args!("{key}={value}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{KEYS, resolved_paths};

    #[test]
    fn resolved_paths_emits_every_key_once_in_order() {
        let keys: Vec<&str> = resolved_paths().into_iter().map(|(key, _)| key).collect();
        assert_eq!(keys, KEYS.to_vec());
    }

    #[test]
    fn resolved_paths_never_emits_an_empty_value() {
        for (key, value) in resolved_paths() {
            assert!(!value.is_empty(), "{key} resolved to an empty string");
        }
    }
}
