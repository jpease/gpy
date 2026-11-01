# SPDX-License-Identifier: GPL-3.0-or-later

# Emit resolved paths as `key=value` in the agent's key order; diffed against
# `gpy debug paths --format kv` by tests/fish/path_parity.test.fish.
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
    # Fish never locates theme files; the agent delivers a rendered export.
    echo "theme_dir=<unimplemented>"
end
