//! `gpy theme new` and `gpy theme save` (ADR-0012 section 5, "Writers").
//!
//! Both write an overlay by default, `extends = "<base>"` and nothing else,
//! so the new theme keeps receiving every change to its base. With `--full`
//! they write the effective theme as a full file instead: the base's raw
//! source when it is a full theme, the merged result when it is an overlay.
//!
//! Every write, here and by `gpy config wizard`, is first loaded as the file
//! it will become ([`check_write`]); when that fails nothing is written.

use super::output;
use super::theme_rebase::{header_only, message_of};
use super::utils::read_config;
use crate::config::discovery::{Source, validate_config_name};
use crate::config::loader::load_named_theme;
use crate::theme::user_copy::{self, builtin_content};
use crate::theme::{ChainLink, LoadedTheme, ThemeChain, ThemeConfig, normalised_table, overlay};
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// The base of `gpy theme new` without `--from`.
const DEFAULT_BASE: &str = "default";

/// Create the user theme `name`: an overlay of `from` (default `default`), or
/// with `full` a full copy of that theme's effective content.
///
/// # Errors
/// Returns an error on an unsafe name, an existing theme file `name`, a base
/// that does not resolve or load, a new file that would not load, or a
/// failed write.
pub fn new(name: &str, from: Option<&str>, full: bool) -> Result<()> {
    validate_config_name("theme", name)?;
    let base = from.unwrap_or(DEFAULT_BASE);
    validate_config_name("theme", base)?;
    if user_copy::user_copy_path(name).exists() {
        return Err(already_exists(name));
    }

    let skeleton = header_only(base);
    let as_overlay = check_write(name, skeleton.clone())?;
    let (text, written, copied_from) = if full {
        // The base's own chain: the new file's chain without the new file.
        let base_chain: Vec<&ChainLink> = as_overlay.chain.iter().skip(1).collect();
        let Some(base_link) = base_chain.first() else {
            return Err(Error::config(format!(
                "theme '{name}' resolved no base for '{base}'"
            )));
        };
        let text = full_text(&base_chain, &as_overlay.theme)?;
        let written = check_write(name, text.clone())?;
        (text, written, Some(base_link.to_string()))
    } else {
        (skeleton, as_overlay, None)
    };

    write_checked(&written, &text, false)?;
    let path = &written.chain.top().path;
    output::plain(format!("Created new theme: {}", path.display()));
    describe(&written, copied_from.as_deref());
    output::plain(format!("Edit: {}", path.display()));
    output::plain(format!("Activate with: gpy theme use {name}"));
    Ok(())
}

/// Save the active theme as the user theme `name`: an overlay of it, or with
/// `full` a full copy of its effective content. Without `name`, or with the
/// active theme's own name, a builtin or plugin theme gets a self-named
/// overlay; a theme that already is a user file is left alone.
///
/// Overwriting an existing file, or omitting `name`, prompts for
/// confirmation unless `yes`; an overwritten file is kept as `.bak`.
///
/// # Errors
/// Returns an error on an unsafe name, an active theme that does not load,
/// a new file that would not load, prompt failure, or write failure.
pub fn save(name: Option<&str>, full: bool, yes: bool) -> Result<()> {
    let config = read_config()?;
    let active = load_named_theme(config.ui.theme.as_str())?;
    let top = active.chain.top();
    let own = top.name.as_str();
    let target = name.unwrap_or(own);
    validate_config_name("theme", target)?;

    if top.source == Source::User && (target == own || same_file(target, &top.path)) {
        output::plain(format!(
            "Theme '{own}' already is your theme file {}; nothing was written.",
            top.path.display()
        ));
        output::plain(format!(
            "Edit it directly, or save it under another name: gpy theme save <name>{}",
            if full { " --full" } else { "" }
        ));
        return Ok(());
    }

    let text = if full {
        let active_chain: Vec<&ChainLink> = active.chain.iter().collect();
        full_text(&active_chain, &active.theme)?
    } else {
        header_only(own)
    };
    let written = check_write(target, text.clone())?;
    let path = written.chain.top().path.clone();
    let exists = written.chain.top().on_disk;

    if !yes && save_requires_confirmation(name, exists) {
        let prompt = if exists {
            format!("Theme '{target}' already exists. Overwrite? (a copy is kept as .bak) [y/N] ")
        } else {
            format!(
                "Save the {} theme '{own}' as {} {}? [y/N] ",
                top.source,
                if full {
                    "a full copy in"
                } else {
                    "an overlay in"
                },
                path.display()
            )
        };
        match confirm(&prompt)? {
            Answer::Yes => {}
            Answer::No => {
                return Err(Error::invalid("theme save cancelled; nothing was written"));
            }
            Answer::Eof => {
                return Err(Error::invalid(format!(
                    "theme save needs confirmation to write {}; pass --yes",
                    path.display()
                )));
            }
        }
    }

    let backup = write_checked(&written, &text, true)?;
    output::plain(format!("Saved theme: {}", path.display()));
    if let Some(copy) = backup {
        output::plain(format!("Previous file kept at {}", copy.display()));
    }
    describe(&written, full.then(|| top.to_string()).as_deref());
    output::plain(format!("Edit: {}", path.display()));
    if target != own {
        output::plain(format!("Activate with: gpy theme use {target}"));
    }
    Ok(())
}

