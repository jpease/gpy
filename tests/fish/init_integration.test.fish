#!/usr/bin/env fish
# tests/fish/init_integration.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The thin Fish integration (conf.d/gpy_init.fish + functions/fish_prompt.fish)
# in an interactive shell:
#   - with this checkout's `gpy`, fish_prompt prints a prompt ending in the
#     prompt symbol and never the 0x1F separator;
#   - fish_prompt hands `gpy prompt fish` the last status, the PID, the Fish
#     version, the shell's instance token (another in every shell), the
#     command count and, only after a command ran, its duration;
#   - fish_exit runs `gpy prompt fish --exit --pid PID`;
#   - `gpy` is found on PATH, else at ~/.local/bin/gpy; with none, or when it
#     fails or prints no separator, the prompt is the bare `❯ `.
# A stand-in `gpy` logs its arguments and prints PROMPT<US>TAIL.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l target_dir $__gpy_root/gpy-agent/target
test -n "$CARGO_TARGET_DIR"; and set target_dir $CARGO_TARGET_DIR
if not test -x $target_dir/debug/gpy
    fail "gpy not built (cd gpy-agent && cargo build --bins)"
    exit 1
end

set -g tmp (mktemp -d)
test_track_dir $tmp
mkdir -p $tmp/home $tmp/cache $tmp/config $tmp/fake $tmp/failing $tmp/nosep $tmp/home/.local/bin
set -g log $tmp/gpy.log

printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$log'" 'printf "FAKEPROMPT\\037FAKETAIL"' >$tmp/fake/gpy
printf '%s\n' '#!/bin/sh' 'printf "BROKEN\\037"' 'exit 3' >$tmp/failing/gpy
printf '%s\n' '#!/bin/sh' 'printf "NOSEPARATOR"' >$tmp/nosep/gpy
cp $tmp/fake/gpy $tmp/home/.local/bin/gpy
chmod +x $tmp/fake/gpy $tmp/failing/gpy $tmp/nosep/gpy $tmp/home/.local/bin/gpy

# A PATH with fish and the system tools but no gpy (fish's own directory may
# hold an installed gpy, so fish is linked into one of its own).
mkdir -p $tmp/fishbin
ln -s (command -s fish) $tmp/fishbin/fish
set -g base_path /usr/bin /bin $tmp/fishbin
for p in /usr/bin /bin
    test -x $p/gpy; and test_skip "a gpy is installed in $p"
end

# Run $argv[2] in a fresh interactive Fish with the integration loaded and
# PATH $argv[1] (colon-separated) and HOME $argv[3] (default: an empty home).
function in_shell --argument-names shell_path code home
    test -n "$home"; or set home $tmp/nohome
    env -i HOME=$home PATH=$shell_path TERM=dumb XDG_CACHE_HOME=$tmp/cache XDG_CONFIG_HOME=$tmp/config \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SOCKET_PATH=$tmp/none.sock \
        fish --no-config -i -c '
            source $argv[1]/fish/conf.d/gpy_init.fish
            source $argv[1]/fish/functions/fish_prompt.fish
            eval $argv[2]' -- $__gpy_root $code </dev/null 2>&1
end

function plain
    string replace -ra '\e\[[0-9;?]*[A-Za-z]' '' -- $argv | string collect
end

set -l real_path (string join : $target_dir/debug $base_path)
set -l fake_path (string join : $tmp/fake $base_path)
set -l bare_path (string join : $base_path)

# --- the real binary ---
set -l out (plain (in_shell $real_path 'fish_prompt' | string collect))
check "the real gpy draws a prompt ending in the symbol" 1 (string match -qr '❯ $' -- "$out"; and echo 1)
check "the prompt never shows the separator" 1 (string match -q '*'\x1f'*' -- "$out"; or echo 1)
check "the prompt shows no error" 1 (string match -qi '*error*' -- "$out"; or echo 1)

# --- the arguments ---
rm -f $log
set out (in_shell $fake_path 'false; fish_prompt; echo; echo pid=$fish_pid; echo ver=$FISH_VERSION; echo instance=$__gpy_instance' | string collect)
check "fish_prompt prints the binary's prompt part" 1 (string match -q 'FAKEPROMPT*' -- "$out"; and echo 1)
set -l pid (string match -rg 'pid=(\d+)' -- $out)
set -l ver (string match -rg 'ver=(\S+)' -- $out)
set -l instance (string match -rg 'instance=(\S+)' -- $out)
check "first prompt arguments" "prompt fish --status 1 --pid $pid --shell-version $ver --instance $instance --cmd 0" (head -1 $log)
# `exec fish` keeps the PID: only the token tells the new shell apart.
set -l other (in_shell $fake_path 'echo $__gpy_instance' | string collect)
check "another shell draws another instance token" 1 (test -n "$instance" -a "$instance" != "$other"; and echo 1)

rm -f $log
in_shell $fake_path '
    emit fish_preexec x; emit fish_postexec x; set -g CMD_DURATION 1234
    emit fish_prompt; fish_prompt >/dev/null
    emit fish_prompt; fish_prompt >/dev/null' >/dev/null
set -l lines (string match -v -- '*--exit*' <$log)
check "a prompt after a command passes the count and its duration" 1 (string match -qr -- '--cmd 1 --duration-ms 1234$' $lines[1]; and echo 1)
check "a prompt with no command since passes no duration" 1 (string match -qr -- '--cmd 1$' $lines[2]; and echo 1)
check "exit unregisters" 1 (string match -qr -- '^prompt fish --exit --pid \d+$' <$log; and echo 1)

set out (in_shell $fake_path 'fish_prompt --final-rendering' | string collect)
check "--final-rendering prints the tail" FAKETAIL "$out"

# --- finding the binary, and the fallbacks ---
set out (in_shell $bare_path 'fish_prompt' $tmp/home | string collect)
check "~/.local/bin/gpy is used when PATH has none" FAKEPROMPT "$out"
set out (in_shell $bare_path 'fish_prompt' | string collect)
check "no gpy at all: the bare symbol" "❯ " "$out"
set out (in_shell (string join : $tmp/failing $base_path) 'fish_prompt' | string collect)
check "a failing gpy: the bare symbol" "❯ " "$out"
set out (in_shell (string join : $tmp/nosep $base_path) 'fish_prompt' | string collect)
check "a gpy that prints no separator: the bare symbol" "❯ " "$out"

test_finish "init integration"
