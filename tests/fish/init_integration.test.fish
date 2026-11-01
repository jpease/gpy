#!/usr/bin/env fish
# tests/fish/init_integration.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Integration test for core/init.fish: it sources cleanly, the prompt renders,
# every gpy-prompt-* command and its deprecated prompt-* wrapper runs, and
# `gpy-prompt-debug validate` agrees with the installation on disk.

source (dirname (status --filename))/../lib/test_helpers.fish

echo "========================================="
echo "GPY Init Integration Test"
echo "========================================="

# Find repo root (where core/init.fish lives)
set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"
echo "Running from: $repo_root"
echo ""

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache.
set -gx XDG_CACHE_HOME (mktemp -d)

# A developer's own installed GPY (loaded by their fish config) may already
# define prompt-* functions; erase them so Test 3 sees only what this
# checkout's init defines.
for stale in (functions -n | string match -r '^(gpy-)?prompt-.*')
    functions -e $stale
end

# Test 1: Source core/init.fish without errors
echo "Test 1: Source core/init.fish"
if source fish/core/init.fish 2>&1 | string match -q "*error*"
    fail "core/init.fish produced errors"
else
    pass "core/init.fish loaded without errors"
end
# The prompt function is autoloaded from fish/functions/ on an installed
# system; here it has to be sourced, or `fish_prompt` below is fish's own
# default prompt and Test 2 measures the wrong program.
source fish/functions/fish_prompt.fish

# Test 2: fish_prompt renders the prompt character. The character is
# shell-rendered and always present; agent-rendered segments are covered by
# e2e_prompt_content.test.fish.
echo ""
echo "Test 2: fish_prompt renders the prompt character"
set -l prompt_output (fish_prompt 2>/dev/null | string replace -ra '\e\[[0-9;]*m' '' | string collect)
# An empty icon would turn the glob below into `**`, which matches anything.
if test -n "$__icon_prompt"; and string match -q "*$__icon_prompt*" -- "$prompt_output"
    pass "fish_prompt renders the prompt character '$__icon_prompt'"
else
    fail "fish_prompt output lacks the prompt character '$__icon_prompt': $prompt_output"
end

# Test 3: no gpy-prompt-* command or deprecated prompt-* wrapper wraps an
# undefined function, which exits 127. Both are autoloaded from fish/functions.
echo ""
echo "Test 3: gpy-prompt-* and prompt-* commands do not exit 127"
set -l prompt_cmds (functions -n | string match -r '^(gpy-)?prompt-.*')
for want in gpy-prompt-debug gpy-prompt-reload prompt-debug prompt-reload
    if contains -- $want $prompt_cmds
        pass "init makes $want available"
    else
        fail "init does not make $want available"
    end
end
for cmd in $prompt_cmds
    $cmd >/dev/null 2>&1
    set -l rc $status
    if test $rc -ne 127
        pass "$cmd exits $rc (not 127)"
    else
        fail "$cmd exits 127 (wraps an undefined function)"
    end
end

# Test 4: `gpy-prompt-debug validate` exits 0 on a sandboxed install whose
# XDG_CONFIG_HOME differs from $HOME/.config, and exits 1 once a file it checks
# is gone: a core file init.fish sources from its list (not only flags.fish),
# the script segment runner or flags.fish. `vars` shows the active theme and
# `benchmark` prints `name: <N>ms` lines without braces.
echo ""
echo "Test 4: gpy-prompt-debug in an XDG_CONFIG_HOME sandbox"
set -l sandbox (mktemp -d)
mkdir -p $sandbox/home $sandbox/xdg/fish $sandbox/run
cp -R $repo_root/fish $sandbox/xdg/fish/gpy
# `gpy setup` installs the script segment runner beside the core files.
cp $repo_root/bash/core/g_runner.bash $sandbox/xdg/fish/gpy/core/g_runner.bash
set -l debug_out (env -i \
    "PATH=$repo_root/gpy-agent/target/debug:/usr/bin:/bin:"(dirname (command -s fish)) \
    "HOME=$sandbox/home" "XDG_CONFIG_HOME=$sandbox/xdg" \
    "XDG_CACHE_HOME=$sandbox/cache" "XDG_RUNTIME_DIR=$sandbox/run" \
    "TMPDIR=$sandbox" GPY_AGENT_SUPERVISOR_ENABLED=0 \
    fish --no-config -c 'source $XDG_CONFIG_HOME/fish/gpy/core/init.fish
        set -l core $XDG_CONFIG_HOME/fish/gpy/core
        gpy-prompt-debug validate >/dev/null 2>&1
        echo "validate_rc=$status"
        gpy-prompt-debug vars 2>&1
        echo "theme_name=$__gpy_theme_name"
        gpy-prompt-debug benchmark 2>&1
        command mv $core/supervisor.fish $core/supervisor.fish.gone
        gpy-prompt-debug validate 2>&1
        gpy-prompt-debug validate >/dev/null 2>&1
        echo "no_supervisor_rc=$status"
        command mv $core/supervisor.fish.gone $core/supervisor.fish
        command rm $core/g_runner.bash
        gpy-prompt-debug validate >/dev/null 2>&1
        echo "no_runner_rc=$status"
        command rm $core/flags.fish
        gpy-prompt-debug validate >/dev/null 2>&1
        echo "broken_validate_rc=$status"' 2>&1 | string collect)
rm -rf $sandbox
set -l debug_lines (string split \n -- "$debug_out")
expect_eq "gpy-prompt-debug validate returns 0 on an intact install" \
    (string match -rg '^validate_rc=(\d+)$' -- $debug_lines) 0
expect_eq "gpy-prompt-debug validate returns 1 when supervisor.fish is missing" \
    (string match -rg '^no_supervisor_rc=(\d+)$' -- $debug_lines) 1
if string match -q '*Missing file: core/supervisor.fish*' -- "$debug_out"
    pass "gpy-prompt-debug validate names the missing supervisor.fish"
else
    fail "gpy-prompt-debug validate does not name supervisor.fish: $debug_out"
end
expect_eq "gpy-prompt-debug validate returns 1 when the runner is missing" \
    (string match -rg '^no_runner_rc=(\d+)$' -- $debug_lines) 1
expect_eq "gpy-prompt-debug validate returns 1 when flags.fish is missing" \
    (string match -rg '^broken_validate_rc=(\d+)$' -- $debug_lines) 1
set -l theme_name (string match -rg '^theme_name=(\S+)$' -- $debug_lines)
if test -n "$theme_name"; and string match -q "*Theme: $theme_name*" -- "$debug_out"
    pass "gpy-prompt-debug vars prints the active theme '$theme_name'"
else
    fail "gpy-prompt-debug vars does not print the active theme ('$theme_name'): $debug_out"
end
if string match -qr '^  \w+: \d+ms$' -- $debug_lines; and not string match -qr '^  \w+: \{' -- $debug_lines
    pass "gpy-prompt-debug benchmark prints plain durations"
else
    fail "gpy-prompt-debug benchmark lines are not 'name: <N>ms': $debug_out"
end

test_finish "init integration"
