# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# DEBUG AND VALIDATION UTILITIES
# ============================================================================

function __prompt_validate_installation
    set -l errors 0

    echo "🔍 Validating GPY installation..."

    # Install dir is derived from this file's own location, not ~/.config.
    set -l install_dir $__gpy_core_dir/..

    # Check core files
    set -l core_files constants.fish util.fish renderer.fish init.fish ipc.fish
    for file in $core_files
        set -l path $install_dir/core/$file
        if test -f $path
            echo "✅ Core file: $file"
        else
            echo "❌ Missing core file: $file"
            set errors (math $errors + 1)
        end
    end

    # Check segments
    echo ""
    echo "📊 Segment validation:"
    for segment in $__enabled_segments
        set -l file $install_dir/segments/$segment.fish
        set -l plugin_file_var __gpy_plugin_segment_file_(string lower (string replace -a '-' '_' -- $segment))
        if test -f $file; or begin
                set -q $plugin_file_var; and test -f $$plugin_file_var
            end
            if functions -q segment_{$segment}_detect; and functions -q segment_{$segment}_render
                echo "✅ $segment (complete)"
            else
                echo "⚠️  $segment (missing functions)"
                set errors (math $errors + 1)
            end
        else
            echo "❌ $segment (file missing)"
            set errors (math $errors + 1)
        end
    end

    # Check theme: built-in themes are embedded in the agent binary, so ask
    # the agent to validate the active theme instead of looking for a file.
    echo ""
    if set -q __gpy_theme_name
        echo "✅ Active theme: $__gpy_theme_name"
        set -l agent_binary (__gpy_resolve_agent_binary)
        if test -n "$agent_binary"
            if "$agent_binary" theme validate >/dev/null 2>&1
                echo "✅ Theme validates"
            else
                echo "❌ Theme validation failed: $__gpy_theme_name"
                set errors (math $errors + 1)
            end
        end
    else
        echo "❌ No active theme (__gpy_theme_name is unset)"
        set errors (math $errors + 1)
    end

    echo ""
    if test $errors -eq 0
        echo "🎉 Installation is valid!"
        return 0
    else
        echo "💥 Found $errors errors"
        return 1
    end
end

function __prompt_benchmark_segments
    echo "⏱️  Benchmarking prompt segments..."

    for segment in $__enabled_segments
        if functions -q segment_{$segment}_detect; and functions -q segment_{$segment}_render
            # Get start time in milliseconds (cross-platform)
            set -l start (__gpy_get_time_ms)

            if segment_{$segment}_detect
                segment_{$segment}_render >/dev/null
            end

            # Get end time in milliseconds
            set -l end (__gpy_get_time_ms)
            set -l duration (math $end - $start)
            echo "  $segment: {$duration}ms"
        end
    end
end

function prompt_debug --argument-names action
    switch $action
        case validate
            __prompt_validate_installation
        case benchmark
            __prompt_benchmark_segments
        case cache
            echo "🗄️  Cache status:"
            echo "  Instant cache dir: "(__gpy_instant_cache_dir)
            echo "  Registered: $__gpy_registered"
            echo "  Agent backoff until: $__gpy_agent_backoff_until"
            echo "  Char cache key: $__gpy_char_cache_key"
            echo "  Dir cache key: $__gpy_dir_cache_key"
        case vars
            echo "🔧 Configuration variables:"
            echo "  Theme: $__gpy_theme_name"
            echo "  Enabled segments: $__enabled_segments"
            echo "  Performance:"
            echo "    Language cache TTL: $GPY_LANGUAGE_CACHE_TTL_SECONDS"
            echo "    Git check interval: $__git_check_interval"
            echo "    Duration threshold: $__duration_threshold"
        case ""
            echo "Usage: prompt_debug [validate|benchmark|cache|vars]"
        case '*'
            echo "Unknown debug action: $action"
    end
end

# ============================================================================
# PATH PARITY DUMP
# ============================================================================

# Emit every path this shell resolves from the environment, as `key=value`
# lines in the agent's key order.
#
# Counterpart to `gpy debug paths --format kv`.
# `tests/fish/path_parity.test.fish` runs both under a matrix of synthetic
# environments and diffs the full maps, so path resolution cannot drift
# between the agent and this shell (#476).
#
# Every value comes from the resolver the prompt itself calls; nothing here
# re-derives a precedence rule, because a reimplementation would agree with
# itself and prove nothing. `cache_root` is the lexical parent of the
# instant-prompt cache directory — a derivation of a real resolver's output,
# not a second copy of the precedence.
#
# Two sentinels carry the cases a path string cannot: `<unimplemented>` for a
# key this shell has no resolver for, and `<unresolved>` for a resolver that
# ran and produced nothing. The harness treats them differently — the first is
# a declared property of this shell, the second is a comparable value.
function __gpy_debug_paths --description 'Dump every GPY path resolved from the environment'
    set -l runtime_root (__gpy_runtime_root)
    test -n "$runtime_root"; or set runtime_root '<unresolved>'

    set -l socket (__gpy_ipc_endpoint)
    test -n "$socket"; or set socket '<unresolved>'

    set -l registry (__gpy_shell_registry_dir)
    test -n "$registry"; or set registry '<unresolved>'

    set -l instant_dir (__gpy_instant_cache_dir)
    set -l cache_root '<unresolved>'
    if test -n "$instant_dir"
        set cache_root (path dirname -- "$instant_dir")
    else
        set instant_dir '<unresolved>'
    end

    set -l theme_export (__gpy_theme_export_cache_path)
    test -n "$theme_export"; or set theme_export '<unresolved>'

    set -l config_path (__gpy_locate_config_path)
    test -n "$config_path"; or set config_path '<unresolved>'

    set -l candidates (string join '|' (__gpy_user_config_candidates))
    test -n "$candidates"; or set candidates '<unresolved>'

    echo "runtime_root=$runtime_root"
    echo "socket=$socket"
    echo "shell_registry_dir=$registry"
    echo "cache_root=$cache_root"
    echo "instant_prompts_dir=$instant_dir"
    echo "theme_export_file=$theme_export"
    echo "config_path=$config_path"
    echo "config_candidates=$candidates"
    # Themes are resolved by the agent and delivered to this shell as an
    # already-rendered export; Fish never locates a theme file itself.
    echo "theme_dir=<unimplemented>"
end
