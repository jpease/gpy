#!/usr/bin/env fish
# tests/fish/install_oneline_main_wrapper.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh's body must be wrapped in main() and invoked only at the
# end: a truncated `curl | sh` (network cut mid-transfer) would otherwise
# execute whatever prefix of a flat script was received -- e.g. install the
# binary but stop mid-way through editing an rc file.
#
# This is a static/text-level check for the structural claims that aren't
# independently network-testable, plus a real (network-free)
# truncated-execution test proving the truncation guarantee concretely.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l installer install-oneline.sh
# --- Static structural checks ---

if grep -q '^main() {' $installer
    pass "$installer: main() {} is defined"
else
    fail "$installer: main() {} is not defined"
end

set -l last_nonblank (grep -v '^[[:space:]]*$' $installer | tail -n 1)
if test "$last_nonblank" = 'main "$@"'
    pass "$installer: main \"\$@\" is the last non-blank line"
else
    fail "$installer: last non-blank line is '$last_nonblank', expected 'main \"\$@\"'"
end

if grep -q 'trap.*EXIT' $installer
    pass "$installer: a trap referencing EXIT is present"
else
    fail "$installer: no trap referencing EXIT found (temp dir won't be cleaned up on a mid-download die)"
end

# --- Truncated-execution check ---
#
# A truncated script must execute nothing of the procedural body -- proven by
# checking that the very first message main()'s body would print
# ("Detecting system configuration...", from the `info` call right after the
# header) never appears. This makes zero network calls: a truncated script
# that never reaches (or never successfully defines/invokes) main() can't get
# far enough to hit `curl`.

set -l total_lines (count (cat $installer))
set -l tmp_dir (mktemp -d)

function __truncate_and_run --argument-names installer tmp_dir label line_count
    set -l truncated "$tmp_dir/truncated_$label.sh"
    head -n $line_count $installer >$truncated
    # Run through `sh`; a truncated script may exit non-zero (e.g. unclosed
    # function body) -- that's fine, we only care what it printed.
    sh $truncated >"$tmp_dir/out_$label.log" 2>&1
    cat "$tmp_dir/out_$label.log"
end

# Truncation point 1: cut off partway through main()'s body (well past the
# `main() {` line, well before the final `main "$@"` call).
set -l mid_body_line (math "$total_lines - 40")
set -l out1 (__truncate_and_run $installer $tmp_dir mid_body $mid_body_line)
if string match -q '*Detecting system configuration*' -- $out1
    fail "Truncation (mid function body, line $mid_body_line/$total_lines): script executed main() body despite truncation"
else
    pass "Truncation (mid function body, line $mid_body_line/$total_lines): main() body did not execute"
end

# Truncation point 2: cut off right before the trailing `main "$@"` line, so
# the whole function is defined but the invocation itself never happens.
set -l before_call_line (math "$total_lines - 1")
set -l out2 (__truncate_and_run $installer $tmp_dir before_call $before_call_line)
if string match -q '*Detecting system configuration*' -- $out2
    fail "Truncation (right before 'main \"\$@\"', line $before_call_line/$total_lines): script ran main() despite the call being cut off"
else
    pass "Truncation (right before 'main \"\$@\"', line $before_call_line/$total_lines): main() was never invoked"
end

rm -rf $tmp_dir

test_finish "install-oneline.sh main() wrapper checks pass"
