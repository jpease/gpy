#!/usr/bin/env fish
# tests/fish/install_dev_remove_conflicting_binaries.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish's _remove_conflicting_binaries deletes files on PATH
# (#649). It must remove only a stale copy of OUR binary under $HOME that
# shadows the install dir, and leave everything else alone:
#   (a) a `gpy-agent` under $HOME earlier on PATH whose --version says
#       `gpy-agent ...` is deleted
#   (b) an unrelated tool that merely shares the name (--version says
#       something else) is kept, with a warning
#   (c) a tool-manager shim (a */shims/* path) is kept
#   (d) a copy AFTER the install dir on PATH is kept (it does not shadow)
#   (e) with the install dir absent from PATH, nothing is deleted (#693)
#   (f) a shadowing copy of ours outside $HOME is kept, with a warning (#693)
#
# The function is sourced from the real install-dev.fish (run with
# --fish-only against a throwaway HOME so sourcing has no side effects on
# this machine), the way install_dev_stop_running_agent.test.fish does.
# Every stub lives under the sandbox, and each driver runs under `env -i`
# with only system dirs on PATH, so an exported `fish_user_paths` or a real
# ~/.local/bin from the caller can never reach a deletion candidate.

set -l script_dir (dirname (status filename))
set -l repo_root (realpath "$script_dir/../..")
source "$repo_root/tests/lib/test_helpers.fish"
init_test_env

# Global: the fail helper below cannot see a script-level local, so a
# `set -l` counter here would stay 0 and the test could never fail.
set -g failures 0
function __gpy_test_fail --argument-names msg
    echo "FAIL: $msg"
    set -g failures (math $failures + 1)
end
function __gpy_test_pass --argument-names msg
    echo "PASS: $msg"
end

set -l sandbox (mktemp -d)
set -l home_dir "$sandbox/home"
mkdir -p "$home_dir"

# PATH layout: early/ (ours, stale, under $HOME) : outside/ (ours, outside
# $HOME) : other/ (unrelated tool) : tools/shims/ (shim) : install/ (the
# target) : late/ (ours, after target).
set -l early "$home_dir/.cargo/bin"
set -l outside "$sandbox/outside"
set -l other "$sandbox/other"
set -l shims "$sandbox/tools/shims"
set -l install "$sandbox/install"
set -l late "$sandbox/late"
# Only on PATH in the install-dir-absent run (e).
set -l off_path_copy "$home_dir/.cargo/bin2"
mkdir -p $early $outside $other $shims $install $late $off_path_copy

function write_stub --argument-names path version_line
    printf '#!/bin/sh\necho "%s"\n' "$version_line" >"$path"
    chmod +x "$path"
end
write_stub "$early/gpy-agent" "gpy-agent 0.0.1"
write_stub "$early/gpy" "gpy 0.0.1"
write_stub "$outside/gpy-agent" "gpy-agent 0.0.1"
write_stub "$other/gpy-agent" "other-tool 1.0"
write_stub "$shims/gpy-agent" "gpy-agent 0.0.1"
write_stub "$install/gpy-agent" "gpy-agent 9.9.9"
write_stub "$install/gpy" "gpy 9.9.9"
write_stub "$late/gpy-agent" "gpy-agent 0.0.2"
write_stub "$off_path_copy/gpy-agent" "gpy-agent 0.0.1"

set -l safe_path (string join : /usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin /usr/local/bin)

# Writes a driver that sources install-dev.fish, sets PATH to the given
# entries, and calls _remove_conflicting_binaries on the install dir.
function write_driver --argument-names driver repo_root home_dir safe_path install
    printf '%s\n' \
        "cd '$repo_root'" \
        "set -gx HOME '$home_dir'" \
        "set -gx XDG_CONFIG_HOME '$home_dir/.config'" \
        "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
        "set -gx PATH '$safe_path'" \
        "source install-dev.fish --fish-only >/dev/null 2>&1" \
        "set -gx PATH "(string escape -- $argv[6..-1] | string join ' ') \
        "_remove_conflicting_binaries '$install'" >"$driver"
