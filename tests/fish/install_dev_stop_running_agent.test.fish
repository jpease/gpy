#!/usr/bin/env fish
# tests/fish/install_dev_stop_running_agent.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish's _stop_running_agent must stop only the agent on its own
# socket and never force-kill by process name (`pkill gpy-agent`), which is
# indistinguishable from a contributor's live dogfooding daemon or from
# another test agent bound to its own gpy-test-*.sock (see
# scripts/cleanup-test-agents.sh). `gpy-agent stop` already polls (bounded)
# until the agent stops responding, so no extra fixed wait is needed.
#
# This drives the REAL _stop_running_agent, sourced from the real
# install-dev.fish (not reimplemented here), against two REAL gpy-agent
# processes: an "own" agent bound to the default-resolved socket (what
# _stop_running_agent's bare `gpy-agent status`/`stop` calls target) and a
# "decoy" agent bound to its own distinct socket (standing in for another
# test agent, or a contributor's dogfooding daemon). A stub `pkill` placed
# ahead of the real one on PATH records whether it is ever invoked.
#
# Function definitions are pulled out of install-dev.fish by actually
# sourcing the file (with --fish-only and PATH scrubbed so gpy-agent is
# unreachable, mirroring install_dev_config_theme_preserve.test.fish's
# once against a throwaway $HOME, then PATH is switched back in so the
# defined _stop_running_agent can be called directly against the two
# real agents started below.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

# Resolve a real gpy-agent binary. Prefer the debug build test_fish.sh
# ensures exists (matches how other Fish tests get a known-fresh binary
# rather than trusting whatever is on the developer's PATH); fall back to
# PATH for a direct `fish tests/fish/...` invocation.
set -l agent_path "$repo_root/gpy-agent/target/debug/gpy-agent"
if not test -x "$agent_path"
    set agent_path (command -v gpy-agent)
end
if test -z "$agent_path"
    fail "gpy-agent not found (build it: cd gpy-agent && cargo build)"
    exit 1
end
set -l agent_dir (path dirname "$agent_path")

set -l home_dir (mktemp -d)
mkdir -p "$home_dir/.local/bin"
printf '#!/bin/sh\nexit 0\n' >"$home_dir/.local/bin/gpy"
chmod +x "$home_dir/.local/bin/gpy"
set -l safe_path /usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin
set -l stub_dir (mktemp -d)
set -l pkill_log "$home_dir/pkill.log"
touch "$pkill_log"

# Fake pkill: records every invocation instead of doing anything. Placed
# ahead of the real pkill on PATH so any matching by process name shows up
# here instead of silently killing the decoy.
printf '%s\n' '#!/bin/sh' 'echo "$@" >> "'"$pkill_log"'"' 'exit 0' >"$stub_dir/pkill"
chmod +x "$stub_dir/pkill"

set -l decoy_socket "$home_dir/decoy-gpy-test.sock"

set -l driver "$home_dir/driver.fish"
printf '%s\n' \
    "cd '$repo_root'" \
    "set -gx PATH '$safe_path'" \
    "source install-dev.fish --fish-only --allow-shadow >/dev/null 2>&1" \
    "" \
    "set -gx PATH '$stub_dir' '$agent_dir' '$safe_path'" \
    "set -e XDG_RUNTIME_DIR" \
    "" \
    "gpy-agent start >'$home_dir/own-agent.log' 2>&1" \
    "if test \$status -ne 0" \
    "    echo OWN_START_FAILED" \
    "    exit 1" \
    end \
    "" \
    "env GPY_AGENT_SOCKET_PATH='$decoy_socket' gpy-agent start >'$home_dir/decoy-agent.log' 2>&1" \
    "if test \$status -ne 0" \
    "    echo DECOY_START_FAILED" \
    "    exit 1" \
    end \
    "" \
    "set -l ready 0" \
    "for i in (seq 1 50)" \
    "    if gpy-agent status 2>&1 | grep -q 'Running and Responding'" \
    "        if env GPY_AGENT_SOCKET_PATH='$decoy_socket' gpy-agent status 2>&1 | grep -q 'Running and Responding'" \
    "            set ready 1" \
    "            break" \
    "        end" \
    "    end" \
    "    sleep 0.1" \
    end \
    "if test \$ready -eq 0" \
    "    echo AGENTS_NOT_READY" \
    "    exit 1" \
    end \
    "" \
    _stop_running_agent \
    "" \
    "if gpy-agent status 2>&1 | grep -q 'Running and Responding'" \
    "    echo OWN_STILL_RUNNING" \
    else \
    "    echo OWN_STOPPED" \
    end \
    "" \
    "if env GPY_AGENT_SOCKET_PATH='$decoy_socket' gpy-agent status 2>&1 | grep -q 'Running and Responding'" \
    "    echo DECOY_SURVIVED" \
    else \
    "    echo DECOY_KILLED" \
    end \
    "" \
    "if test -s '$pkill_log'" \
    "    echo PKILL_WAS_CALLED" \
    else \
    "    echo PKILL_NOT_CALLED" \
    end \
    "" \
    "env GPY_AGENT_SOCKET_PATH='$decoy_socket' gpy-agent stop >/dev/null 2>&1" >$driver

set -l out (env HOME=$home_dir XDG_CONFIG_HOME="$home_dir/.config" XDG_CACHE_HOME="$home_dir/.cache" \
    fish --no-config $driver 2>&1)
set -l out_status $status

if test $out_status -ne 0
    fail "_stop_running_agent driver exited $out_status: $out"
else
    if string match -q '*OWN_START_FAILED*' -- $out
        fail "own agent failed to start: $out"
    else if string match -q '*DECOY_START_FAILED*' -- $out
        fail "decoy agent failed to start: $out"
    else if string match -q '*AGENTS_NOT_READY*' -- $out
        fail "own/decoy agents never became ready: $out"
    else
        if string match -q '*OWN_STOPPED*' -- $out
            pass "_stop_running_agent: own agent (default socket) is stopped"
        else
            fail "_stop_running_agent: own agent is still running and responding: $out"
        end

        if string match -q '*DECOY_SURVIVED*' -- $out
            pass "_stop_running_agent: decoy agent (distinct socket) survives untouched"
        else
            fail "_stop_running_agent: decoy agent was killed: $out"
        end

        if string match -q '*PKILL_NOT_CALLED*' -- $out
            pass "_stop_running_agent: pkill is never invoked"
        else
            fail "_stop_running_agent: pkill was invoked (matching by process name): "(cat $pkill_log)
        end
    end
end

# Best-effort cleanup in case an assertion above failed before the driver's
# own decoy stop ran.
env GPY_AGENT_SOCKET_PATH=$decoy_socket "$agent_path" stop >/dev/null 2>&1
rm -rf "$home_dir" "$stub_dir"

test_finish "install-dev.fish's _stop_running_agent stops only its own agent, never by process name"
