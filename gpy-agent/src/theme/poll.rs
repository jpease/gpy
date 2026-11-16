//! The files the theme poll fallback fingerprints, re-resolved only when
//! what theme resolution reads has changed.

use super::ThemeManager;
use super::manager::{push_unique, resolve_top};
use super::overlay::{self, ThemeChain};
use crate::sync_ext::MutexExt;
use crate::watcher::hot_reload::content_fingerprint;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a poll tick resolves paths for: the active name, the name a fallback
/// stands in for (equal to `name` otherwise), and the chain last loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PollSubject {
    pub(super) name: String,
    pub(super) requested: String,
    pub(super) chain: ThemeChain,
}

/// Theme resolution's inputs besides the chain files, which the poll loop
/// fingerprints itself: the user themes dir listing and, for each directory
/// under each plugin root, its manifest's content and its `themes/` listing.
/// Ownership and mode changes, which discovery's trust check also reads, are
/// not covered; the watcher and a config reload still see them.
#[derive(Debug, PartialEq, Eq)]
struct ResolutionInputs(Vec<(PathBuf, Option<u64>)>);

impl ResolutionInputs {
    /// Read the inputs now: directory listings and manifest fingerprints
    /// only, no manifest parse or trust walk.
    fn read() -> Self {
        let mut inputs = Vec::new();
        if let Ok(dir) = ThemeManager::user_themes_dir() {
            push_toml_files(&dir, &mut inputs);
        }
        let bundled_plugins = crate::plugin::bundled_plugins_dir();
        let user_plugins = crate::plugin::user_plugins_dir().ok();
        for root in bundled_plugins.into_iter().chain(user_plugins) {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for dir in entries.flatten().map(|entry| entry.path()) {
                // Fingerprinted, not just listed: a manifest is created empty
                // and written after, and only the written one is discovered.
                let manifest = dir.join("plugin.toml");
                let fingerprint = content_fingerprint(&manifest);
                inputs.push((manifest, fingerprint));
                push_toml_files(&dir.join("themes"), &mut inputs);
            }
        }
        inputs.sort_unstable();
        Self(inputs)
    }
}

/// Append every `.toml` file directly in `dir`, unfingerprinted.
fn push_toml_files(dir: &Path, inputs: &mut Vec<(PathBuf, Option<u64>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    inputs.extend(
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension() == Some(OsStr::new("toml")))
            .map(|path| (path, None)),
    );
}

/// The poll fallback's path source: re-resolves the chain (plugin discovery
/// plus a full theme load) only when the subject or the resolution inputs
/// changed since the last tick, not on every tick.
#[derive(Default)]
pub(super) struct PollPaths {
    memo: Mutex<Option<(PollSubject, ResolutionInputs, Vec<PathBuf>)>>,
}

impl PollPaths {
    /// The files to fingerprint for `subject` this tick.
    pub(super) fn get(&self, subject: PollSubject) -> Vec<PathBuf> {
        let inputs = ResolutionInputs::read();
        let mut memo = self.memo.lock_or_recover();
        if let Some((known_subject, known_inputs, paths)) = memo.as_ref()
            && *known_subject == subject
            && *known_inputs == inputs
        {
            return paths.clone();
        }
        let paths = resolve_paths(&subject);
        *memo = Some((subject, inputs, paths.clone()));
        drop(memo);
        paths
    }
}

/// The files to fingerprint for `subject`, re-resolving its chain now. A
/// chain that no longer loads is polled at the file its name resolves to and
/// at the files of the chain last loaded, so fixing any of them reloads; so is
/// the file a requested name a fallback stands in for resolves to.
fn resolve_paths(subject: &PollSubject) -> Vec<PathBuf> {
    let top = resolve_top(&subject.name);
    let top_path = top.path.clone();
    let mut paths = overlay::load_link(top).map_or_else(
        |_| chain_files(&subject.chain, vec![top_path]),
        |loaded| chain_files(&loaded.chain, Vec::new()),
    );
    if subject.requested != subject.name {
        push_unique(&mut paths, &resolve_top(&subject.requested).path);
    }
    paths
}

/// `paths` plus every file of `chain` on disk and the requested file's path,
/// where an override appears even while the embedded builtin is loaded.
fn chain_files(chain: &ThemeChain, mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    push_unique(&mut paths, &chain.top().path);
    for link in chain.iter().filter(|link| link.on_disk) {
        push_unique(&mut paths, &link.path);
    }
    paths
}
