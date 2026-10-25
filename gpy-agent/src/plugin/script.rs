//! Script segment manifests (`segments/<id>.toml`) for `entry_type = "script"` plugins.
//!
//! The manifest is the author-facing contract of ADR-0011 option G: which
//! script runs, what it receives, when it runs and how long its output is
//! reused. The runner itself lives in the shell adapters; this module only
//! declares and validates.

use super::manifest::SegmentName;
use super::settings::{SettingDecl, SettingKey};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Manifest fields the user overrides in `[plugin.<segment>]` next to the
/// declared settings; a declared setting may not reuse these names.
pub const RESERVED_SETTING_KEYS: [&str; 2] = ["ttl_secs", "timeout_ms"];

/// Cap on segment manifest reads, as for `plugin.toml`.
const SEGMENT_MANIFEST_READ_CAP: usize = 65_536;

/// Path of the manifest for `segment` inside a script plugin `root`.
#[must_use]
pub fn script_segment_manifest_path(root: &Path, segment: &SegmentName) -> PathBuf {
    root.join("segments").join(format!("{segment}.toml"))
}

/// Whether `name` is a portable environment variable name
/// (`^[A-Za-z_][A-Za-z0-9_]*$`, ASCII only).
#[must_use]
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The message of an [`Error::Invalid`] without its `Invalid operation:`
/// prefix, so nested contexts read as one sentence.
pub(crate) fn detail(error: Error) -> String {
    match error {
        Error::Invalid { message } => message,
        other => other.to_string(),
    }
}

/// Implement `Deserialize` through a validating `new(&str) -> Result<Self>`.
macro_rules! deserialize_via_new {
    ($ty:ty) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                Self::new(&raw).map_err(|e| serde::de::Error::custom(detail(e)))
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

/// A public shell value a script can declare in `inputs`.
///
/// Each reaches the script as `GPY_<NAME>`; only declared inputs are set and
/// only they are part of the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputName {
    /// The shell's working directory; the script then runs in it and the
    /// cache becomes per folder.
    Pwd,
    /// `1` when the shell runs as root, else `0`.
    IsRoot,
    /// `1` inside a `sudo` session, else `0`.
    IsSudo,
    /// `1` over SSH, else `0`.
    IsSsh,
    /// The last command's duration in milliseconds; empty before the first command.
    CmdDuration,
    /// The glyph mode: `nerd` (Nerd Font icons) or `ascii`.
    PromptIcons,
}

impl InputName {
    /// Every input, in documentation order.
    pub const ALL: [Self; 6] = [
        Self::Pwd,
        Self::IsRoot,
        Self::IsSudo,
        Self::IsSsh,
        Self::CmdDuration,
        Self::PromptIcons,
    ];

    /// Manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pwd => "pwd",
            Self::IsRoot => "is_root",
            Self::IsSudo => "is_sudo",
            Self::IsSsh => "is_ssh",
            Self::CmdDuration => "cmd_duration",
            Self::PromptIcons => "prompt_icons",
        }
    }

    /// Environment variable the runner sets for this input.
    #[must_use]
    pub const fn env_var(self) -> &'static str {
        match self {
            Self::Pwd => "GPY_PWD",
            Self::IsRoot => "GPY_IS_ROOT",
            Self::IsSudo => "GPY_IS_SUDO",
            Self::IsSsh => "GPY_IS_SSH",
            Self::CmdDuration => "GPY_CMD_DURATION",
            Self::PromptIcons => "GPY_PROMPT_ICONS",
        }
    }
}

/// How widely a cached script result is reused, derived from `inputs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope {
    /// No `pwd` input: one result per shell session (and input values).
    Session,
    /// `pwd` declared: one result per working directory.
    Folder,
}

impl CacheScope {
    /// Lowercase name used in CLI output and docs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Folder => "folder",
        }
    }
}

/// A validated environment variable name (`^[A-Za-z_][A-Za-z0-9_]*$`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct EnvName(String);

