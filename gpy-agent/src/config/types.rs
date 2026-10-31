//! Type-safe configuration primitives ("parse, don't validate").

use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// Error for a string that names no variant of a string-parsed setting.
///
/// Renders as `Invalid <field> '<value>'. Use <expected>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    field: &'static str,
    value: String,
    expected: &'static str,
}

impl ParseEnumError {
    pub(crate) fn new(field: &'static str, value: &str, expected: &'static str) -> Self {
        Self {
            field,
            value: value.to_owned(),
            expected,
        }
    }
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid {} '{}'. Use {}",
            self.field, self.value, self.expected
        )
    }
}

impl std::error::Error for ParseEnumError {}

impl From<ParseEnumError> for crate::Error {
    fn from(error: ParseEnumError) -> Self {
        Self::config(error.to_string())
    }
}

/// Declare a range-checked numeric newtype.
///
/// Generates `MIN`/`MAX`/`DEFAULT`, a const `new` returning `None` outside
/// `MIN..=MAX`, `get`, `Default`, `Display`, `TryFrom` (with the message built
/// by `error(min, max)`), and a `Deserialize` that rejects out-of-range values
/// with that message.
macro_rules! bounded_int {
    (
        $(#[$meta:meta])*
        $name:ident($int:ty) {
            min: $min:expr,
            max: $max:expr,
            default: $default:expr,
            error: $error:expr $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Serialize)]
        pub struct $name($int);

        impl $name {
            /// Default value.
            pub const DEFAULT: $int = $default;
            /// Smallest accepted value.
            pub const MIN: $int = $min;
            /// Largest accepted value.
            pub const MAX: $int = $max;

            /// Create a new value if it lies within `MIN..=MAX`.
            #[must_use]
            pub const fn new(value: $int) -> Option<Self> {
                if value >= Self::MIN && value <= Self::MAX {
                    Some(Self(value))
                } else {
                    None
                }
            }

            /// Get the inner value.
            #[must_use]
            pub const fn get(self) -> $int {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(Self::DEFAULT)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl TryFrom<$int> for $name {
            type Error = String;

            fn try_from(value: $int) -> Result<Self, Self::Error> {
                let describe: fn($int, $int) -> String = $error;
                Self::new(value).ok_or_else(|| describe(Self::MIN, Self::MAX))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <$int>::deserialize(deserializer)?;
                Self::try_from(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

/// A validated color specification (Hex, ANSI code, named color, or magic keyword),
/// stored in its canonical spelling so every shell and the template engine read
/// the same value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ColorSpec(String);

impl ColorSpec {
    /// Create a new `ColorSpec` from any accepted spelling.
    ///
    /// # Errors
    /// Returns an error if the string is not a valid color specification.
    pub fn new(value: &str) -> crate::Result<Self> {
        crate::config::validation::colors::canonical_color(value)
            .map(Self)
            .ok_or_else(|| crate::Error::invalid(format!("Invalid color specification: '{value}'")))
    }

    /// White, the fallback when a configured color does not validate.
    #[must_use]
    pub fn white() -> Self {
        Self("#ffffff".to_owned())
    }

    /// Get the color string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for ColorSpec {
    type Err = crate::Error;

    fn from_str(value: &str) -> crate::Result<Self> {
        Self::new(value)
    }
}

impl std::ops::Deref for ColorSpec {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<&str> for ColorSpec {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ColorSpec> for &str {
    fn eq(&self, other: &ColorSpec) -> bool {
        other == self
    }
}

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ColorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated UI icon: any string without control characters; may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Icon(String);

impl Icon {
    /// Create a new `Icon` after validation.
    ///
    /// # Errors
    /// Returns an error if the icon string contains control characters.
    pub fn new(value: impl Into<String>) -> crate::Result<Self> {
        let s = value.into();
        if s.chars().any(char::is_control) {
            return Err(crate::Error::invalid("Icon contains control characters"));
        }
        Ok(Self(s))
    }

    /// Get the icon string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Icon {
    type Err = crate::Error;

    fn from_str(value: &str) -> crate::Result<Self> {
        Self::new(value)
    }
}

impl std::ops::Deref for Icon {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<&str> for Icon {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Icon {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Icon> for &str {
    fn eq(&self, other: &Icon) -> bool {
        other == self
    }
}

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Icon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Icon {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<Icon> for String {
    fn from(icon: Icon) -> Self {
        icon.0
    }
}

bounded_int! {
    /// Agent timeout in seconds (1..=300).
    #[derive(Eq)]
    AgentTimeout(u64) {
        min: 1,
        max: 300,
        default: 5,
        error: |min, max| format!("agent timeout must be between {min} and {max}"),
    }
}

bounded_int! {
    /// Git timeout in seconds (1..=600).
    #[derive(Eq)]
    GitTimeout(u64) {
        min: 1,
        max: 600,
        default: 10,
        error: |min, max| format!("git timeout must be between {min} and {max}"),
    }
}

bounded_int! {
    /// Supervisor check interval in seconds (5..=3600).
    ///
    /// Out-of-range values, including `0`, are rejected at deserialize time.
    #[derive(Eq)]
    SupervisorCheckInterval(u64) {
        min: 5,
        max: 3600,
        default: 30,
        error: |min, max| format!(
            "agent.supervisor.check_interval_seconds must be between {min} and {max} seconds"
        ),
    }
}

bounded_int! {
    /// Maximum supervisor restart attempts before giving up (1..=100).
    #[derive(Eq)]
    SupervisorMaxRestartAttempts(u32) {
        min: 1,
        max: 100,
        default: 5,
        error: |min, max| format!(
            "agent.supervisor.max_restart_attempts must be between {min} and {max}"
        ),
    }
}

bounded_int! {
    /// Maximum branch length (0..=500); `0` means unlimited.
    #[derive(Eq)]
    MaxBranchLength(usize) {
        min: 0,
        max: 500,
        default: 0,
        error: |_, max| format!("max branch length must be <= {max}"),
    }
}

bounded_int! {
    /// Language cache TTL in hours (1..=720).
    #[derive(Eq)]
    CacheTtlHours(u64) {
        min: 1,
        max: 720,
        default: 24,
        error: |min, max| format!("cache TTL must be between {min} and {max}"),
    }
}

bounded_int! {
    /// Confidence threshold (0.0..=1.0).
    ConfidenceThreshold(f32) {
        min: 0.0,
        max: 1.0,
        default: 0.1,
        error: |_, _| "confidence threshold must be between 0.0 and 1.0".to_owned(),
    }
}

bounded_int! {
    /// Maximum path length (1..=1000).
    ///
    /// Hard cap in `char`s; at or below the ellipsis width (3) the ellipsis is
    /// dropped and the raw trailing characters are shown.
    #[derive(Eq)]
    MaxPathLength(usize) {
        min: 1,
        max: 1000,
        default: 80,
        error: |min, max| format!("max path length must be between {min} and {max}"),
    }
}

bounded_int! {
    /// Number of path components kept when the directory segment is truncated (1..=255).
    #[derive(Eq)]
    DirectoryTruncationLength(usize) {
        min: 1,
        max: 255,
        default: 3,
        error: |min, max| format!(
            "directory truncation length must be between {min} and {max}"
        ),
    }
}

/// The rejection text for a theme or palette `name` that fails [`is_safe_config_name`].
#[must_use]
pub fn invalid_name_message(kind: &str, name: &str) -> String {
    format!(
        "invalid {kind} name '{name}': must not contain path separators, '..', or control characters"
    )
}

/// Return `true` when `name` is safe to use as a palette or theme file stem.
///
/// Rejects empty, `/`, `\`, `..`, and control characters so joins stay inside
/// the config directory and TOML provenance headers can't be injected.
#[must_use]
pub fn is_safe_config_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

/// Marks which kind of file stem a [`SafeName`] names; supplies the label used in errors.
pub trait NameKind {
    /// Noun used in the rejection message, e.g. `theme`.
    const KIND: &'static str;
}

/// Marker for [`ThemeName`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeKind {}

impl NameKind for ThemeKind {
    const KIND: &'static str = "theme";
}

/// Marker for [`PaletteName`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteKind {}

impl NameKind for PaletteKind {
    const KIND: &'static str = "palette";
}

/// A theme or palette name that [`is_safe_config_name`] accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeName<K>(String, PhantomData<K>);

/// Theme name.
pub type ThemeName = SafeName<ThemeKind>;

/// Palette (color theme) name.
pub type PaletteName = SafeName<PaletteKind>;

impl<K: NameKind> SafeName<K> {
    /// Default name.
    pub const DEFAULT: &'static str = "default";

    /// Create a new name if the value is valid.
    ///
    /// Returns `None` unless [`is_safe_config_name`].
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        is_safe_config_name(&value).then_some(Self(value, PhantomData))
    }

    /// Get the inner value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<K: NameKind> Default for SafeName<K> {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned(), PhantomData)
    }
}

impl<K> fmt::Display for SafeName<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<K: NameKind> TryFrom<String> for SafeName<K> {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if is_safe_config_name(&value) {
            Ok(Self(value, PhantomData))
        } else {
            Err(invalid_name_message(K::KIND, &value))
        }
    }
}

impl<K> Serialize for SafeName<K> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, K: NameKind> Deserialize<'de> for SafeName<K> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Directory segment display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DirectoryDisplay {
    /// Show only the current directory name.
    #[default]
    Basename,
    /// Show the path with intermediate components abbreviated to their first letter.
    Abbreviated,
    /// Show the path truncated to its trailing components.
    Truncated,
    /// Show the full absolute path.
    Full,
}

impl fmt::Display for DirectoryDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Basename => "basename",
            Self::Abbreviated => "abbreviated",
            Self::Truncated => "truncated",
            Self::Full => "full",
        };
        write!(f, "{s}")
    }
}

impl FromStr for DirectoryDisplay {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "basename" => Ok(Self::Basename),
            "abbreviated" => Ok(Self::Abbreviated),
            "truncated" => Ok(Self::Truncated),
            "full" => Ok(Self::Full),
            _ => Err(ParseEnumError::new(
                "ui.directory.display",
                s,
                "'basename', 'abbreviated', 'truncated', or 'full'",
            )),
        }
    }
}

/// Prefix shown before a truncated directory path (rejects control characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(try_from = "String")]
pub struct DirectoryTruncationSymbol(String);

impl DirectoryTruncationSymbol {
    /// Create a new `DirectoryTruncationSymbol` if the value is valid.
    ///
    /// Returns `None` on Unicode control characters; empty is valid (default).
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if value.chars().any(char::is_control) {
            None
        } else {
            Some(Self(value))
        }
    }

    /// Get the inner value
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DirectoryTruncationSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for DirectoryTruncationSymbol {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            "directory truncation symbol must not contain control characters".to_owned()
        })
    }
}

/// Language segment display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LanguageDisplay {
    /// Show language/tool icons (falling back to the name when no icon exists).
    #[default]
    Icon,
    /// Always show the language name.
    Text,
}

