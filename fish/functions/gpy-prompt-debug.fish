# SPDX-License-Identifier: GPL-3.0-or-later

function __gpy_validate_installation
    set -l errors 0

    echo "Validating GPY installation..."

    # Install dir is the one init.fish loaded from, not ~/.config.
    set -l install_dir $__gpy_core_dir/..

    # Every file init.fish and conf.d/gpy_init.fish need: the files init.fish
    # sources directly, the list it sources the rest from, the script segment
    # runner, and the loader.
    set -l needed core/flags.fish core/util.fish core/defaults.fish core/init.fish
    for file in $__gpy_core_files
        set -a needed core/$file
    end
    set -a needed core/g_runner.bash conf.d/gpy_init.fish
    for file in $needed
        if test -f $install_dir/$file
            echo "✓ File: $file"
        else
            echo "✗ Missing file: $file"
            set errors (math $errors + 1)
        end
    end

    echo ""
    echo "Segments:"
    for segment in $__enabled_segments
        set -l file $install_dir/segments/$segment.fish
        set -l plugin_file_var (__gpy_plugin_segment_file_var "$segment")
        if test -f $file; or begin
                set -q $plugin_file_var; and test -f $$plugin_file_var
            end
            if functions -q segment_{$segment}_detect; and functions -q segment_{$segment}_render
                echo "✓ $segment (complete)"
            else
                echo "! $segment (missing functions)"
                set errors (math $errors + 1)
            end
        else
            echo "✗ $segment (file missing)"
            set errors (math $errors + 1)
        end
    end

    # Built-in themes live in the binaries; `gpy` sits beside `gpy-agent`.
    echo ""
    if set -q __gpy_theme_name
        echo "✓ Active theme: $__gpy_theme_name"
        set -l agent_binary (__gpy_resolve_agent_binary)
        set -l cli (path dirname -- "$agent_binary")/gpy
        if test -n "$agent_binary"; and test -x "$cli"
            if "$cli" theme validate >/dev/null 2>&1
                echo "✓ Theme validates"
            else
                echo "✗ Theme validation failed: $__gpy_theme_name"
                set errors (math $errors + 1)
            end
        end
    else
        echo "✗ No active theme (__gpy_theme_name is unset)"
        set errors (math $errors + 1)
    end

    echo ""
    if test $errors -eq 0
        echo "Installation is valid."
        return 0
    else
        echo "Found $errors errors."
        return 1
    end
end

function __gpy_benchmark_segments
    echo "Benchmarking prompt segments..."

    for segment in $__enabled_segments
        if functions -q segment_{$segment}_detect; and functions -q segment_{$segment}_render
            set -l start (__gpy_get_time_ms)

            if segment_{$segment}_detect
                segment_{$segment}_render >/dev/null
            end

            set -l end (__gpy_get_time_ms)
            set -l duration (math $end - $start)
            echo "  $segment: "$duration"ms"
        end
    end
end

function gpy-prompt-debug --description 'GPY debug helpers' --argument-names action
    switch $action
        case validate
            __gpy_validate_installation
        case benchmark
            __gpy_benchmark_segments
        case cache
            echo "Cache status:"
            echo "  Instant cache dir: "(__gpy_instant_cache_dir)
            echo "  Registered: $__gpy_registered"
            echo "  Char cache key: $__gpy_char_cache_key"
            echo "  Dir cache key: $__gpy_dir_cache_key"
        case vars
            echo "Configuration variables:"
            echo "  Theme: $__gpy_theme_name"
            echo "  Enabled segments: $__enabled_segments"
            echo "  Performance:"
            echo "    Language cache TTL: $GPY_LANGUAGE_CACHE_TTL_SECONDS"
            echo "    Duration threshold: $__duration_threshold_ms"
        case ""
            echo "Usage: gpy-prompt-debug [validate|benchmark|cache|vars]"
        case '*'
            echo "Unknown debug action: $action"
    end
end
