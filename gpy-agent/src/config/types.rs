//! Type-safe configuration primitives
//!
//! This module defines types that enforce validation rules during construction
//! or deserialization, adhering to the "Parse, don't validate" philosophy.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Strongly-typed process ID for registered clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ClientPid(u32);

impl ClientPid {
    /// Largest plausible process ID (Linux's default `pid_max` ceiling).
    const MAX: u32 = 4_194_304;

    /// Create a new `ClientPid` after validation.
    ///
    /// Range only: this does *not* probe whether a process with this PID is
    /// alive. Liveness is checked exactly once per request, at IPC route time
    /// (`ipc::server::connection::route_request_secure`), gated by
    /// `GuardSettings::validate_pids`. Probing here as well — unconditionally,
    /// before any `GuardSettings` was in scope — meant that toggle could not
    /// actually disable liveness probing, and every registration paid for the
    /// same `kill(pid, 0)` syscall twice (#578).
    ///
    /// # Errors
    /// Returns an error if the PID is out of range (0, or above the platform's
    /// plausible maximum).
    pub fn new(pid: u32) -> crate::Result<Self> {
        if Self::pid_in_range(pid) {
            Ok(Self(pid))
        } else {
            Err(crate::Error::ipc(format!("Invalid PID: {pid}")))
        }
    }

    /// Whether `pid` is a plausible process ID: nonzero and within the
    /// platform's maximum.
    ///
    /// Applied uniformly on every platform (#578) — the upper bound used to be
    /// checked on the non-Unix path only, so a Unix host accepted any nonzero
    /// `u32` as a PID.
    #[must_use]
    pub const fn pid_in_range(pid: u32) -> bool {
        pid > 0 && pid <= Self::MAX
    }

    /// Get the raw PID value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ClientPid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<u32> for ClientPid {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ClientPid> for u32 {
    fn eq(&self, other: &ClientPid) -> bool {
        *self == other.0
    }
}

impl<'de> Deserialize<'de> for ClientPid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Supported shell variants for registered clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShellVariant {
    /// Fish shell
    Fish,
    /// Zsh shell
    Zsh,
    /// Bash shell
    Bash,
    /// Unknown or unsupported shell
    #[default]
    Unknown,
}

impl FromStr for ShellVariant {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fish" => Ok(Self::Fish),
            "zsh" => Ok(Self::Zsh),
            "bash" => Ok(Self::Bash),
            _ => Ok(Self::Unknown),
        }
    }
}

impl fmt::Display for ShellVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Fish => "fish",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Unknown => "unknown",
        };
        write!(f, "{s}")
    }
}

/// A validated color specification (Hex, ANSI code, named color, or magic keyword).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ColorSpec(String);

impl ColorSpec {
    /// Create a new `ColorSpec` after validation.
    ///
    /// # Errors
    /// Returns an error if the string is not a valid color specification.
    pub fn new(value: &str) -> crate::Result<Self> {
        if crate::config::validation::colors::is_valid_color(value) {
            Ok(Self(value.to_lowercase()))
        } else {
            Err(crate::Error::invalid(format!(
                "Invalid color specification: '{value}'"
            )))
        }
    }

    /// Get the color string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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

/// A validated UI icon (non-empty string).
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

/// Agent timeout in seconds (1..=300)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct AgentTimeout(u64);

impl AgentTimeout {
    /// Default agent timeout
    pub const DEFAULT: u64 = 5;
    /// Minimum allowed timeout
    pub const MIN: u64 = 1;
    /// Maximum allowed timeout
    pub const MAX: u64 = 300;