impl fmt::Display for LanguageDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Icon => "icon",
            Self::Text => "text",
        };
        write!(f, "{s}")
    }
}

impl FromStr for LanguageDisplay {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "icon" => Ok(Self::Icon),
            "text" => Ok(Self::Text),
            _ => Err(ParseEnumError::new(
                "language.display",
                s,
                "'icon' or 'text'",
            )),
        }
    }
}

/// Strategy for deciding which languages the prompt surfaces.
///
/// `markers` (default): require project markers; `content`: rank by file
/// prevalence; `hybrid`: content ranking gated by a marker file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DetectionMode {
    /// Recursive content scan, ranked by prevalence (opt-in).
    Content,
    /// Marker-based: show a language only when its markers/extensions exist (default).
    #[default]
    Markers,
    /// Content scan filtered to languages whose project marker file is present (opt-in).
    Hybrid,
}

impl fmt::Display for DetectionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Content => "content",
            Self::Markers => "markers",
            Self::Hybrid => "hybrid",
        };
        write!(f, "{s}")
    }
}

impl FromStr for DetectionMode {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "content" => Ok(Self::Content),
            "markers" => Ok(Self::Markers),
            "hybrid" => Ok(Self::Hybrid),
            _ => Err(ParseEnumError::new(
                "language.detection_mode",
                s,
                "'content', 'markers', or 'hybrid'",
            )),
        }
    }
}

