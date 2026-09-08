#!/usr/bin/env fish
# tests/fish/uninstall_no_repo_checkout.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #310: scripts/uninstall.fish sourced
# ../fish/core/util.fish (a repo-relative path) just to call the 5-line
# __gpy_config_root helper, so the uninstaller crashed immediately for
# anyone who installed via install.sh/install-oneline.sh and has no local
# repo checkout. It also used `read -p "literal text"`, which fish
# interprets as a custom prompt *function name* (not literal text) and
# errors with "Unknown command: Press".
#
# This test copies ONLY scripts/uninstall.fish (never fish/core/util.fish,
# never any other repo file) into an isolated directory with no `fish/`
# sibling at all, then runs it there against a sandboxed $HOME/
# $XDG_CONFIG_HOME, proving it neither errors trying to source a
# repo-relative file nor errors on the confirmation prompt.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -g failed 0

function __gpy_test_fail --argument-names msg
    echo "❌ $msg"
    set -g failed 1
end

function __gpy_test_pass --argument-names msg
    echo "✅ $msg"
end

# --- Static check: read -P (literal string) instead of read -p (function name) ---

if grep -q -- '-p "Press' scripts/uninstall.fish
    __gpy_test_fail "scripts/uninstall.fish still uses 'read -p' with a literal string (fish -p takes a function name, not text)"
else
    __gpy_test_pass "scripts/uninstall.fish does not use 'read -p \"literal text\"'"
end

if grep -q -- '-P "Press Enter' scripts/uninstall.fish
    __gpy_test_pass "scripts/uninstall.fish uses 'read -P' (literal string prompt)"
else
    __gpy_test_fail "scripts/uninstall.fish does not use 'read -P' for its confirmation prompt"
end

# --- Static check: no repo-relative sourcing ---------------------------------

if grep -q 'source.*\.\./fish/core' scripts/uninstall.fish
    __gpy_test_fail "scripts/uninstall.fish still sources a repo-relative fish/core file"
else
    __gpy_test_pass "scripts/uninstall.fish does not source any repo-relative file"
end

# --- Runtime check: works with no repo checkout on disk ----------------------

set -l isolated_dir (mktemp -d)
cp scripts/uninstall.fish "$isolated_dir/uninstall.fish"
# Deliberately do NOT copy fish/ alongside it -- prove there is no sibling
# dependency by asserting the isolated dir has nothing else in it.
set -l sibling_count (count (ls "$isolated_dir"))
if test "$sibling_count" -ne 1
    __gpy_test_fail "test setup error: isolated dir should contain only uninstall.fish, found $sibling_count entries"
end

set -l sandbox_home (mktemp -d)
mkdir -p "$sandbox_home/.config/fish"
set -l sandbox_config "$sandbox_home/.config/fish/config.fish"
printf '%s\n' "set -gx EXISTING_VAR before_gpy" >"$sandbox_config"

# No agent binary, no prompt dir, no conf.d file -- everything should be
# reported as "not found" rather than erroring.
set -l run_out (printf '\n' | env HOME=$sandbox_home fish "$isolated_dir/uninstall.fish" 2>&1)
set -l run_status $status

if test $run_status -ne 0
    __gpy_test_fail "scripts/uninstall.fish exited $run_status with no repo checkout present: $run_out"
else
    __gpy_test_pass "scripts/uninstall.fish exits 0 with no repo checkout present"
end

if string match -q '*No such file or directory*' -- $run_out
    __gpy_test_fail "scripts/uninstall.fish printed a file-not-found error: $run_out"
else
    __gpy_test_pass "scripts/uninstall.fish printed no file-not-found errors"
end

if string match -q '*Unknown command*' -- $run_out
    __gpy_test_fail "scripts/uninstall.fish printed an 'Unknown command' error (read -p bug): $run_out"
else
    __gpy_test_pass "scripts/uninstall.fish printed no 'Unknown command' errors"
end

rm -rf "$isolated_dir" "$sandbox_home"

# ===========================================================================

if test $failed -eq 1
    exit 1
end

echo "✅ scripts/uninstall.fish runs standalone with no repo checkout and no read -p bug"
