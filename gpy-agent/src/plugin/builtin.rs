//! The built-in prompt segments GPY ships.
//!
//! This is the single source of truth for builtin segment identity. It lives
//! in `plugin` (not `commands`) so the first-party `gpy-core` manifest built
//! in [`super::registry`] can derive its `provided_segments` from
//! [`BUILTIN_ORDER`] without a `plugin -> commands` dependency; `commands`
//! already depends on `plugin`, so both sides can use it.

use std::fmt;

/// A built-in prompt segment name — the fixed set of segments GPY ships,
/// distinct from user- or plugin-provided segment names (see
/// [`super::SegmentName`]).
///
/// No bare string literal should name a builtin segment for
/// dispatch/comparison purposes anywhere else in this crate. Parse into this
/// type at the boundary where a segment name first enters from
/// config/CLI/user input (via [`BuiltinSegment::try_from`]), then work with
/// the typed value from there on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltinSegment {
    Clock,
    Duration,
    Language,
    Directory,
    Git,
    Status,
    Username,
    Hostname,
}

impl BuiltinSegment {
    /// The segment's canonical lowercase name, matching config/CLI spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Duration => "duration",
            Self::Language => "language",
            Self::Directory => "directory",
            Self::Git => "git",
            Self::Status => "status",
            Self::Username => "username",
            Self::Hostname => "hostname",
        }
    }
}

impl fmt::Display for BuiltinSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for BuiltinSegment {
    type Error = ();

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "clock" => Ok(Self::Clock),
            "duration" => Ok(Self::Duration),
            "language" => Ok(Self::Language),
            "directory" => Ok(Self::Directory),
            "git" => Ok(Self::Git),
            "status" => Ok(Self::Status),
            "username" => Ok(Self::Username),
            "hostname" => Ok(Self::Hostname),
            _ => Err(()),
        }
    }
}

/// Canonical ordering for the builtin segments.
///
/// Used throughout the wizard/CLI (e.g. rebuilding `ui.enabled_segments`) and
/// for the first-party `gpy-core` manifest's `provided_segments`.
///
/// `username` and `hostname` are opt-in: they are listed here but are not in
/// `config::defaults::default_enabled_segments`.
pub const BUILTIN_ORDER: &[BuiltinSegment] = &[
    BuiltinSegment::Clock,
    BuiltinSegment::Duration,
    BuiltinSegment::Language,
    BuiltinSegment::Directory,
    BuiltinSegment::Git,
    BuiltinSegment::Status,
    BuiltinSegment::Username,
    BuiltinSegment::Hostname,
];
