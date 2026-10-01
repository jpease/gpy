#!/usr/bin/env fish
# Test for the SIGURG doorbell (#674)
#
# The agent notifies shells with SIGURG only; the meaning travels in flag
# files next to this shell's registry file (<dir>/<pid>.reregister,
# <dir>/<pid>.reload). Every ring repaints via the variable-change pattern
# (__gpy_repaint_trigger -> __gpy_repaint_on_variable -> force-repaint).
# The real signal is delivered to this process with `kill -URG`.
#
# The visual repaint itself needs a terminal; see
# tests/fish/e2e_interactive_session.test.fish (a real pty session, #645).
#
# Run with: fish tests/fish/doorbell_signal.test.fish

# Use the repo copy, not the installed ~/.config/fish/gpy.
set -l repo_root (path resolve (dirname (status -f))/../..)
source $repo_root/fish/core/util.fish
source $repo_root/fish/core/ipc.fish

set -gx XDG_RUNTIME_DIR (mktemp -d)
mkdir -p (__gpy_shell_registry_dir)
set -g registry_file (__gpy_shell_registry_file)

set -g failures 0
function check --argument-names name expected actual
    if test "$expected" = "$actual"
        echo "PASS: $name"
    else
        echo "FAIL: $name (expected '$expected', got '$actual')"
        set -g failures (math $failures + 1)
    end
end

# Record the side-effect functions instead of talking to an agent.
set -g reregister_calls 0
set -g reload_calls 0
function __gpy_refresh_registration_after_restart
    set -g reregister_calls (math $reregister_calls + 1)
end
function __gpy_apply_agent_reload
    set -g reload_calls (math $reload_calls + 1)
end

function ring
    kill -URG $fish_pid
    # Signal handlers run at the next safe point; give it one.
    sleep 0.05
end

set -e __gpy_repaint_trigger

# 1. Bare ring: repaint only.
ring
check "bare ring bumps repaint trigger" 1 "$__gpy_repaint_trigger"
check "bare ring does not re-register" 0 $reregister_calls
check "bare ring does not reload" 0 $reload_calls

# 2. Reload flag: reload once, flag consumed, still repaints.
touch $registry_file.reload
ring
check "reload flag runs reload" 1 $reload_calls
check "reload flag is consumed" 1 (test -e $registry_file.reload; and echo 0; or echo 1)
check "reload ring repaints" 2 "$__gpy_repaint_trigger"

# 3. Reregister flag: re-register once, flag consumed.
touch $registry_file.reregister
ring
check "reregister flag re-registers" 1 $reregister_calls
check "reregister flag is consumed" 1 (test -e $registry_file.reregister; and echo 0; or echo 1)
check "reregister does not reload" 1 $reload_calls

# 4. A later bare ring does not replay consumed flags.
ring
check "consumed flags are not replayed" "1 1 4" "$reregister_calls $reload_calls $__gpy_repaint_trigger"

# 5. Untracking removes the shell file and both flags.
touch $registry_file $registry_file.reload $registry_file.reregister
__gpy_untrack_shell_for_agent_recovery
check "untrack removes registry file and flags" 0 (path filter $registry_file $registry_file.reload $registry_file.reregister | count)

rm -rf $XDG_RUNTIME_DIR

if test $failures -eq 0
    echo "✅ All tests passed"
    exit 0
end
echo "❌ $failures test(s) failed"
exit 1
