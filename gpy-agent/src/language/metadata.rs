//! Static metadata for languages supported by the prompt.
//!
//! Detection returns language names from `gengo-language` and version probes; this
//! module maps those names to display metadata such as icons, colors, aliases,
//! and known project files. The formatter and config layers use these values to
//! present language segments consistently.

/// Metadata for a supported programming language
pub struct LanguageMeta {
    /// Canonical name (used for internal logic and config keys)
    pub canonical: &'static str,
    /// List of aliases (including detector language names and file extensions)
    pub aliases: &'static [&'static str],
    /// Key used for looking up the icon in `LanguageIcons`
    pub icon_key: &'static str,
    /// Default background color
    pub default_color: Option<&'static str>,
    /// Project marker filenames (case-insensitive) whose presence in a directory
    /// signals this language under `DetectionMode::Markers` (e.g. `Cargo.toml`).
    pub marker_files: &'static [&'static str],
    /// File extensions (without the dot) whose presence in a directory signals
    /// this language under `DetectionMode::Markers` (e.g. `rs`).
    pub marker_extensions: &'static [&'static str],
}

/// Centralized definition of all supported languages
pub const LANGUAGES: &[LanguageMeta] = &[
    LanguageMeta {
        canonical: "rust",
        aliases: &["rust", "rs"],
        icon_key: "rust",
        default_color: Some("red"),
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    LanguageMeta {
        canonical: "python",
        aliases: &["python", "py"],
        icon_key: "python",
        default_color: Some("blue"),
        marker_files: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "Pipfile",
            "tox.ini",
        ],
        marker_extensions: &["py"],
    },
    LanguageMeta {
        canonical: "node",
        aliases: &["node", "nodejs", "javascript", "js", "typescript", "ts"],
        icon_key: "node",
        default_color: Some("green"),
        marker_files: &["package.json", "tsconfig.json"],
        marker_extensions: &["js", "mjs", "cjs", "ts", "jsx", "tsx"],
    },
    LanguageMeta {
        canonical: "go",
        aliases: &["go", "golang"],
        icon_key: "go",
        default_color: Some("cyan"),
        marker_files: &["go.mod", "go.sum"],
        marker_extensions: &["go"],
    },
    LanguageMeta {
        canonical: "java",
        aliases: &["java"],
        icon_key: "java",
        default_color: Some("blue"),
        marker_files: &["pom.xml", "build.gradle", "build.gradle.kts"],
        marker_extensions: &["java"],
    },
    LanguageMeta {
        canonical: "ruby",
        aliases: &["ruby", "rb"],
        icon_key: "ruby",
        default_color: Some("red"),
        marker_files: &["Gemfile", "Rakefile", ".ruby-version"],
        marker_extensions: &["rb"],
    },
    LanguageMeta {
        canonical: "swift",
        aliases: &["swift"],
        icon_key: "swift",
        default_color: Some("red"),
        marker_files: &["Package.swift"],
        marker_extensions: &["swift"],
    },
    LanguageMeta {
        canonical: "elixir",
        aliases: &["elixir", "ex"],
        icon_key: "elixir",
        default_color: Some("magenta"),
        marker_files: &["mix.exs"],
        marker_extensions: &["ex", "exs"],
    },
    LanguageMeta {
        canonical: "php",
        aliases: &["php"],
        icon_key: "php",
        default_color: Some("blue"),
        marker_files: &["composer.json"],
        marker_extensions: &["php"],
    },
    LanguageMeta {
        canonical: "csharp",
        aliases: &["csharp", "c#", "cs"],
        icon_key: "csharp",
        default_color: Some("magenta"),
        marker_files: &[],
        marker_extensions: &["cs", "csproj", "sln"],
    },
    LanguageMeta {
        canonical: "cpp",
        aliases: &["cpp", "c++", "cplusplus"],
        icon_key: "cpp",
        default_color: Some("blue"),
        marker_files: &["CMakeLists.txt"],
        marker_extensions: &["cpp", "cxx", "cc", "hpp", "hh"],
    },
    LanguageMeta {
        canonical: "c",
        aliases: &["c"],
        icon_key: "c",
        default_color: Some("blue"),
        marker_files: &["Makefile"],
        marker_extensions: &["c", "h"],
    },
    LanguageMeta {
        canonical: "erlang",
        aliases: &["erlang"],
        icon_key: "erlang",
        default_color: Some("red"),
        marker_files: &["rebar.config"],
        marker_extensions: &["erl"],
    },
    LanguageMeta {
        canonical: "fish",
        aliases: &["fish"],
        icon_key: "fish",
        default_color: Some("green"),
        marker_files: &[],
        marker_extensions: &["fish"],
    },
];