    /// Create a new `AgentTimeout` if the value is valid
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for AgentTimeout {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for AgentTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<u64> for AgentTimeout {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "agent timeout must be between {} and {}",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

/// Git timeout in seconds (1..=600)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct GitTimeout(u64);

impl GitTimeout {
    /// Default git timeout
    pub const DEFAULT: u64 = 10;
    /// Minimum allowed timeout
    pub const MIN: u64 = 1;
    /// Maximum allowed timeout
    pub const MAX: u64 = 600;

    /// Create a new `GitTimeout` if the value is valid
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for GitTimeout {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for GitTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<u64> for GitTimeout {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "git timeout must be between {} and {}",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

/// Supervisor check interval in seconds (5..=3600).
///
/// Bounded at construction (#597) so an explicit `0` in a saved config is
/// rejected at deserialize time rather than silently coerced to the default
/// by the loader — see `config::loader::apply_defaults`'s history for the
/// inconsistency this closes (a value of exactly `0` used to be silently
/// replaced, while `1`..`4` hard-failed later in `validate_config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct SupervisorCheckInterval(u64);

impl SupervisorCheckInterval {
    /// Default supervisor check interval
    pub const DEFAULT: u64 = 30;
    /// Minimum allowed interval
    pub const MIN: u64 = 5;
    /// Maximum allowed interval
    pub const MAX: u64 = 3600;

    /// Create a new `SupervisorCheckInterval` if the value is valid
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for SupervisorCheckInterval {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "agent.supervisor.check_interval_seconds must be between {} and {} seconds",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

impl Default for SupervisorCheckInterval {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for SupervisorCheckInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Maximum supervisor restart attempts before giving up (1..=100).
///
/// Bounded at construction (#597); see [`SupervisorCheckInterval`]'s doc for
/// the inconsistency this closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32")]
pub struct SupervisorMaxRestartAttempts(u32);

impl SupervisorMaxRestartAttempts {
    /// Default maximum restart attempts
    pub const DEFAULT: u32 = 5;
    /// Minimum allowed attempts
    pub const MIN: u32 = 1;
    /// Maximum allowed attempts
    pub const MAX: u32 = 100;

    /// Create a new `SupervisorMaxRestartAttempts` if the value is valid
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for SupervisorMaxRestartAttempts {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "agent.supervisor.max_restart_attempts must be between {} and {}",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

impl Default for SupervisorMaxRestartAttempts {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for SupervisorMaxRestartAttempts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Maximum branch length (0..=500)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize")]
pub struct MaxBranchLength(usize);

impl MaxBranchLength {
    /// Default max branch length (0 = unlimited)
    pub const DEFAULT: usize = 0;
    /// Maximum allowed length
    pub const MAX: usize = 500;

    /// Create a new `MaxBranchLength` if the value is valid
    #[must_use]
    pub const fn new(value: usize) -> Option<Self> {
        if value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for MaxBranchLength {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for MaxBranchLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<usize> for MaxBranchLength {
    type Error = String;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| format!("max branch length must be <= {}", Self::MAX))
    }
}

/// Language cache TTL in hours (1..=720)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct CacheTtlHours(u64);

impl CacheTtlHours {
    /// Default cache TTL
    pub const DEFAULT: u64 = 24;
    /// Minimum allowed TTL
    pub const MIN: u64 = 1;
    /// Maximum allowed TTL
    pub const MAX: u64 = 720;

    /// Create a new `CacheTtlHours` if the value is valid
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for CacheTtlHours {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for CacheTtlHours {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<u64> for CacheTtlHours {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
            .ok_or_else(|| format!("cache TTL must be between {} and {}", Self::MIN, Self::MAX))
    }
}

/// Confidence threshold (0.0..=1.0)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f32")]
pub struct ConfidenceThreshold(f32);

impl ConfidenceThreshold {
    /// Default confidence threshold
    pub const DEFAULT: f32 = 0.1;

    /// Create a new `ConfidenceThreshold` if the value is valid
    #[must_use]
    pub const fn new(value: f32) -> Option<Self> {
        if value >= 0.0 && value <= 1.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for ConfidenceThreshold {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for ConfidenceThreshold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<f32> for ConfidenceThreshold {
    type Error = String;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
            .ok_or_else(|| "confidence threshold must be between 0.0 and 1.0".to_owned())
    }
}

/// Maximum path length (1..=1000)
///
/// This is a hard cap: the rendered directory string never exceeds this many
/// `char`s. For values at or below the truncation ellipsis width (3 — see
/// `directory_resolver::display_path`'s `ELLIPSIS_LEN`), the ellipsis would
/// either overflow the cap or consume the entire budget on dots that convey
/// no path information, so it is dropped and the raw last `max_length`
/// characters of the path are shown instead (e.g. `MIN` = 1 shows just the
/// final character, with no `"..."`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize")]
pub struct MaxPathLength(usize);

impl MaxPathLength {
    /// Default max path length
    pub const DEFAULT: usize = 80;
    /// Minimum allowed length
    pub const MIN: usize = 1;
    /// Maximum allowed length
    pub const MAX: usize = 1000;

    /// Create a new `MaxPathLength` if the value is valid
    #[must_use]
    pub const fn new(value: usize) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for MaxPathLength {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for MaxPathLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<usize> for MaxPathLength {
    type Error = String;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "max path length must be between {} and {}",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

/// Return `true` when `name` is safe to use as a palette or theme file stem.
///
/// Rejects names that are empty, contain path separators (`/`, `\`), the
/// parent-directory token (`..`), or ASCII control characters (including `\n`
/// and `\r`).  Keeps the name→path join inside the intended config directory
/// and prevents control characters from being interpolated into provenance
/// headers in emitted TOML.
#[must_use]
pub fn is_safe_config_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

/// Theme name (non-empty string)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ThemeName(String);

impl ThemeName {
    /// Default theme name
    pub const DEFAULT: &'static str = "default";

    /// Create a new `ThemeName` if the value is valid.
    ///
    /// Returns `None` when the value is empty, contains path separators or
    /// `..`, or contains control characters.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if is_safe_config_name(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ThemeName {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for ThemeName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            "theme name must be non-empty and must not contain path separators or control characters"
                .to_owned()
        })
    }
}

/// Palette (color theme) name (non-empty string)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct PaletteName(String);

impl PaletteName {
    /// Default palette name
    pub const DEFAULT: &'static str = "default";

    /// Create a new `PaletteName` if the value is valid.
    ///
    /// Returns `None` when the value is empty, contains path separators or
    /// `..`, or contains control characters.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        if is_safe_config_name(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PaletteName {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl fmt::Display for PaletteName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for PaletteName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            "palette name must be non-empty and must not contain path separators or control characters"
                .to_owned()
        })
    }
}

/// Directory segment display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DirectoryDisplay {
    /// Show only the current directory name.
    #[default]
    Basename,
    /// Show the path with intermediate components abbreviated to their first
    /// letter (shell-side; the Rust resolver currently aliases this to `Full`).
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

/// Error returned when parsing an unrecognized `DirectoryDisplay` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDirectoryDisplayError(String);

impl fmt::Display for ParseDirectoryDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unrecognized directory display mode: {}", self.0)
    }
}

impl std::error::Error for ParseDirectoryDisplayError {}

impl FromStr for DirectoryDisplay {
    type Err = ParseDirectoryDisplayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "basename" => Ok(Self::Basename),
            "abbreviated" => Ok(Self::Abbreviated),
            "truncated" => Ok(Self::Truncated),
            "full" => Ok(Self::Full),
            other => Err(ParseDirectoryDisplayError(other.to_owned())),
        }
    }
}

/// Number of path components kept when the directory segment is truncated (1..=255).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize")]
pub struct DirectoryTruncationLength(usize);

impl DirectoryTruncationLength {
    /// Default number of trailing components kept
    pub const DEFAULT: usize = 3;
    /// Minimum allowed length
    pub const MIN: usize = 1;
    /// Maximum allowed length
    pub const MAX: usize = 255;

    /// Create a new `DirectoryTruncationLength` if the value is valid
    #[must_use]
    pub const fn new(value: usize) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Get the inner value
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for DirectoryTruncationLength {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl fmt::Display for DirectoryTruncationLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<usize> for DirectoryTruncationLength {
    type Error = String;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            format!(
                "directory truncation length must be between {} and {}",
                Self::MIN,
                Self::MAX
            )
        })
    }
}

/// Prefix shown before a truncated directory path (rejects control characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(try_from = "String")]
pub struct DirectoryTruncationSymbol(String);

impl DirectoryTruncationSymbol {
    /// Create a new `DirectoryTruncationSymbol` if the value is valid.
    ///
    /// Returns `None` when the value contains ASCII control characters. An
    /// empty string is valid and is the default.
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

/// Error returned when parsing an unrecognized `LanguageDisplay` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLanguageDisplayError(String);

impl fmt::Display for ParseLanguageDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unrecognized language display mode: {}", self.0)
    }
}

impl std::error::Error for ParseLanguageDisplayError {}

impl FromStr for LanguageDisplay {
    type Err = ParseLanguageDisplayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "icon" => Ok(Self::Icon),
            "text" => Ok(Self::Text),
            other => Err(ParseLanguageDisplayError(other.to_owned())),
        }
    }
}

/// Strategy for deciding which languages the prompt surfaces.
///
/// `content` scans the directory's files and ranks
/// languages by prevalence — zero-config but noisy. `markers` shows a language
/// only when its project markers/extensions are present (Starship-style intent).
/// `hybrid` runs the `content` scan but keeps only the languages whose
/// project marker *file* is also present — prevalence ranking, gated by
/// intent (a stray source file's extension alone isn't enough).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DetectionMode {
    /// Recursive content scan, ranked by prevalence (default).
    #[default]
    Content,
    /// Marker-based: show a language only when its markers/extensions exist.
    Markers,
    /// Content scan filtered to languages whose project marker file is present.
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "content" => Ok(Self::Content),
            "markers" => Ok(Self::Markers),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(format!(
                "Invalid language.detection_mode '{other}'. Use 'content', 'markers', or 'hybrid'"
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(Self::All),
            "primary" => Ok(Self::Primary),
            other => other.parse::<usize>().map(Self::Top).map_err(|_| {
                format!("Invalid language.filter '{other}'. Use 'all', 'primary', or a number (e.g. '3')")
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
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]
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
        assert!(err.contains("language.filter"), "got {err}");
        assert!("-1".parse::<LanguageFilter>().is_err());
    }

    #[test]
    fn language_filter_round_trips_via_display() {
        for value in ["all", "primary", "7"] {
            let parsed: LanguageFilter = value.parse().expect("valid");
            assert_eq!(parsed.to_string(), value);
        }
    }
}