end

set -l driver "$sandbox/driver.fish"
write_driver $driver $repo_root $home_dir $safe_path $install \
    $early $outside $other $shims $install $late $safe_path

set -l out (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver" 2>&1)
set -l rc $status
if test $rc -ne 0
    __gpy_test_fail "driver exited $rc: $out"
end

# (a) the stale copy of ours, earlier on PATH, is gone
if not test -e "$early/gpy-agent"; and not test -e "$early/gpy"
    __gpy_test_pass "stale gpy and gpy-agent earlier on PATH were removed"
else
    __gpy_test_fail "a stale binary earlier on PATH survived: "(ls $early)
end
if string match -q "*Removed conflicting gpy-agent*$early/gpy-agent*" -- "$out"
    __gpy_test_pass "the removal was reported"
else
    __gpy_test_fail "no removal report for $early/gpy-agent in: $out"
end

# (b) an unrelated tool sharing the name is kept and warned about
if test -e "$other/gpy-agent"
    __gpy_test_pass "an unrelated tool named gpy-agent was kept"
else
    __gpy_test_fail "the unrelated tool at $other/gpy-agent was deleted"
end
if string match -q "*doesn't look like ours*$other/gpy-agent*" -- "$out"
    __gpy_test_pass "the unrelated tool was warned about"
else
    __gpy_test_fail "no warning for the unrelated tool in: $out"
end

# (c) a tool-manager shim is kept
if test -e "$shims/gpy-agent"
    __gpy_test_pass "a */shims/* entry was kept"
else
    __gpy_test_fail "the shim at $shims/gpy-agent was deleted"
end

# (d) the install dir itself and anything after it are untouched
if test -e "$install/gpy-agent"; and test -e "$install/gpy"
    __gpy_test_pass "the install dir's own binaries are untouched"
else
    __gpy_test_fail "the install dir lost a binary: "(ls $install)
end
if test -e "$late/gpy-agent"
    __gpy_test_pass "a copy after the install dir on PATH was kept"
else
    __gpy_test_fail "the copy after the install dir was deleted"
end

# (f) a shadowing copy of ours outside $HOME is kept and named in a warning
if test -e "$outside/gpy-agent"
    __gpy_test_pass "a shadowing copy outside \$HOME was kept"
else
    __gpy_test_fail "the copy outside \$HOME at $outside/gpy-agent was deleted"
end
if string match -q "*outside \$HOME*$outside/gpy-agent*" -- "$out"
    __gpy_test_pass "the copy outside \$HOME was warned about"
else
    __gpy_test_fail "no outside-\$HOME warning for $outside/gpy-agent in: $out"
end

# (e) install dir absent from PATH: nothing shadows it, nothing is deleted
set -l driver_off "$sandbox/driver_off.fish"
write_driver $driver_off $repo_root $home_dir $safe_path $install \
    $off_path_copy $safe_path
set -l out_off (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver_off" 2>&1)
set -l rc_off $status
if test $rc_off -ne 0
    __gpy_test_fail "off-PATH driver exited $rc_off: $out_off"
end
if test -e "$off_path_copy/gpy-agent"
    __gpy_test_pass "with the install dir off PATH, the copy on PATH was kept"
else
    __gpy_test_fail "with the install dir off PATH, $off_path_copy/gpy-agent was deleted"
end
if string match -q "*Removed conflicting*" -- "$out_off"
    __gpy_test_fail "with the install dir off PATH, a removal was reported: $out_off"
else
    __gpy_test_pass "with the install dir off PATH, no removal was reported"
end

rm -rf "$sandbox"
if test $failures -gt 0
    echo "FAILED: $failures assertion(s)"
    exit 1
end
echo "PASS: _remove_conflicting_binaries removes only our stale binaries that shadow the install"
