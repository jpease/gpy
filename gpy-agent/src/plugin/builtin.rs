//! The built-in prompt segments GPY ships (single source of truth).
//!
//! Lives in `plugin` so the first-party manifest can use it without depending on `commands`.

use std::fmt;

/// Declares [`BuiltinSegment`] and everything derived from its one table: the
/// canonical lowercase name (config/CLI spelling), parsing and [`BUILTIN_ORDER`].
macro_rules! builtin_segments {
    ($($(#[$doc:meta])* $variant:ident => $name:literal,)+) => {
        /// A built-in prompt segment name, distinct from plugin [`super::SegmentName`]s.
        ///
        /// Parse at the input boundary via [`BuiltinSegment::try_from`]; never compare bare strings.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum BuiltinSegment {
            $($(#[$doc])* $variant,)+
        }

        /// Canonical ordering for the builtin segments.
        ///
        /// `username` and `hostname` are listed but not enabled by default.
        pub const BUILTIN_ORDER: &[BuiltinSegment] = &[$(BuiltinSegment::$variant),+];

        impl BuiltinSegment {
            /// The segment's canonical lowercase name, matching config/CLI spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }

        impl TryFrom<&str> for BuiltinSegment {
            type Error = NotBuiltin;

            fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
                match value {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(NotBuiltin),
                }
            }
        }
    };
}

builtin_segments! {
    /// Current time.
    Clock => "clock",
    /// Duration of the last command.
    Duration => "duration",
    /// Detected language toolchains.
    Language => "language",
    /// Working directory.
    Directory => "directory",
    /// Git repository status.
    Git => "git",
    /// Last command's exit status.
    Status => "status",
    /// Current user name.
    Username => "username",
    /// Host name.
    Hostname => "hostname",
}

impl fmt::Display for BuiltinSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The name is not a builtin segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotBuiltin;

impl fmt::Display for NotBuiltin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a builtin segment")
    }
}

impl std::error::Error for NotBuiltin {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_over_the_canonical_order() {
        for &segment in BUILTIN_ORDER {
            assert_eq!(BuiltinSegment::try_from(segment.as_str()), Ok(segment));
        }
        assert_eq!(BuiltinSegment::try_from("nope"), Err(NotBuiltin));
    }
}
