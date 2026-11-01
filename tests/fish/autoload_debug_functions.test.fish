#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# gpy-prompt-debug and gpy-prompt-reload are autoloaded from fish/functions,
# not parsed at startup; the deprecated prompt-debug and prompt-reload wrappers
# print one notice per shell; init.fish adds the functions directory to
# fish_function_path once, even when an absolute entry for it already exists
# through a symlinked spelling and init.fish is sourced by a relative path.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g t_sandbox (mktemp -d)
test_track_dir $t_sandbox
mkdir -p $t_sandbox/home $t_sandbox/cache $t_sandbox/config $t_sandbox/run
# The checkout, reached through a symlink: `path resolve` would follow it.
ln -s $__gpy_root $t_sandbox/checkout

# Run $argv[1] in a fresh Fish started in the symlinked checkout, with no agent
# on PATH. stdout and stderr are the caller's.
function run_child --argument-names code
    command env -i HOME=$t_sandbox/home XDG_CONFIG_HOME=$t_sandbox/config XDG_CACHE_HOME=$t_sandbox/cache \
        XDG_RUNTIME_DIR=$t_sandbox/run TMPDIR=$t_sandbox PATH=/usr/bin:/bin TERM=dumb \
        GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=$t_sandbox/run/gpy.sock \
        T_CHECKOUT=$t_sandbox/checkout (command -s fish) --no-config -c "cd \$T_CHECKOUT
$code" 2>&1
end

echo "--- autoloaded from fish/functions ---"
# A function init.fish defined itself would report init.fish (or another core
# file) as its source.
set -l out (run_child '
    source fish/core/init.fish
    for name in gpy-prompt-debug gpy-prompt-reload prompt-debug prompt-reload __gpy_debug_paths
        functions -q $name; and echo "$name=autoloadable"
        echo "$name="(functions --details -- $name | string replace -r "^.*/" "")
    end
')
for name in gpy-prompt-debug gpy-prompt-reload prompt-debug prompt-reload __gpy_debug_paths
    check "$name is available" "$name=autoloadable" (string match "$name=autoloadable" -- $out)
    check "$name comes from its own file" "$name=$name.fish" (string match "$name=*.fish" -- $out)
end

echo "--- deprecated wrappers print once per shell ---"
set out (run_child '
    source fish/core/init.fish
    prompt-reload
    prompt-reload
    prompt-debug
    prompt-debug
')
check "prompt-reload prints its notice once" 1 (count (string match "gpy: prompt-reload is deprecated; use gpy-prompt-reload instead" -- $out))
check "prompt-debug prints its notice once" 1 (count (string match "gpy: prompt-debug is deprecated; use gpy-prompt-debug instead" -- $out))
check "prompt-debug still works through the wrapper" 2 (count (string match "Usage: gpy-prompt-debug*" -- $out))

echo "--- fish_function_path ---"
set out (run_child '
    source fish/core/init.fish
    printf "after_one=%s\n" (count (string match "*/checkout/fish/functions" -- $fish_function_path))
    source fish/core/init.fish
    printf "after_two=%s\n" (count (string match "*/checkout/fish/functions" -- $fish_function_path))
')
check "init adds the functions directory once" after_one=1 "$out[1]"
check "sourcing init again adds nothing" after_two=1 "$out[2]"

set out (run_child '
    set -a fish_function_path $T_CHECKOUT/fish/functions
    source fish/core/init.fish
    printf "preset=%s\n" (count (string match "*/checkout/fish/functions" -- $fish_function_path))
')
check "an absolute entry through a symlinked path is not duplicated" preset=1 "$out[1]"

test_finish "autoload debug functions"
