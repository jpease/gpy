#!/usr/bin/env fish
# tests/fish/disabled_footprint_prompt.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# When GPY is fully disabled
# (GPY_AGENT_ENABLED=0 and GPY_AGENT_SUPERVISOR_ENABLED=0), the installed,
# autoloaded fish_prompt still runs on every render — Fish's autoloader finds
# it in functions/ regardless of whether GPY's own init sourced anything.
# Both documented entry points into "disabled mode" must leave fish_prompt's
# globals in a safe, defined state so it renders quietly instead of emitting
# "Invalid number" / "Unknown color" diagnostics on every prompt:
#
#   A) fish/conf.d/gpy_init.fish's own disabled early-return (what actually
#      runs at shell startup for an installed user)
#   B) fish/core/init.fish's own disabled early-return (taken when both
#      GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED are 0)
#
# The same holds for the two *unintentional* routes into that state, where
# GPY is enabled but the install is broken. Both reach end-of-init with the
# agent flags forced to 0 and no safe prompt defaults set:
#
#   D) fish/conf.d/gpy_init.fish's install-not-found branch ($gpy_install_dir/
#      core/init.fish missing)
#   E) fish/conf.d/gpy_init.fish's fallback-mode branch (core/init.fish exists
#      but fails to source)
#
# Both keep their existing stderr warning — those are genuine one-shot install
# diagnostics, unlike the per-render prompt noise this file exists to prevent.

# Echo every stderr line that is NOT one of gpy_init.fish's own `gpy[init]:`
# install warnings. Those warnings are expected on the broken-install paths;
# anything else (Fish's "Invalid number" / "Unknown color" diagnostics) is the
# per-render prompt noise under test.
source (dirname (status --filename))/../lib/test_helpers.fish

function stderr_beyond_init_warning --argument-names err_file
    for line in (cat $err_file)
        string match -qr '^gpy\[init\]:' -- $line; and continue
        test -z (string trim -- $line); and continue
        echo $line
    end
end

echo "========================================="
echo "GPY Disabled-Footprint Prompt Safety Test"
echo "========================================="

# Find repo root
set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"
echo "Running from: $repo_root"

# --- Scenario A: fish/conf.d/gpy_init.fish's own disabled early-return ---
echo ""
echo "Scenario A: conf.d/gpy_init.fish disabled early-return"

# Build a throwaway "installed" layout so gpy_init.fish's install-dir
# resolution finds this checkout's fish/ tree, not any real dogfooding
# install that may exist at ~/.config/fish/gpy on the machine running this
# test.
set -l scenario_a_home (mktemp -d)
mkdir -p "$scenario_a_home/.config/fish"
ln -s "$repo_root/fish" "$scenario_a_home/.config/fish/gpy"

set -l scenario_a_err (mktemp)
set -l scenario_a_out (env HOME=$scenario_a_home XDG_CONFIG_HOME=$scenario_a_home/.config \
    GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    fish --no-config -i -c 'source fish/conf.d/gpy_init.fish; source fish/functions/fish_prompt.fish; fish_prompt' </dev/null \
     2>$scenario_a_err | string collect)
set -l scenario_a_status $status

if test -s $scenario_a_err
    fail "conf.d/gpy_init.fish disabled path emitted stderr: "(cat $scenario_a_err | string collect)
else
    pass "conf.d/gpy_init.fish disabled path produced no stderr"
end

if test $scenario_a_status -eq 0
    pass "conf.d/gpy_init.fish disabled path exited 0"
else
    fail "conf.d/gpy_init.fish disabled path exited $scenario_a_status"
end

if test -n "$scenario_a_out"
    pass "conf.d/gpy_init.fish disabled path rendered a non-empty prompt"
else
    fail "conf.d/gpy_init.fish disabled path rendered an empty prompt"
end

# The per-prompt marker reset must not fork `rm` with no operands.
set -l scenario_a_trace (mktemp)
env HOME=$scenario_a_home XDG_CONFIG_HOME=$scenario_a_home/.config \
    GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    fish --no-config -i -c 'source fish/conf.d/gpy_init.fish; source fish/functions/fish_prompt.fish; set fish_trace 1; fish_prompt >/dev/null' </dev/null 2>$scenario_a_trace >/dev/null
if grep -Eq -- '^-+> rm' $scenario_a_trace
    fail "disabled-footprint fish_prompt forked rm: "(grep -E -- '^-+> rm' $scenario_a_trace | string collect)
else
    pass "disabled-footprint fish_prompt forked no rm"
end
rm -f $scenario_a_trace
rm -f $scenario_a_err
rm -rf $scenario_a_home

# --- Scenario B: fish/core/init.fish's own disabled early-return ---
echo ""
echo "Scenario B: core/init.fish disabled-footprint early-return"

