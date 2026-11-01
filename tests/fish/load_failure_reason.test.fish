#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# A Fish load failure shows its cause:
#   - a syntax error in a core file prints Fish's own error and the fallback
#     notice on interactive start;
#   - Fish older than 3.6 prints the version line without GPY_DEBUG;
#   - a normal start writes nothing to stderr.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g t_sandbox (mktemp -d)
test_track_dir $t_sandbox
mkdir -p $t_sandbox/home $t_sandbox/cache $t_sandbox/run $t_sandbox/xdg/fish $t_sandbox/data/gpy/bash/core
cp -R $__gpy_root/fish $t_sandbox/data/gpy/fish
cp $__gpy_root/bash/core/g_runner.bash $t_sandbox/data/gpy/bash/core/g_runner.bash
set -g t_err $t_sandbox/stderr

# Run $argv[1] in an interactive Fish with no agent on PATH; stderr goes to
# $t_err and stdout is the caller's.
function run_interactive --argument-names code
    command env -i HOME=$t_sandbox/home XDG_CONFIG_HOME=$t_sandbox/xdg XDG_DATA_HOME=$t_sandbox/data XDG_CACHE_HOME=$t_sandbox/cache \
        XDG_RUNTIME_DIR=$t_sandbox/run TMPDIR=$t_sandbox PATH=/usr/bin:/bin TERM=dumb \
        GPY_AGENT_SUPERVISOR_ENABLED=0 T_INSTALL=$t_sandbox/data/gpy/fish \
        (command -s fish) --no-config -i -c "$code" </dev/null 2>$t_err
end

echo "--- a normal start ---"
run_interactive 'source $T_INSTALL/conf.d/gpy_init.fish' >/dev/null
check "a normal interactive start writes nothing to stderr" "" (string collect <$t_err)

echo "--- Fish older than 3.6 ---"
run_interactive 'set -g __gpy_test_fish_version 3.5.0
source $T_INSTALL/core/init.fish' >/dev/null
check "the version line shows without GPY_DEBUG" \
    "gpy: GPY needs Fish 3.6 or newer (this is 3.5.0); see https://fishshell.com/" (string collect <$t_err)

echo "--- a broken core file ---"
printf 'function broken\n' >$t_sandbox/data/gpy/fish/core/util.fish
run_interactive 'source $T_INSTALL/conf.d/gpy_init.fish' >/dev/null
set -l err_text (string collect <$t_err)
if string match -q '*core/util.fish*' -- $err_text; and string match -q '*Missing end to balance this function definition*' -- $err_text
    pass "Fish's own error for the broken file is shown"
else
    fail "Fish's own error is missing: $err_text"
end
if string match -q '*gpy[init]: GPY failed to load; using the fallback prompt*' -- $err_text
    pass "the fallback notice is shown"
else
    fail "the fallback notice is missing: $err_text"
end

test_finish "load failure reason"