/// Language segment filter: how many detected languages to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanguageFilter {
    /// Show every detected language.
    #[default]
    All,
    /// Show only the dominant (highest-confidence) language.
    Primary,
    /// Show the top `n` languages by confidence.
    Top(usize),
}

impl fmt::Display for LanguageFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Primary => write!(f, "primary"),
            Self::Top(n) => write!(f, "{n}"),
        }
    }
}

impl FromStr for LanguageFilter {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(Self::All),
            "primary" => Ok(Self::Primary),
            other => other.parse::<usize>().ok().map(Self::Top).ok_or_else(|| {
                ParseEnumError::new(
                    "language.filter",
                    other,
                    "'all', 'primary', or a number (e.g. '3')",
                )
            }),
        }
    }
}

impl Serialize for LanguageFilter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LanguageFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_config_name_guards() {
        assert!(super::is_safe_config_name("nord"));
        assert!(super::is_safe_config_name("my-theme"));
        assert!(super::is_safe_config_name("theme_v2"));
        assert!(!super::is_safe_config_name(""));
        assert!(!super::is_safe_config_name("   "));
        assert!(!super::is_safe_config_name("../../foo"));
        assert!(!super::is_safe_config_name("a/b"));
        assert!(!super::is_safe_config_name("a\\b"));
        assert!(!super::is_safe_config_name("name\nwith-newline"));
        assert!(!super::is_safe_config_name("name\0null"));
    }

    #[test]
    fn palette_name_rejects_empty_and_traversal() {
        assert!(PaletteName::new("  ".to_owned()).is_none());
        assert!(PaletteName::new("../../etc/passwd".to_owned()).is_none());
        assert!(PaletteName::new("foo/bar".to_owned()).is_none());
        assert!(PaletteName::new("foo\nbar".to_owned()).is_none());
        assert_eq!(PaletteName::default().as_str(), "default");
        let n = PaletteName::new("nord".to_owned()).expect("valid");
        assert_eq!(n.as_str(), "nord");
    }

    #[test]
    fn theme_name_rejects_empty_and_traversal() {
        assert!(ThemeName::new("  ".to_owned()).is_none());
        assert!(ThemeName::new("../../etc/passwd".to_owned()).is_none());
        assert!(ThemeName::new("foo/bar".to_owned()).is_none());
        assert!(ThemeName::new("foo\nbar".to_owned()).is_none());
        assert_eq!(ThemeName::default().as_str(), "default");
        let n = ThemeName::new("my-theme".to_owned()).expect("valid");
        assert_eq!(n.as_str(), "my-theme");
    }

    #[test]
    fn directory_truncation_length_enforces_bounds() {
        assert!(DirectoryTruncationLength::new(0).is_none());
        assert!(DirectoryTruncationLength::new(256).is_none());
        assert_eq!(DirectoryTruncationLength::default().get(), 3);
        assert_eq!(
            DirectoryTruncationLength::new(DirectoryTruncationLength::MIN)
                .expect("MIN is valid")
                .get(),
            1
        );
        assert_eq!(
            DirectoryTruncationLength::new(DirectoryTruncationLength::MAX)
                .expect("MAX is valid")
                .get(),
            255
        );
    }

    #[test]
    fn directory_truncation_symbol_rejects_control_chars() {
        assert!(DirectoryTruncationSymbol::new("bad\nsymbol".to_owned()).is_none());
        assert!(DirectoryTruncationSymbol::new("tab\there".to_owned()).is_none());
        assert!(DirectoryTruncationSymbol::new("null\0".to_owned()).is_none());
        assert_eq!(DirectoryTruncationSymbol::default().as_str(), "");
        assert_eq!(
            DirectoryTruncationSymbol::new(String::new())
                .expect("empty is valid")
                .as_str(),
            ""
        );
        assert_eq!(
            DirectoryTruncationSymbol::new("…/".to_owned())
                .expect("ellipsis prefix is valid")
                .as_str(),
            "…/"
        );
    }

    #[test]
    fn directory_display_renders_and_keeps_all_variants() {
        assert_eq!(DirectoryDisplay::Basename.to_string(), "basename");
        assert_eq!(DirectoryDisplay::Abbreviated.to_string(), "abbreviated");
        assert_eq!(DirectoryDisplay::Truncated.to_string(), "truncated");
        assert_eq!(DirectoryDisplay::Full.to_string(), "full");
    }

    #[test]
    fn language_display_parses_and_renders() {
        assert_eq!("icon".parse::<LanguageDisplay>(), Ok(LanguageDisplay::Icon));
        assert_eq!("text".parse::<LanguageDisplay>(), Ok(LanguageDisplay::Text));
        assert_eq!("ICON".parse::<LanguageDisplay>(), Ok(LanguageDisplay::Icon));
        assert!("glyph".parse::<LanguageDisplay>().is_err());
        assert_eq!(LanguageDisplay::default(), LanguageDisplay::Icon);
        assert_eq!(LanguageDisplay::Icon.to_string(), "icon");
        assert_eq!(LanguageDisplay::Text.to_string(), "text");
    }

    #[test]
    fn language_filter_parses_all_primary_and_counts() {
        assert_eq!("all".parse::<LanguageFilter>(), Ok(LanguageFilter::All));
        assert_eq!(
            "primary".parse::<LanguageFilter>(),
            Ok(LanguageFilter::Primary)
        );
        assert_eq!("3".parse::<LanguageFilter>(), Ok(LanguageFilter::Top(3)));
        assert_eq!("0".parse::<LanguageFilter>(), Ok(LanguageFilter::Top(0)));
        assert_eq!(LanguageFilter::default(), LanguageFilter::All);
    }

    #[test]
    fn language_filter_rejects_garbage_with_message() {
        let err = "bogus".parse::<LanguageFilter>().expect_err("must reject");
        assert!(err.to_string().contains("language.filter"), "got {err}");
        assert!("-1".parse::<LanguageFilter>().is_err());
    }

    #[test]
    fn language_filter_round_trips_via_display() {
        for value in ["all", "primary", "7"] {
            let parsed: LanguageFilter = value.parse().expect("valid");
            assert_eq!(parsed.to_string(), value);
        }
    }

    #[test]
    fn bounded_ints_enforce_range_and_report_it() {
        assert!(AgentTimeout::new(0).is_none());
        assert!(AgentTimeout::new(301).is_none());
        assert_eq!(AgentTimeout::new(300).expect("MAX is valid").get(), 300);
        assert_eq!(
            AgentTimeout::try_from(0).expect_err("below MIN"),
            "agent timeout must be between 1 and 300"
        );
        assert_eq!(
            SupervisorCheckInterval::try_from(4).expect_err("below MIN"),
            "agent.supervisor.check_interval_seconds must be between 5 and 3600 seconds"
        );
        assert_eq!(
            MaxBranchLength::try_from(501).expect_err("above MAX"),
            "max branch length must be <= 500"
        );
        assert_eq!(MaxBranchLength::new(0).expect("zero is unlimited").get(), 0);
        assert_eq!(
            ConfidenceThreshold::try_from(1.5).expect_err("above MAX"),
            "confidence threshold must be between 0.0 and 1.0"
        );
        assert!(ConfidenceThreshold::new(f32::NAN).is_none());
        assert_eq!(AgentTimeout::default().to_string(), "5");
    }

    #[test]
    fn bounded_int_deserialize_rejects_out_of_range_with_message() {
        #[derive(Debug, Deserialize)]
        struct Holder {
            timeout: GitTimeout,
        }
        let ok: Holder = toml::from_str("timeout = 600").expect("MAX is valid");
        assert_eq!(ok.timeout.get(), 600);
        let err = toml::from_str::<Holder>("timeout = 601").expect_err("above MAX");
        assert!(
            err.to_string()
                .contains("git timeout must be between 1 and 600"),
            "got {err}"
        );
    }

    #[test]
    fn safe_names_report_their_kind() {
        assert!(
            ThemeName::try_from(String::new())
                .expect_err("empty")
                .starts_with("invalid theme name")
        );
        assert!(
            PaletteName::try_from("a/b".to_owned())
                .expect_err("separator")
                .starts_with("invalid palette name")
        );
    }

    #[test]
    fn string_enums_share_one_error_style() {
        assert_eq!(
            "sideways"
                .parse::<DirectoryDisplay>()
                .unwrap_err()
                .to_string(),
            "Invalid ui.directory.display 'sideways'. Use 'basename', 'abbreviated', 'truncated', or 'full'"
        );
        assert_eq!(
            "glyph".parse::<LanguageDisplay>().unwrap_err().to_string(),
            "Invalid language.display 'glyph'. Use 'icon' or 'text'"
        );
        assert_eq!(
            "x".parse::<DetectionMode>().unwrap_err().to_string(),
            "Invalid language.detection_mode 'x'. Use 'content', 'markers', or 'hybrid'"
        );
        assert_eq!(
            "x".parse::<LanguageFilter>().unwrap_err().to_string(),
            "Invalid language.filter 'x'. Use 'all', 'primary', or a number (e.g. '3')"
        );
    }
}