/// Find the `LanguageMeta` whose canonical name or alias list matches
/// `alias` (case-insensitive).
#[must_use]
fn lookup(alias: &str) -> Option<&'static LanguageMeta> {
    let alias_lower = alias.to_lowercase();
    LANGUAGES
        .iter()
        .find(|lang| lang.canonical == alias_lower || lang.aliases.contains(&alias_lower.as_str()))
}

/// Get the canonical name for a language (case-insensitive lookup)
#[must_use]
pub fn get_canonical_name(alias: &str) -> Option<&'static str> {
    lookup(alias).map(|lang| lang.canonical)
}

/// Get the icon key for a language (case-insensitive lookup)
#[must_use]
pub fn get_icon_key(alias: &str) -> Option<&'static str> {
    lookup(alias).map(|lang| lang.icon_key)
}

/// Get the default color for a language (case-insensitive lookup)
#[must_use]
pub fn get_default_color(alias: &str) -> Option<&'static str> {
    lookup(alias).and_then(|lang| lang.default_color)
}

/// Every [`LanguageMeta::marker_files`] entry across [`LANGUAGES`], flattened
/// into one set and built once.
///
/// Shared by the language detector's directory-marker detection
/// ([`crate::language::detector::Detector`]) and the watcher's
/// `classify_event` cache-invalidation classification (`watcher/mod.rs`,
/// gpy-agent#602) so a marker file added here automatically also triggers a
/// watcher refresh, with no second list to keep in sync.
///
/// Returns raw, unfolded `&str`s: callers that need case-insensitive
/// comparison (as directory-marker detection does) must fold case
/// themselves, e.g. via `eq_ignore_ascii_case`.
#[must_use]
pub(crate) fn marker_file_names() -> &'static std::collections::HashSet<&'static str> {
    static MARKER_FILES: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    MARKER_FILES.get_or_init(|| {
        LANGUAGES
            .iter()
            .flat_map(|language| language.marker_files.iter().copied())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn lookup_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("rust", Some("rust")),
            ("rs", Some("rust")),
            ("RUST", Some("rust")),
            ("Rs", Some("rust")),
            ("nodejs", Some("node")),
            ("c++", Some("cpp")),
            ("not-a-real-language", None),
            ("", None),
        ];
        for (alias, expected_canonical) in cases {
            let actual = lookup(alias).map(|lang| lang.canonical);
            assert_eq!(actual, *expected_canonical, "alias: {alias:?}");
        }
    }

    #[test]
    fn get_canonical_name_table() {
        let cases: &[(&str, Option<&str>)] =
            &[("rust", Some("rust")), ("RS", Some("rust")), ("nope", None)];
        for (alias, expected) in cases {
            assert_eq!(get_canonical_name(alias), *expected, "alias: {alias:?}");
        }
    }

    #[test]
    fn get_icon_key_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("rust", Some("rust")),
            ("PY", Some("python")),
            ("nope", None),
        ];
        for (alias, expected) in cases {
            assert_eq!(get_icon_key(alias), *expected, "alias: {alias:?}");
        }
    }

    #[test]
    fn get_default_color_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("rust", Some("red")),
            ("CSHARP", Some("magenta")),
            ("nope", None),
        ];
        for (alias, expected) in cases {
            assert_eq!(get_default_color(alias), *expected, "alias: {alias:?}");
        }
    }
}
