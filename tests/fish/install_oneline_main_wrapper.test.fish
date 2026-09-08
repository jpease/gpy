#!/usr/bin/env fish
# tests/fish/install_oneline_main_wrapper.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #309: install-oneline.sh's body was a flat top-to-bottom
# script (not wrapped in main()+invoked at the end), so a truncated
# `curl | sh` (network cut mid-transfer) could execute whatever prefix of the
# script was received -- e.g. install the binary but stop mid-way through
# editing an rc file. Also, individual core-file downloads only `warn`ed on
# failure and fell through to editing rc files unconditionally, so a failed
# core-file download still left the rc file modified.
#
# This is a static/text-level check in the same spirit as
# install_oneline_file_lists.test.fish (#308) for the structural claims that
# aren't independently network-testable, plus a real (network-free)
# truncated-execution test proving criterion 3 concretely.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l installer install-oneline.sh
set -g failed 0

# --- Static structural checks -------------------------------------------------

if grep -q '^main() {' $installer
    echo "✅ $installer: main() {} is defined"
else
    echo "❌ $installer: main() {} is not defined"
    set -g failed 1
end

set -l last_nonblank (grep -v '^[[:space:]]*$' $installer | tail -n 1)
if test "$last_nonblank" = 'main "$@"'
    echo "✅ $installer: main \"\$@\" is the last non-blank line"
else
    echo "❌ $installer: last non-blank line is '$last_nonblank', expected 'main \"\$@\"'"
    set -g failed 1
end

if grep -q 'trap.*EXIT' $installer
    echo "✅ $installer: a trap referencing EXIT is present"
else
    echo "❌ $installer: no trap referencing EXIT found (temp dir won't be cleaned up on a mid-download die)"
    set -g failed 1
end

# The three per-shell core-file download loops must hard-fail (die) rather
# than warn-and-continue, so a failed core-file download can never reach the
# "Configure shell RC file" section below it. Extract each loop's body with
# awk (range match from the `for` line to the following `done`) rather than
# a fish multi-line regex, since fish's `string match -r` matches per-line by
# default.
for shell_upper in FISH ZSH BASH
    set -l block (awk "/for file in \\\$"$shell_upper"_CORE_FILES/,/^        done/" $installer)
    if test -z "$block"
        echo "❌ $installer: could not find the $shell_upper core-file download loop"
        set -g failed 1
        continue
    end

    if string match -q '*|| warn *' -- $block
        echo "❌ $installer: $shell_upper core-file download loop still uses 'warn' on failure (must 'die')"
        set -g failed 1
    else if string match -q '*|| die *' -- $block
        echo "✅ $installer: $shell_upper core-file download loop uses 'die' on failure"
    else
        echo "❌ $installer: $shell_upper core-file download loop has neither 'warn' nor 'die' -- inspect manually"
        set -g failed 1
    end
end

# --- Truncated-execution check (criterion 3) ----------------------------------
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
    echo "❌ Truncation (mid function body, line $mid_body_line/$total_lines): script executed main() body despite truncation"
    set -g failed 1
else
    echo "✅ Truncation (mid function body, line $mid_body_line/$total_lines): main() body did not execute"
end

# Truncation point 2: cut off right before the trailing `main "$@"` line, so
# the whole function is defined but the invocation itself never happens.
set -l before_call_line (math "$total_lines - 1")
set -l out2 (__truncate_and_run $installer $tmp_dir before_call $before_call_line)
if string match -q '*Detecting system configuration*' -- $out2
    echo "❌ Truncation (right before 'main \"\$@\"', line $before_call_line/$total_lines): script ran main() despite the call being cut off"
    set -g failed 1
else
    echo "✅ Truncation (right before 'main \"\$@\"', line $before_call_line/$total_lines): main() was never invoked"
end

rm -rf $tmp_dir

if test $failed -eq 1
    exit 1
end

echo "✅ install-oneline.sh main() wrapper and core-file die-on-failure checks pass"
