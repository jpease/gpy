//! Display metadata (icons, colors, aliases, marker files) per language.

/// Metadata for a supported programming language
pub struct LanguageMeta {
    /// Canonical name (used for internal logic and config keys)
    pub canonical: &'static str,
    /// List of aliases (including detector language names and file extensions)
    pub aliases: &'static [&'static str],
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
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    LanguageMeta {
        canonical: "python",
        aliases: &["python", "py"],
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
        marker_files: &["package.json", "tsconfig.json"],
        marker_extensions: &["js", "mjs", "cjs", "ts", "jsx", "tsx"],
    },
    LanguageMeta {
        canonical: "go",
        aliases: &["go", "golang"],
        marker_files: &["go.mod", "go.sum"],
        marker_extensions: &["go"],
    },
    LanguageMeta {
        canonical: "java",
        aliases: &["java"],
        marker_files: &["pom.xml", "build.gradle", "build.gradle.kts"],
        marker_extensions: &["java"],
    },
    LanguageMeta {
        canonical: "ruby",
        aliases: &["ruby", "rb"],
        marker_files: &["Gemfile", "Rakefile", ".ruby-version"],
        marker_extensions: &["rb"],
    },
    LanguageMeta {
        canonical: "swift",
        aliases: &["swift"],
        marker_files: &["Package.swift"],
        marker_extensions: &["swift"],
    },
    LanguageMeta {
        canonical: "elixir",
        aliases: &["elixir", "ex"],
        marker_files: &["mix.exs"],
        marker_extensions: &["ex", "exs"],
    },
    LanguageMeta {
        canonical: "php",
        aliases: &["php"],
        marker_files: &["composer.json"],
        marker_extensions: &["php"],
    },
    LanguageMeta {
        canonical: "csharp",
        aliases: &["csharp", "c#", "cs"],
        marker_files: &[],
        marker_extensions: &["cs", "csproj", "sln"],
    },
    LanguageMeta {
        canonical: "cpp",
        aliases: &["cpp", "c++", "cplusplus"],
        marker_files: &["CMakeLists.txt"],
        marker_extensions: &["cpp", "cxx", "cc", "hpp", "hh"],
    },
    LanguageMeta {
        canonical: "c",
        aliases: &["c"],
        marker_files: &[],
        marker_extensions: &["c", "h"],
    },
    LanguageMeta {
        canonical: "erlang",
        aliases: &["erlang"],
        marker_files: &["rebar.config"],
        marker_extensions: &["erl"],
    },
    LanguageMeta {
        canonical: "fish",
        aliases: &["fish"],
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

/// Marker files that count as evidence for a language only when the directory
/// holds no evidence for any other language: `(file name, language)`.
pub(crate) const WEAK_MARKER_FILES: &[(&str, &str)] = &[("Makefile", "c")];

/// All [`LanguageMeta::marker_files`] and the [`WEAK_MARKER_FILES`] names
/// flattened, built once. Shared with the watcher's invalidation and the
/// shells' pre-filter so there is one list. Raw case; callers fold case.
#[must_use]
pub(crate) fn marker_file_names() -> &'static std::collections::HashSet<&'static str> {
    static MARKER_FILES: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    MARKER_FILES.get_or_init(|| {
        LANGUAGES
            .iter()
            .flat_map(|language| language.marker_files.iter().copied())
            .chain(WEAK_MARKER_FILES.iter().map(|(file, _)| *file))
            .collect()
    })
}

/// All [`LanguageMeta::marker_files`] flattened, built once; excludes weak
/// markers, which never make a directory a project root.
#[must_use]
pub(crate) fn project_root_marker_names() -> &'static std::collections::HashSet<&'static str> {
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
}
