#!/usr/bin/env fish
# tests/fish/fresh_install.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY in a fresh environment: an empty HOME with no GPY config. An interactive
# Fish that loads the integration (conf.d/gpy_init.fish and
# functions/fish_prompt.fish) with this checkout's `gpy` draws a prompt with
# the default theme in a plain directory and in a new git repository, writes
# no config file, and sets no public GPY_* variable.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g target_dir $__gpy_root/gpy-agent/target
test -n "$CARGO_TARGET_DIR"; and set target_dir $CARGO_TARGET_DIR
if not test -x $target_dir/debug/gpy
    fail "gpy not built (cd gpy-agent && cargo build --bins)"
    exit 1
end

set -g home (mktemp -d)
test_track_dir $home
mkdir -p $home/repo
git -C $home/repo init -q
set -g fish_dir (path dirname (command -s fish))

# The agent stays off: a fresh install would start it, which
# first_prompt_agent_start.test.fish covers without leaving a daemon behind.
function fresh_prompt --argument-names dir
    env -i HOME=$home PATH=$target_dir/debug:/usr/bin:/bin:$fish_dir TERM=xterm-256color \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SOCKET_PATH=$home/agent.sock \
        fish --no-config -i -c '
            source $argv[1]/fish/conf.d/gpy_init.fish
            source $argv[1]/fish/functions/fish_prompt.fish
            cd $argv[2]
            fish_prompt
            echo
            set -l public (set -n | string match -r "^GPY_.*" | string match -vr "^GPY_AGENT_(ENABLED|SOCKET_PATH)\$")
            echo "vars=$public"' -- $__gpy_root $dir </dev/null 2>&1 | string collect
end

for dir in $home $home/repo
    set -l out (fresh_prompt $dir | string collect)
    set -l prompt (string replace -ra '\e\[[0-9;]*m' '' -- $out | string collect)
    set -l name (path basename $dir)
    # HOME itself is shown as ~.
    test $dir = $home; and set name '~'
    check "first prompt in $name ends with the prompt symbol" 1 (string match -qr '❯ \n' -- "$prompt"; and echo 1)
    check "first prompt in $name carries the directory" 1 (string match -q "*$name*" -- "$prompt"; and echo 1)
    check "first prompt in $name has no JSON or error" 1 (string match -qr '\{"|rror' -- "$prompt"; or echo 1)
    check "first prompt in $name uses colour" 1 (string match -q '*'\e'[*' -- "$out"; and echo 1)
    check "GPY sets no public GPY_* variable in $name" vars= (string match -r '(?m)^vars=.*$' -- "$out")
end

set -l created (find $home -path '*/gpy/config.toml' 2>/dev/null)
check "a fresh prompt writes no config.toml" "" "$created"

test_finish "fresh install"
