//! `gpy debug paths` — dump every path GPY resolves from the environment.
//!
//! This is the Rust half of the cross-shell path parity contract (#476). Path
//! rules are implemented four times — here, in Fish, in Bash and in Zsh — for
//! two roots (runtime and cache). They agree today only by convention. The
//! parity harness (`tests/fish/path_parity.test.fish`) runs this command and
//! each shell's `__gpy_debug_paths` under the same synthetic environments and
//! diffs the full maps, so #477 can centralize resolution and prove it changed
//! nothing on Unix.
//!
//! Every value here comes from the resolver production actually calls. Nothing
//! in this module re-derives a precedence rule: a reimplementation would agree
//! with itself and make the whole harness worthless. Where a resolver is not
//! public, the value is derived from a public one that is built on it (see
//! `instant_prompts_dir`), never re-expressed.
//!
//! Note that [`crate::agent::lifecycle::get_runtime_dir`] materializes the
//! directory it returns, so running this command has the same filesystem side
//! effect as any other command that resolves the runtime root.

use std::path::Path;

/// Emitted when a resolver ran but produced no path (unset environment, or an
/// error return). Shell implementations emit the same token.
pub const UNRESOLVED: &str = "<unresolved>";

/// Separator for list-valued keys (`config_candidates`).
///
/// `:` and `,` occur in real paths often enough to be ambiguous; `|` is legal
/// in a POSIX path but appears in none that GPY constructs.
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

/// Render an optional path as a string, falling back to [`UNRESOLVED`].
///
/// An empty path is [`UNRESOLVED`] too: `GPY_AGENT_SOCKET_PATH=` resolves to
/// `PathBuf::from("")`, which is a resolver producing nothing rather than a
/// path, and is what every shell reports for the same input.
fn show(path: Option<&Path>) -> String {
    let rendered = path.map(|p| p.to_string_lossy().into_owned());
    match rendered {
        Some(value) if !value.is_empty() => value,
        _ => UNRESOLVED.to_owned(),
    }
}

/// Resolve every GPY path for the current process environment.
///
/// Returns `(key, value)` pairs in [`KEYS`] order. A key whose resolver fails
/// carries [`UNRESOLVED`] rather than being omitted, so the key set is stable
/// across environments and a missing key is always a bug rather than a
/// legitimate "not applicable".
#[must_use]
pub fn resolved_paths() -> Vec<(&'static str, String)> {
    let runtime_root = crate::agent::lifecycle::get_runtime_dir().ok();
    let socket = crate::agent::lifecycle::get_socket_path().ok();
    let shell_registry_dir = runtime_root.as_ref().map(|root| root.join("shells"));

    // `get_instant_cache_dir` is private; `cache_file_for_dir` is the public
    // entry point built on it, so the directory is taken from a real cache
    // file's parent rather than re-deriving the precedence here. The cache root
    // is that directory's parent, which is how `get_gpy_cache_dir` relates to
    // it in `cache::instant_prompt`.
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
    let config_candidates = crate::config::schema::get_config_paths().join(LIST_SEPARATOR);
    let theme_dir = crate::theme::ThemeManager::user_themes_dir();

    vec![
        ("runtime_root", show(runtime_root.as_deref())),
        ("socket", show(socket.as_deref())),
        ("shell_registry_dir", show(shell_registry_dir.as_deref())),
        ("cache_root", show(cache_root.as_deref())),
        ("instant_prompts_dir", show(instant_prompts_dir.as_deref())),
        ("theme_export_file", show(theme_export_file.as_deref())),
        (
            "config_path",
            config_path
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| UNRESOLVED.to_owned()),
        ),
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

/// Print the resolved path map.
///
/// `as_json` selects a JSON object (stable key order); otherwise `key=value`
/// lines, which is what the shell implementations emit and what the parity
/// harness diffs.
pub fn run(as_json: bool) {
    let pairs = resolved_paths();

    if as_json {
        // Preserve KEYS order: serde_json's default Map is a BTreeMap unless
        // `preserve_order` is on, so build the object text from the ordered
        // pairs and let serde_json handle only the string escaping.
        let body = pairs
            .iter()
            .map(|(key, value)| {
                format!(
                    "  {}: {}",
                    serde_json::Value::String((*key).to_owned()),
                    serde_json::Value::String(value.clone())
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        println!("{{\n{body}\n}}");
    } else {
        for (key, value) in pairs {
            println!("{key}={value}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
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