/// Load `text` as the user theme file `name` will be once written: an
/// existing file is replaced by `text`, a new one resolves like a user file
/// named `name`.
///
/// # Errors
/// Returns an error naming `name` when the result would not load, including
/// an `extends` cycle the new file would close.
pub(crate) fn check_write(name: &str, text: String) -> Result<LoadedTheme> {
    let path = user_copy::user_copy_path(name);
    let link = if path.exists() {
        overlay::own_user_link(name)?
    } else {
        ChainLink {
            name: name.to_owned(),
            source: Source::User,
            path,
            on_disk: false,
        }
    };
    let existing = link.on_disk;
    let loaded = overlay::load_text(link, text)
        .map_err(|error| would_not_load(name, &message_of(&error)))?;
    if !existing && let Some(cycle) = cycle_once_written(&loaded.chain, name) {
        return Err(would_not_load(name, &cycle));
    }
    Ok(loaded)
}

/// Write `text`, which [`check_write`] loaded as `loaded`, to its file: a new
/// file is created, an existing one replaced atomically, first copied aside
/// when `keep_copy`. Returns the copy's path.
///
/// # Errors
/// Returns an error if the directory, the copy or the file can't be written.
pub(crate) fn write_checked(
    loaded: &LoadedTheme,
    text: &str,
    keep_copy: bool,
) -> Result<Option<PathBuf>> {
    let top = loaded.chain.top();
    let label = top.path.display().to_string();
    if top.on_disk {
        let backup = if keep_copy {
            Some(user_copy::copy_aside(&top.path)?)
        } else {
            None
        };
        crate::fs_util::write_atomically(&top.path, text.as_bytes())
            .map_err(|e| Error::io_context(format_args!("write {label}"), &e))?;
        return Ok(backup);
    }
    if let Some(dir) = top.path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::config(format!("failed to create themes directory: {e}")))?;
    }
    crate::fs_util::create_new(&top.path, text.as_bytes()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            already_exists(&top.name)
        } else {
            Error::io_context(format_args!("write {label}"), &e)
        }
    })?;
    Ok(None)
}

/// One line per plugin whose theme `chain` extends: the theme stops loading
/// when that plugin is removed or disabled.
pub(crate) fn plugin_dependencies(chain: &ThemeChain) -> Vec<String> {
    let mut plugins: Vec<&str> = Vec::new();
    for link in chain.iter().skip(1) {
        if let Source::Plugin { plugin_id } = &link.source
            && !plugins.contains(&plugin_id.as_str())
        {
            plugins.push(plugin_id);
        }
    }
    plugins
        .into_iter()
        .map(|plugin| {
            format!(
                "Depends on plugin '{plugin}': if it is removed or disabled, this theme no longer loads (--full writes an independent copy)."
            )
        })
        .collect()
}

/// The theme whose chain is `chain` (the theme first) as a full file: its raw
/// source when it is a full theme, else its merged result `theme`, normalised
/// and serialized.
///
/// # Errors
/// Returns an error if the source can't be read or the theme serialized.
fn full_text(chain: &[&ChainLink], theme: &ThemeConfig) -> Result<String> {
    if let [full] = chain {
        return raw_source(full);
    }
    let body = toml::to_string_pretty(&normalised_table(theme)?)
        .map_err(|e| Error::config(format!("failed to serialize theme to TOML: {e}")))?;
    let merged_from: Vec<String> = chain.iter().map(ToString::to_string).collect();
    Ok(format!(
        "# Merged from {}, written as a full theme by --full.\n\n{body}",
        merged_from.join(" → ")
    ))
}

/// The text of the full theme file `link`, or of the embedded builtin.
///
/// # Errors
/// Returns an error if the file can't be read or no builtin matches.
fn raw_source(link: &ChainLink) -> Result<String> {
    if link.on_disk {
        return std::fs::read_to_string(&link.path)
            .map_err(|e| Error::io_context(format_args!("read {}", link.path.display()), &e));
    }
    builtin_content(&link.name)
        .map(str::to_owned)
        .ok_or_else(|| Error::config(format!("Unknown builtin theme: {}", link.name)))
}

