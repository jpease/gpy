//! Display metadata (icons, colors, aliases, marker files) per language.

/// Metadata for a supported programming language
pub struct LanguageMeta {
    /// Canonical name (used for internal logic and config keys)
    pub canonical: &'static str,
    /// List of aliases (including detector language names and file extensions)
    pub aliases: &'static [&'static str],
    /// Key used for looking up the icon in `LanguageIcons`
    pub icon_key: &'static str,
    /// Marker filenames (case-insensitive) signalling this language in `DetectionMode::Markers`.
    pub marker_files: &'static [&'static str],
    /// Marker file extensions (no dot) for `DetectionMode::Markers`.
    pub marker_extensions: &'static [&'static str],
}

/// Centralized definition of all supported languages
pub const LANGUAGES: &[LanguageMeta] = &[
    LanguageMeta {
        canonical: "rust",
        aliases: &["rust", "rs"],
        icon_key: "rust",
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    LanguageMeta {
        canonical: "python",
        aliases: &["python", "py"],
        icon_key: "python",
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
        marker_files: &["package.json", "tsconfig.json"],
        marker_extensions: &["js", "mjs", "cjs", "ts", "jsx", "tsx"],
    },
    LanguageMeta {
        canonical: "go",
        aliases: &["go", "golang"],
        icon_key: "go",
        marker_files: &["go.mod", "go.sum"],
        marker_extensions: &["go"],
    },
    LanguageMeta {
        canonical: "java",
        aliases: &["java"],
        icon_key: "java",
        marker_files: &["pom.xml", "build.gradle", "build.gradle.kts"],
        marker_extensions: &["java"],
    },
    LanguageMeta {
        canonical: "ruby",
        aliases: &["ruby", "rb"],
        icon_key: "ruby",
        marker_files: &["Gemfile", "Rakefile", ".ruby-version"],
        marker_extensions: &["rb"],
    },
    LanguageMeta {
        canonical: "swift",
        aliases: &["swift"],
        icon_key: "swift",
        marker_files: &["Package.swift"],
        marker_extensions: &["swift"],
    },
    LanguageMeta {
        canonical: "elixir",
        aliases: &["elixir", "ex"],
        icon_key: "elixir",
        marker_files: &["mix.exs"],
        marker_extensions: &["ex", "exs"],
    },
    LanguageMeta {
        canonical: "php",
        aliases: &["php"],
        icon_key: "php",
        marker_files: &["composer.json"],
        marker_extensions: &["php"],
    },
    LanguageMeta {
        canonical: "csharp",
        aliases: &["csharp", "c#", "cs"],
        icon_key: "csharp",
        marker_files: &[],
        marker_extensions: &["cs", "csproj", "sln"],
    },
    LanguageMeta {
        canonical: "cpp",
        aliases: &["cpp", "c++", "cplusplus"],
        icon_key: "cpp",
        marker_files: &["CMakeLists.txt"],
        marker_extensions: &["cpp", "cxx", "cc", "hpp", "hh"],
    },
    LanguageMeta {
        canonical: "c",
        aliases: &["c"],
        icon_key: "c",
        marker_files: &["Makefile"],
        marker_extensions: &["c", "h"],
    },
    LanguageMeta {
        canonical: "erlang",
        aliases: &["erlang"],
        icon_key: "erlang",
        marker_files: &["rebar.config"],
        marker_extensions: &["erl"],
    },
    LanguageMeta {
        canonical: "fish",
        aliases: &["fish"],
        icon_key: "fish",
        marker_files: &[],
        marker_extensions: &["fish"],
    },
];

/// Find the `LanguageMeta` matching `alias` (case-insensitive).
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

/// All [`LanguageMeta::marker_files`] flattened, built once. Shared with the
/// watcher's invalidation so there is one list. Raw case; callers fold case.
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
    fn alias_lookup_is_case_insensitive_and_unknown_aliases_miss() {
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
            assert_eq!(
                get_canonical_name(alias),
                *expected_canonical,
                "alias: {alias:?}"
            );
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
}
