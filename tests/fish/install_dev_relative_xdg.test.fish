#!/usr/bin/env fish
# tests/fish/install_dev_relative_xdg.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish's _config_home resolves the config root as
# `paths::config_root_for` does: an XDG_CONFIG_HOME is honoured only when it
# starts with `/`. Relative, empty, `~` and Windows-shaped values fall back to
# $HOME/.config.
#
# The function is sourced from the real install-dev.fish (run with
# --files-only against a throwaway HOME so sourcing has no side effects on
# this machine), the way install_dev_shadowing_binaries.test.fish does. The
# driver runs under `env -i` with only system dirs on PATH.

set -l script_dir (dirname (status filename))
set -g repo_root (realpath "$script_dir/../..")
source "$repo_root/tests/lib/test_helpers.fish"
init_test_env

set -g sandbox (mktemp -d)
set -g home_dir "$sandbox/home"
mkdir -p "$home_dir"
set -g safe_path (string join : /usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin /usr/local/bin)

# Runs _config_home in a fresh shell with XDG_CONFIG_HOME set to `value`, or
# left out of the environment when `value` is the word `unset`.
function resolve_config_home --argument-names value
    set -l driver "$sandbox/driver.fish"
    set -l lines \
        "cd '$repo_root'" \
        "set -gx HOME '$home_dir'" \
        "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
        "set -gx PATH '$safe_path'" \
        "set -e XDG_CONFIG_HOME"
    if test "$value" != unset
        set -a lines "set -gx XDG_CONFIG_HOME "(string escape -- "$value")
    end
    set -a lines \
        "source install-dev.fish --files-only --allow-shadow >/dev/null 2>&1" \
        "echo resolved=(_config_home)"
    printf '%s\n' $lines >"$driver"
    set -l out (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver" 2>&1)
    string replace -rf '^resolved=' '' -- $out
end

set -l cases \
    "unset|unset|$home_dir/.config" \
    "an empty value||$home_dir/.config" \
    "a relative value|rel/conf|$home_dir/.config" \
    "a dot-relative value|./conf|$home_dir/.config" \
    "a tilde value|~/conf|$home_dir/.config" \
    "a Windows drive value|C:/conf|$home_dir/.config" \
    "a UNC value|\\\\server\\share|$home_dir/.config" \
    "a leading-space value| /conf|$home_dir/.config" \
    "an absolute value|$sandbox/xdg|$sandbox/xdg"

for entry in $cases
    set -l parts (string split '|' -- $entry)
    set -l label $parts[1]
    set -l value $parts[2]
    set -l expected $parts[3]
    test "$label" = unset; and set label "an unset XDG_CONFIG_HOME"
    set -l got (resolve_config_home "$value")
    if test "$got" = "$expected"
        pass "_config_home with $label resolves to $expected"
    else
        fail "_config_home with $label resolved to '$got', expected '$expected'"
    end
end

rm -rf "$sandbox"
test_finish "_config_home ignores a relative XDG_CONFIG_HOME"
