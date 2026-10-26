//! Parsing for the agent's tuning environment variables.
//!
//! One rule per shape so every variable treats whitespace, zero, and garbage
//! the same way. The `parse_*` forms take the raw value so callers and tests
//! need no `std::env::set_var`.

/// Spellings read as `true` by [`parse_flag`], case-insensitive.
const TRUTHY: [&str; 4] = ["1", "true", "yes", "on"];

/// Spellings read as `false` by [`parse_flag`], case-insensitive.
const FALSEY: [&str; 4] = ["0", "false", "no", "off"];

/// A whole number, ignoring surrounding whitespace; `None` when unset or not a `u64`.
#[must_use]
pub(crate) fn parse_u64(raw: Option<&str>) -> Option<u64> {
    raw?.trim().parse().ok()
}

/// [`parse_u64`] where `0` also means unset.
#[must_use]
pub(crate) fn parse_u64_positive(raw: Option<&str>) -> Option<u64> {
    parse_u64(raw).filter(|value| *value > 0)
}

/// A boolean: a truthy spelling is `true`, a falsey one `false`; unset, empty,
/// or anything else is `default`.
#[must_use]
pub(crate) fn parse_flag(raw: Option<&str>, default: bool) -> bool {
    let Some(value) = raw.map(str::trim) else {
        return default;
    };
    if TRUTHY.iter().any(|word| value.eq_ignore_ascii_case(word)) {
        true
    } else if FALSEY.iter().any(|word| value.eq_ignore_ascii_case(word)) {
        false
    } else {
        default
    }
}

/// Value of `name` as UTF-8; `None` when unset or not valid Unicode.
#[must_use]
pub(crate) fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// [`parse_u64_positive`] of the variable `name`.
#[must_use]
pub(crate) fn env_u64_positive(name: &str) -> Option<u64> {
    parse_u64_positive(env_string(name).as_deref())
}

/// [`parse_flag`] of the variable `name`.
#[must_use]
pub(crate) fn env_flag(name: &str, default: bool) -> bool {
    parse_flag(env_string(name).as_deref(), default)
}

#[cfg(test)]
mod tests {
    use super::{parse_flag, parse_u64, parse_u64_positive};

    #[test]
    fn parse_u64_handles_unset_empty_invalid_and_valid() {
        assert_eq!(parse_u64(None), None);
        assert_eq!(parse_u64(Some("")), None);
        assert_eq!(parse_u64(Some("abc")), None);
        assert_eq!(parse_u64(Some("-1")), None);
        assert_eq!(parse_u64(Some("0")), Some(0));
        assert_eq!(parse_u64(Some(" 25 ")), Some(25));
    }

    #[test]
    fn parse_u64_positive_rejects_zero() {
        assert_eq!(parse_u64_positive(None), None);
        assert_eq!(parse_u64_positive(Some("")), None);
        assert_eq!(parse_u64_positive(Some("nope")), None);
        assert_eq!(parse_u64_positive(Some("0")), None);
        assert_eq!(parse_u64_positive(Some("7")), Some(7));
    }

    #[test]
    fn parse_flag_falls_back_to_default_for_unset_empty_and_garbage() {
        for default in [true, false] {
            assert_eq!(parse_flag(None, default), default);
            assert_eq!(parse_flag(Some(""), default), default);
            assert_eq!(parse_flag(Some("maybe"), default), default);
        }
    }

    #[test]
    fn parse_flag_reads_known_spellings_case_insensitively() {
        for word in ["1", "true", "YES", " On "] {
            assert!(parse_flag(Some(word), false), "{word}");
        }
        for word in ["0", "False", "no", "OFF"] {
            assert!(!parse_flag(Some(word), true), "{word}");
        }
    }
}
