//! Shared plumbing for the validated string newtypes of the plugin model.

use crate::Error;

/// The message of an [`Error::Invalid`] without its `Invalid operation:`
/// prefix, so nested contexts read as one sentence.
pub(crate) fn detail(error: Error) -> String {
    match error {
        Error::Invalid { message } => message,
        other => other.to_string(),
    }
}

/// Implement `Deserialize` through a validating `new(&str) -> Result<Self>`.
///
/// The one-argument form reports the bare message; `keep_prefix` keeps the
/// `Invalid operation:` prefix of the validation error.
macro_rules! deserialize_via_new {
    ($ty:ty) => {
        $crate::plugin::newtype::deserialize_via_new!($ty, |e| $crate::plugin::newtype::detail(e));
    };
    ($ty:ty, keep_prefix) => {
        $crate::plugin::newtype::deserialize_via_new!($ty, |e| e);
    };
    ($ty:ty, $message:expr) => {
        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
                Self::new(&raw).map_err(|e| serde::de::Error::custom(($message)(e)))
            }
        }
    };
}

/// Implement `Display` for a tuple newtype over a string.
macro_rules! display_inner {
    ($ty:ty) => {
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

pub(crate) use deserialize_via_new;
pub(crate) use display_inner;
