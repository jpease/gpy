//! The built-in prompt segments GPY ships (single source of truth).
//!
//! Lives in `plugin` so the `gpy-core` manifest can use it without depending on `commands`.

use std::fmt;

/// A built-in prompt segment name, distinct from plugin [`super::SegmentName`]s.
///
/// Parse at the input boundary via [`BuiltinSegment::try_from`]; never compare bare strings.
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
/// `username` and `hostname` are listed but not enabled by default.
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