impl EnvName {
    /// Validate an environment variable name.
    ///
    /// # Errors
    /// Returns an error naming the value if it is not a portable name.
    pub fn new(name: &str) -> Result<Self> {
        if is_env_name(name) {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::invalid(format!(
                "'{name}' is not an environment variable name (^[A-Za-z_][A-Za-z0-9_]*$)"
            )))
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via_new!(EnvName);

/// A plugin-relative path to a POSIX `sh` script (`command`, `require_command`).
///
/// Syntax is checked at parse; [`ScriptPath::resolve`] checks that it names a
/// file that stays inside the plugin root after symlinks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ScriptPath(String);

impl ScriptPath {
    /// Validate a plugin-relative script path.
    ///
    /// # Errors
    /// Returns an error if the path is empty, absolute, contains `..` or a
    /// control character.
    pub fn new(path: &str) -> Result<Self> {
        if path.is_empty() || path.chars().any(char::is_control) || path.contains('\\') {
            return Err(Error::invalid(format!(
                "'{path}' is not a plugin-relative path (empty, control character or backslash)"
            )));
        }
        let parsed = Path::new(path);
        if !parsed
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(Error::invalid(format!(
                "'{path}' must be relative to the plugin root, without '..'"
            )));
        }
        Ok(Self(path.to_owned()))
    }

    /// The path as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolve under `root`, requiring a regular file inside it after symlinks.
    ///
    /// # Errors
    /// Returns an error if the file is missing or escapes the plugin root.
    pub fn resolve(&self, root: &Path) -> Result<PathBuf> {
        let canonical_root = root.canonicalize().map_err(|e| {
            Error::invalid(format!(
                "Cannot resolve plugin root {}: {e}",
                root.display()
            ))
        })?;
        let target = root.join(&self.0);
        let canonical = target
            .canonicalize()
            .map_err(|e| Error::invalid(format!("Script '{}' not found: {e}", target.display())))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(Error::invalid(format!(
                "Script '{}' resolves outside the plugin root",
                self.0
            )));
        }
        if !canonical.is_file() {
            return Err(Error::invalid(format!(
                "Script '{}' is not a regular file",
                self.0
            )));
        }
        Ok(canonical)
    }
}

deserialize_via_new!(ScriptPath);

/// One file or folder name searched for by `require_files`/`require_folders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ScanName(String);