/// Print what the written theme is built from: its base and the plugins it
/// depends on, or, for a full copy, the theme it was copied from.
fn describe(written: &LoadedTheme, copied_from: Option<&str>) {
    if let Some(from) = copied_from {
        output::plain(format!(
            "A full copy of {from}: it does not receive later changes to that theme."
        ));
        return;
    }
    if let Some(base) = written.chain.iter().nth(1) {
        output::plain(format!("Extends: {base}"));
    }
    for note in plugin_dependencies(&written.chain) {
        output::plain(note);
    }
}

/// Where a new user file `name` would close an `extends` cycle: a user file
/// further down `chain` that resolves `name` below the user source today
/// would resolve it to the new file once it exists.
fn cycle_once_written(chain: &ThemeChain, name: &str) -> Option<String> {
    let links: Vec<&ChainLink> = chain.iter().collect();
    let at = links.windows(2).skip(1).position(
        |pair| matches!(pair, [from, to] if from.source == Source::User && to.name == name),
    )?;
    let path: Vec<String> = links
        .iter()
        .take(at.saturating_add(2))
        .map(ToString::to_string)
        .collect();
    Some(format!(
        "`extends` cycle once the file exists: {} → {name} (user)",
        path.join(" → ")
    ))
}

/// Whether the user theme file `name` exists and is the file at `path`.
fn same_file(name: &str, path: &Path) -> bool {
    let target = user_copy::user_copy_path(name);
    match (std::fs::canonicalize(target), std::fs::canonicalize(path)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn already_exists(name: &str) -> Error {
    Error::config(format!("theme '{name}' already exists"))
}

fn would_not_load(name: &str, reason: &str) -> Error {
    Error::config(format!(
        "Theme '{name}' would not load, so nothing was written\n  \
         Diagnostic: {reason}\n  \
         Remediation: Pick another base or name (see `gpy theme list`)."
    ))
}

/// Omitting `name` always writes over the theme in use, so it always
/// confirms, even if the active theme is an unmaterialized builtin.
const fn save_requires_confirmation(name: Option<&str>, target_exists: bool) -> bool {
    name.is_none() || target_exists
}

/// What the user said to a yes/no question.
enum Answer {
    Yes,
    No,
    /// Input ended before any answer.
    Eof,
}

fn confirm(prompt: &str) -> Result<Answer> {
    use std::io::Write;

    print!("{prompt}");
    std::io::stdout()
        .flush()
        .map_err(|e| Error::config(format!("failed to write prompt: {e}")))?;

    let mut answer = String::new();
    let read = std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| Error::config(format!("failed to read confirmation: {e}")))?;

    Ok(if read == 0 {
        Answer::Eof
    } else if parse_confirmation(&answer) {
        Answer::Yes
    } else {
        Answer::No
    })
}

/// Only (case-insensitive) `y`/`yes` confirm.
fn parse_confirmation(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::{cycle_once_written, parse_confirmation, save_requires_confirmation};
    use crate::config::discovery::Source;
    use crate::theme::overlay::load_text;
    use crate::theme::{ChainLink, user_copy};

    #[test]
    fn no_confirmation_needed_for_new_named_theme() {
        assert!(!save_requires_confirmation(Some("mytheme"), false));
    }

    #[test]
    fn confirmation_needed_when_named_theme_already_exists() {
        assert!(save_requires_confirmation(Some("mytheme"), true));
    }

    #[test]
    fn confirmation_always_needed_when_name_omitted() {
        assert!(save_requires_confirmation(None, false));
        assert!(save_requires_confirmation(None, true));
    }

    #[test]
    fn parses_yes_variants_as_confirmed() {
        assert!(parse_confirmation("y"));
        assert!(parse_confirmation("Y"));
        assert!(parse_confirmation("yes"));
        assert!(parse_confirmation("YES"));
        assert!(parse_confirmation("  y  \n"));
    }

    #[test]
    fn parses_everything_else_as_declined() {
        assert!(!parse_confirmation("n"));
        assert!(!parse_confirmation(""));
        assert!(!parse_confirmation("\n"));
        assert!(!parse_confirmation("maybe"));
    }

    /// A self-named overlay resolves below its own name, never to itself.
    #[test]
    fn a_self_named_overlay_closes_no_cycle() {
        let link = ChainLink {
            name: "starship".to_owned(),
            source: Source::User,
            path: user_copy::user_copy_path("starship"),
            on_disk: false,
        };
        let loaded = load_text(link, "extends = \"starship\"\n".to_owned()).expect("overlay loads");
        assert_eq!(cycle_once_written(&loaded.chain, "starship"), None);
    }
}
