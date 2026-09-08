# Developer helpers for GPY language segment
# These functions are optional but useful for debugging and support.

# Print current language-segment configuration and registry entries.
function gpy_lang_status
    set -l debounce_ms $GPY_LANG_SIG_DEBOUNCE_MS
    if not set -q debounce_ms[1]
        set debounce_ms 0
    end
    echo "SIG_MODE=$GPY_LANG_SIG_MODE, PRESET=$GPY_LANG_SIG_PRESET, DEBOUNCE=$debounce_ms"
    test -n "$GPY_LANG_ECOSYSTEMS"; and echo "ECO=$GPY_LANG_ECOSYSTEMS"
    test -n "$GPY_LANG_SIG_INCLUDE$GPY_LANG_SIG_EXCLUDE"; and echo "INCL=$GPY_LANG_SIG_INCLUDE  EXCL=$GPY_LANG_SIG_EXCLUDE"
    command -q __lang_dump_registry; and __lang_dump_registry
end

# Optional: provide no-op fallbacks for section functions when theme isn't loaded.
# This prevents errors if someone sources the language segment standalone.
functions -q gpy_section_start; and functions -q gpy_section_end; or begin
    function gpy_section_start --argument-names color fg text
        # no-op fallback when theming isn't loaded
        echo -n ""
    end
    function gpy_section_end --argument-names color
        echo -n ""
    end
end
