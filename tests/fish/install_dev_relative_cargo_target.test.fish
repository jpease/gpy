#!/usr/bin/env fish
# tests/fish/install_dev_relative_cargo_target.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish builds inside gpy-agent/, where cargo resolves a relative
# CARGO_TARGET_DIR, but the install step runs from the repo root. _release_dir
# must therefore name the directory cargo actually wrote to: a relative value
# is pinned against gpy-agent/, an absolute one and an unset one are unchanged.
#
# The function is sourced from the real install-dev.fish (--files-only against
# a throwaway HOME), as install_dev_relative_xdg.test.fish does.

set -l script_dir (dirname (status filename))
set -g repo_root (realpath "$script_dir/../..")
source "$repo_root/tests/lib/test_helpers.fish"
init_test_env

set -g sandbox (mktemp -d)
set -g home_dir "$sandbox/home"
mkdir -p "$home_dir"
set -g safe_path (string join : /usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin /usr/local/bin)

# Runs _release_dir in a fresh shell with CARGO_TARGET_DIR set to `value`, or
# left out of the environment when `value` is the word `unset`.
function resolve_release_dir --argument-names value
    set -l driver "$sandbox/driver.fish"
    set -l lines \
        "cd '$repo_root'" \
        "set -gx HOME '$home_dir'" \
        "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
        "set -gx PATH '$safe_path'" \
        "set -e CARGO_TARGET_DIR"
    if test "$value" != unset
        set -a lines "set -gx CARGO_TARGET_DIR "(string escape -- "$value")
    end
    set -a lines \
        "source install-dev.fish --files-only --allow-shadow >/dev/null 2>&1" \
        "echo resolved=(_release_dir)"
    printf '%s\n' $lines >"$driver"
    set -l out (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver" 2>&1)
    string replace -rf '^resolved=' '' -- $out
end

set -l cases \
    "a relative value|target|$repo_root/gpy-agent/target/release" \
    "an absolute value|/abs/dir|/abs/dir/release" \
    "an unset value|unset|gpy-agent/target/release"

for entry in $cases
    set -l parts (string split '|' -- $entry)
    set -l got (resolve_release_dir "$parts[2]")
    if test "$got" = "$parts[3]"
        pass "_release_dir with $parts[1] resolves to $parts[3]"
    else
        fail "_release_dir with $parts[1] resolved to '$got', expected '$parts[3]'"
    end
end

rm -rf "$sandbox"
test_finish "_release_dir follows a relative CARGO_TARGET_DIR"