set -l scenario_b_err (mktemp)
set -l scenario_b_out (env GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    fish --no-config -c 'source fish/core/init.fish; source fish/functions/fish_prompt.fish; fish_prompt' \
    2>$scenario_b_err | string collect)
set -l scenario_b_status $status

if test -s $scenario_b_err
    fail "core/init.fish disabled-footprint path emitted stderr: "(cat $scenario_b_err | string collect)
else
    pass "core/init.fish disabled-footprint path produced no stderr"
end

if test $scenario_b_status -eq 0
    pass "core/init.fish disabled-footprint path exited 0"
else
    fail "core/init.fish disabled-footprint path exited $scenario_b_status"
end

if test -n "$scenario_b_out"
    pass "core/init.fish disabled-footprint path rendered a non-empty prompt"
else
    fail "core/init.fish disabled-footprint path rendered an empty prompt"
end

rm -f $scenario_b_err

# --- Scenario C: the safe defaults are actually defined, not just silent ---
echo ""
echo "Scenario C: safe defaults are defined in disabled-footprint mode"

set -l scenario_c_vars (env GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    fish --no-config -c '
        source fish/core/init.fish
        for v in __gpy_is_root __prompt_color __root_prompt_color __icon_prompt __icon_status_ok __icon_status_fail
            set -q $v; and echo "$v=defined"; or echo "$v=MISSING"
        end
    ' 2>/dev/null | string collect)

for line in (string split \n -- $scenario_c_vars)
    if string match -q "*MISSING*" -- $line
        fail "expected disabled-footprint default not set: $line"
    else if test -n "$line"
        pass "disabled-footprint default set: $line"
    end
end

# --- Scenario D: conf.d/gpy_init.fish install-not-found branch ---
echo ""
echo "Scenario D: conf.d/gpy_init.fish install-not-found branch"

# A HOME whose gpy/core holds defaults.fish but not init.fish, so
# $gpy_install_dir/core/init.fish resolves to a missing file. GPY itself is
# left ENABLED here — this branch is reached by a broken install, not a user
# choice.
set -l scenario_d_home (mktemp -d)
mkdir -p "$scenario_d_home/.config/fish/gpy/core"
cp "$repo_root/fish/core/defaults.fish" "$scenario_d_home/.config/fish/gpy/core/defaults.fish"
cp "$repo_root/fish/core/flags.fish" "$scenario_d_home/.config/fish/gpy/core/flags.fish"

set -l scenario_d_err (mktemp)
set -l scenario_d_out (env HOME=$scenario_d_home XDG_CONFIG_HOME=$scenario_d_home/.config \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    fish --no-config -i -c 'source fish/conf.d/gpy_init.fish; source fish/functions/fish_prompt.fish; fish_prompt' </dev/null \
     2>$scenario_d_err | string collect)
set -l scenario_d_status $status

if grep -q 'core files not found' $scenario_d_err
    pass "install-not-found path preserved its install warning"
else
    fail "install-not-found path lost its install warning: "(cat $scenario_d_err | string collect)
end

set -l scenario_d_noise (stderr_beyond_init_warning $scenario_d_err | string collect)
if test -z "$scenario_d_noise"
    pass "install-not-found path emitted no prompt-render noise"
else
    fail "install-not-found path emitted prompt-render noise: $scenario_d_noise"
end

if test $scenario_d_status -eq 0
    pass "install-not-found path exited 0"
else
    fail "install-not-found path exited $scenario_d_status"
end

if test -n "$scenario_d_out"
    pass "install-not-found path rendered a non-empty prompt"
else
    fail "install-not-found path rendered an empty prompt"
end

set -l scenario_d_probe (env HOME=$scenario_d_home XDG_CONFIG_HOME=$scenario_d_home/.config \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    fish --no-config -i -c '
        source fish/conf.d/gpy_init.fish
        __gpy_agent_enabled; and echo "enabled=[yes]"; or echo "enabled=[no]"
        fish --no-config -c \'echo "child=[$GPY_AGENT_ENABLED]"\'
    ' </dev/null 2>/dev/null | string collect)

if string match -qr 'enabled=\[no\]' -- $scenario_d_probe
    pass "install-not-found path disables __gpy_agent_enabled"
else
    fail "install-not-found path left __gpy_agent_enabled on: $scenario_d_probe"
end

if string match -qr 'child=\[0\]' -- $scenario_d_probe
    fail "install-not-found path exported GPY_AGENT_ENABLED=0 to children"
else
    pass "install-not-found path did not export GPY_AGENT_ENABLED=0"
end

rm -f $scenario_d_err
rm -rf $scenario_d_home

# --- Scenario E: conf.d/gpy_init.fish fallback-mode branch ---
echo ""
echo "Scenario E: conf.d/gpy_init.fish fallback-mode branch"

# core/init.fish exists but fails to source, so gpy_init.fish takes the
# "GPY failed to load" branch and falls through to end-of-file. This is the
# realistic broken-install case: fish_prompt.fish is definitely installed here
# (only the source failed), so the autoloaded prompt runs uninitialized.
set -l scenario_e_home (mktemp -d)
mkdir -p "$scenario_e_home/.config/fish/gpy/core"
echo 'return 1' >"$scenario_e_home/.config/fish/gpy/core/init.fish"
cp "$repo_root/fish/core/defaults.fish" "$scenario_e_home/.config/fish/gpy/core/defaults.fish"
cp "$repo_root/fish/core/flags.fish" "$scenario_e_home/.config/fish/gpy/core/flags.fish"

set -l scenario_e_err (mktemp)
set -l scenario_e_out (env HOME=$scenario_e_home XDG_CONFIG_HOME=$scenario_e_home/.config \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    fish --no-config -i -c 'source fish/conf.d/gpy_init.fish; source fish/functions/fish_prompt.fish; fish_prompt' </dev/null \
     2>$scenario_e_err | string collect)
set -l scenario_e_status $status

if grep -q 'gpy\[init\]: GPY failed to load; using the fallback prompt' $scenario_e_err
    pass "fallback-mode path preserved its load-failure warning"
else
    fail "fallback-mode path lost its load-failure warning: "(cat $scenario_e_err | string collect)
end

set -l scenario_e_noise (stderr_beyond_init_warning $scenario_e_err | string collect)
if test -z "$scenario_e_noise"
    pass "fallback-mode path emitted no prompt-render noise"
else
    fail "fallback-mode path emitted prompt-render noise: $scenario_e_noise"
end

if test $scenario_e_status -eq 0
    pass "fallback-mode path exited 0"
else
    fail "fallback-mode path exited $scenario_e_status"
end

if test -n "$scenario_e_out"
    pass "fallback-mode path rendered a non-empty prompt"
else
    fail "fallback-mode path rendered an empty prompt"
end

rm -f $scenario_e_err

# A failed load disables this shell only: the failure is recorded in a
# non-exported global, so children see no exported "0" and no leaked marker.
set -l scenario_e_probe (env HOME=$scenario_e_home XDG_CONFIG_HOME=$scenario_e_home/.config \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    fish --no-config -i -c '
        source fish/conf.d/gpy_init.fish
        __gpy_agent_enabled; and echo "enabled=[yes]"; or echo "enabled=[no]"
        __gpy_supervisor_enabled; and echo "supervisor=[yes]"; or echo "supervisor=[no]"
        fish --no-config -c \'echo "child=[$GPY_AGENT_ENABLED]"\'
        env | string match -q "__gpy_load_failed=*"; and echo "leak=[yes]"; or echo "leak=[no]"
    ' </dev/null 2>/dev/null | string collect)

if string match -qr 'enabled=\[no\]' -- $scenario_e_probe
    pass "fallback-mode path disables __gpy_agent_enabled"
else
    fail "fallback-mode path left __gpy_agent_enabled on: $scenario_e_probe"
end

if string match -qr 'supervisor=\[no\]' -- $scenario_e_probe
    pass "fallback-mode path disables __gpy_supervisor_enabled"
else
    fail "fallback-mode path left __gpy_supervisor_enabled on: $scenario_e_probe"
end

if string match -qr 'child=\[0\]' -- $scenario_e_probe
    fail "fallback-mode path exported GPY_AGENT_ENABLED=0 to children"
else
    pass "fallback-mode path did not export GPY_AGENT_ENABLED=0"
end

if string match -qr 'leak=\[no\]' -- $scenario_e_probe
    pass "fallback-mode path did not export __gpy_load_failed"
else
    fail "fallback-mode path exported __gpy_load_failed: $scenario_e_probe"
end

# Re-sourcing after the cause is fixed retries the load.
set -l scenario_e_retry (env HOME=$scenario_e_home XDG_CONFIG_HOME=$scenario_e_home/.config \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    fish --no-config -i -c "
        source fish/conf.d/gpy_init.fish
        echo 'return 0' >$scenario_e_home/.config/fish/gpy/core/init.fish
        source fish/conf.d/gpy_init.fish
        __gpy_agent_enabled; and echo 'retry=[yes]'
    " </dev/null 2>/dev/null | string collect)

if string match -qr 'retry=\[yes\]' -- $scenario_e_retry
    pass "re-sourcing conf.d/gpy_init.fish after the fix re-enables GPY"
else
    fail "re-sourcing conf.d/gpy_init.fish did not retry the load: $scenario_e_retry"
end
rm -rf $scenario_e_home

# Summary
echo ""
echo "========================================="
test_finish "disabled footprint prompt"