impl ScanName {
    /// Validate a single path component.
    ///
    /// # Errors
    /// Returns an error if the name is empty, `.`/`..`, holds a separator or a control character.
    pub fn new(name: &str) -> Result<Self> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(Error::invalid(format!(
                "'{name}' is not a single file or folder name"
            )));
        }
        Ok(Self(name.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via_new!(ScanName);

/// A file extension for `require_extensions`, without the leading dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Extension(String);

impl Extension {
    /// Validate an extension such as `tf` or `tar.gz`.
    ///
    /// # Errors
    /// Returns an error if it is empty, starts with `.` or holds other than
    /// ASCII letters, digits, `_`, `+`, `-` and `.`.
    pub fn new(extension: &str) -> Result<Self> {
        let valid = !extension.is_empty()
            && !extension.starts_with('.')
            && extension
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.'));
        if valid {
            Ok(Self(extension.to_owned()))
        } else {
            Err(Error::invalid(format!(
                "'{extension}' is not a file extension (no leading dot; letters, digits, _ + - .)"
            )))
        }
    }

    /// The extension.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via_new!(Extension);

/// Bounded `u32` newtype with a default, deserialized via `try_from`.
macro_rules! bounded_u32 {
    ($(#[$doc:meta])* $name:ident, $min:expr, $max:expr, $default:expr, $what:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "u32", into = "u32")]
        pub struct $name(u32);

        impl $name {
            /// Smallest accepted value.
            pub const MIN: u32 = $min;
            /// Largest accepted value.
            pub const MAX: u32 = $max;

            /// Validate a value.
            ///
            /// # Errors
            /// Returns an error naming the accepted range.
            pub fn new(value: u32) -> Result<Self> {
                if (Self::MIN..=Self::MAX).contains(&value) {
                    Ok(Self(value))
                } else {
                    Err(Error::invalid(format!(
                        concat!($what, " must be {}..={}, got {}"),
                        Self::MIN,
                        Self::MAX,
                        value
                    )))
                }
            }

            /// The value.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self($default)
            }
        }

        impl TryFrom<u32> for $name {
            type Error = String;
            fn try_from(value: u32) -> std::result::Result<Self, String> {
                Self::new(value).map_err(detail)
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

bounded_u32!(
    /// Seconds a script result is reused; `0` runs the script on every due prompt.
    TtlSecs,
    0,
    31_536_000,
    5,
    "ttl_secs"
);

bounded_u32!(
    /// Milliseconds a script may run before it is killed and the segment hidden.
    TimeoutMs,
    1,
    500,
    500,
    "timeout_ms"
);

/// The optional second output line: a colour variant chosen by the script
/// (`^[a-z0-9_]{1,32}$`), styled by the theme's `<variant>_bg_color` and
/// `<variant>_text_color`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct VariantName(String);

impl VariantName {
    /// Longest accepted variant name.
    pub const MAX_LEN: usize = 32;

    /// Validate a variant name.
    ///
    /// # Errors
    /// Returns an error if it is empty, longer than 32 bytes or holds other
    /// than lowercase ASCII letters, digits and `_`.
    pub fn new(name: &str) -> Result<Self> {
        let valid = (1..=Self::MAX_LEN).contains(&name.len())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(Error::invalid(format!(
                "'{name}' is not a colour variant name (^[a-z0-9_]{{1,32}}$)"
            )))
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

deserialize_via_new!(VariantName);

/// Which colour of a variant a theme key sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantColorPart {
    /// `<variant>_bg_color`.
    Bg,
    /// `<variant>_text_color`.
    Text,
}

/// A plugin segment theme key `<variant>_bg_color` or `<variant>_text_color`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantColorKey {
    /// The variant the script prints on its second line.
    pub variant: VariantName,
    /// Background or text colour.
    pub part: VariantColorPart,
}

impl VariantColorKey {
    /// Parse a theme key.
    ///
    /// # Errors
    /// Returns an error listing the accepted plugin segment style keys.
    pub fn new(key: &str) -> Result<Self> {
        let parsed = key
            .strip_suffix("_bg_color")
            .map(|variant| (variant, VariantColorPart::Bg))
            .or_else(|| {
                key.strip_suffix("_text_color")
                    .map(|variant| (variant, VariantColorPart::Text))
            })
            .and_then(|(variant, part)| {
                VariantName::new(variant).ok().map(|name| Self {
                    variant: name,
                    part,
                })
            });
        parsed.ok_or_else(|| {
            Error::invalid(format!(
                "unknown plugin segment style key '{key}': use icon, open, close, bg_color, \
                 text_color or <variant>_bg_color / <variant>_text_color \
                 (behaviour settings belong in config.toml [plugin.<segment>])"
            ))
        })
    }
}

impl fmt::Display for VariantColorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let suffix = match self.part {
            VariantColorPart::Bg => "bg_color",
            VariantColorPart::Text => "text_color",
        };
        write!(f, "{}_{suffix}", self.variant)
    }
}

impl Serialize for VariantColorKey {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for VariantColorKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(|e| serde::de::Error::custom(detail(e)))
    }
}

/// A script segment manifest, `segments/<id>.toml` in a script plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSegmentManifest {
    /// One line shown by `gpy plugin validate`.
    #[serde(default)]
    pub description: Option<String>,
    /// The script, relative to the plugin root; run by `/bin/sh`.
    pub command: ScriptPath,
    /// Public shell values passed as `GPY_<NAME>`; `pwd` makes the cache per folder.
    #[serde(default)]
    pub inputs: Vec<InputName>,
    /// Extra environment variables forwarded to the script and keyed in the cache.
    #[serde(default)]
    pub env: Vec<EnvName>,
    /// Seconds a result is reused (default 5; `0` disables the cache).
    #[serde(default)]
    pub ttl_secs: TtlSecs,
    /// Run time limit in milliseconds (default and maximum 500).
    #[serde(default)]
    pub timeout_ms: TimeoutMs,
    /// Every name must exist as a file in the working directory or an ancestor.
    #[serde(default)]
    pub require_files: Vec<ScanName>,
    /// Every name must exist as a folder in the working directory or an ancestor.
    #[serde(default)]
    pub require_folders: Vec<ScanName>,
    /// Every extension must match a file in the working directory itself.
    #[serde(default)]
    pub require_extensions: Vec<Extension>,
    /// Every variable must be set and non-empty in the shell.
    #[serde(default)]
    pub require_env: Vec<EnvName>,
    /// A script, relative to the plugin root, that must exit 0; run last.
    #[serde(default)]
    pub require_command: Option<ScriptPath>,
    /// Typed settings, passed as `GPY_OPT_<key>` and overridable in `config.toml`.
    #[serde(default)]
    pub settings: BTreeMap<SettingKey, SettingDecl>,
}

/// First value in `items` that appears twice, by `key`.
fn first_duplicate<'a, T, K: PartialEq>(items: &'a [T], key: impl Fn(&'a T) -> K) -> Option<&'a T> {
    items.iter().enumerate().find_map(|(index, item)| {
        items
            .iter()
            .take(index)
            .any(|earlier| key(earlier) == key(item))
            .then_some(item)
    })
}

impl ScriptSegmentManifest {
    /// Parse and validate a segment manifest's text (no filesystem checks).
    ///
    /// # Errors
    /// Returns an error naming the offending key.
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(text).map_err(|e| Error::invalid(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if let Some(input) = first_duplicate(&self.inputs, |input| *input) {
            return Err(Error::invalid(format!(
                "inputs: '{}' is listed twice",
                input.as_str()
            )));
        }
        if let Some(name) = first_duplicate(&self.env, EnvName::as_str) {
            return Err(Error::invalid(format!("env: '{name}' is listed twice")));
        }
        if let Some(name) = self
            .env
            .iter()
            .find(|name| name.as_str().starts_with("GPY_"))
        {
            return Err(Error::invalid(format!(
                "env: '{name}' is reserved (GPY_* variables are set by the runner)"
            )));
        }
        for (key, names) in [
            ("require_files", &self.require_files),
            ("require_folders", &self.require_folders),
        ] {
            if let Some(name) = first_duplicate(names, ScanName::as_str) {
                return Err(Error::invalid(format!("{key}: '{name}' is listed twice")));
            }
        }
        if let Some(extension) = first_duplicate(&self.require_extensions, Extension::as_str) {
            return Err(Error::invalid(format!(
                "require_extensions: '{extension}' is listed twice"
            )));
        }
        if let Some(name) = first_duplicate(&self.require_env, EnvName::as_str) {
            return Err(Error::invalid(format!(
                "require_env: '{name}' is listed twice"
            )));
        }
        for (key, decl) in &self.settings {
            if RESERVED_SETTING_KEYS.contains(&key.as_str()) {
                return Err(Error::invalid(format!(
                    "settings.{key}: '{key}' is a manifest field, not a setting"
                )));
            }
            decl.validate()
                .map_err(|e| Error::invalid(format!("settings.{key}: {}", detail(e))))?;
        }
        Ok(())
    }

    /// Read, parse and validate `segments/<segment>.toml` under `root`,
    /// including script containment.
    ///
    /// # Errors
    /// Returns an error naming the manifest file and the offending key.
    pub fn load(root: &Path, segment: &SegmentName) -> Result<Self> {
        let path = script_segment_manifest_path(root, segment);
        let context = |message: String| Error::invalid(format!("{}: {message}", path.display()));
        let text = crate::fs_util::read_small_file(&path, SEGMENT_MANIFEST_READ_CAP)
            .map_err(|e| context(format!("cannot read: {e}")))?;
        let manifest = Self::parse(&text).map_err(|e| context(detail(e)))?;
        manifest
            .command
            .resolve(root)
            .map_err(|e| context(format!("command: {}", detail(e))))?;
        if let Some(require_command) = &manifest.require_command {
            require_command
                .resolve(root)
                .map_err(|e| context(format!("require_command: {}", detail(e))))?;
        }
        Ok(manifest)
    }

    /// Cache scope implied by the declared inputs.
    #[must_use]
    pub fn cache_scope(&self) -> CacheScope {
        if self.inputs.contains(&InputName::Pwd) {
            CacheScope::Folder
        } else {
            CacheScope::Session
        }
    }
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
