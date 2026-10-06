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

# #814: the confirmation listing must show the conf.d path, not an empty item.
set -l fish_line (string match -r -- '.*- Fish: .*' $run_out)
if string match -q -- "*$sandbox_home/.config/fish/conf.d/gpy_init.fish*" $fish_line; and not string match -q -- '*, ,*' $fish_line
    __gpy_test_pass "#814: listing shows the conf.d/gpy_init.fish path"
else
    __gpy_test_fail "#814: Fish listing line lacks conf.d path or has an empty item: $fish_line"
end

rm -rf "$isolated_dir" "$sandbox_home"

# --- Regression suite for #667 (prompt destination preservation) and #670 (backup timestamp selection) ---

set -l isolated_bin (mktemp -d)
cp scripts/uninstall.fish "$isolated_bin/uninstall.fish"

function __gpy_test_uninstall --argument-names isolated_script root
    set -l s_home "$root/home"
    set -l s_config "$root/config"
    set -l s_cache "$root/cache"
    set -l s_runtime "$root/runtime"
    mkdir -p "$s_home" "$s_config" "$s_cache" "$s_runtime"
    printf '\n' | env -u GPY_BUNDLED_PLUGIN_DIR \
        HOME="$s_home" \
        XDG_CONFIG_HOME="$s_config" \
        XDG_CACHE_HOME="$s_cache" \
        XDG_RUNTIME_DIR="$s_runtime" \
        GPY_CONFIG_PATH="$s_config/gpy/config.toml" \
        GPY_AGENT_SOCKET_PATH="$s_runtime/gpy.sock" \
        fish "$isolated_script" >/dev/null 2>&1
end

# 1. Current unrelated regular prompt + valid backup: bytes unchanged, backup unchanged (#667)
set -l t1 (mktemp -d)
set -l t1_fn "$t1/config/fish/functions"
mkdir -p "$t1_fn"
set -l t1_prompt "$t1_fn/fish_prompt.fish"
set -l t1_backup "$t1_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo unrelated_current; end" >"$t1_prompt"
printf '%s\n' "function fish_prompt; echo obsolete_backup; end" >"$t1_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t1"
if test (cat "$t1_prompt") = "function fish_prompt; echo unrelated_current; end"; and test -f "$t1_backup"
    __gpy_test_pass "#667 matrix 1: unrelated regular prompt and backup preserved byte-for-byte"
else
    __gpy_test_fail "#667 matrix 1: unrelated prompt or backup was modified"
end
rm -rf "$t1"

# 2. Unrelated valid symlink + backup: target string and target bytes unchanged, backup unchanged (#667)
set -l t2 (mktemp -d)
set -l t2_fn "$t2/config/fish/functions"
set -l t2_custom "$t2/custom_theme"
mkdir -p "$t2_fn" "$t2_custom"
set -l t2_target "$t2_custom/my_prompt.fish"
printf '%s\n' "function fish_prompt; echo symlink_target; end" >"$t2_target"
set -l t2_prompt "$t2_fn/fish_prompt.fish"
ln -s "$t2_target" "$t2_prompt"
set -l t2_backup "$t2_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo obsolete_backup; end" >"$t2_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t2"
if test -L "$t2_prompt"; and test (readlink "$t2_prompt") = "$t2_target"; and test (cat "$t2_prompt") = "function fish_prompt; echo symlink_target; end"; and test -f "$t2_backup"
    __gpy_test_pass "#667 matrix 2: unrelated valid symlink and backup preserved"
else
    __gpy_test_fail "#667 matrix 2: unrelated valid symlink or backup was modified"
end
rm -rf "$t2"

# 3. Unrelated dangling symlink + backup: remains a symlink with the same target, backup unchanged (#667)
set -l t3 (mktemp -d)
set -l t3_fn "$t3/config/fish/functions"
mkdir -p "$t3_fn"
set -l t3_prompt "$t3_fn/fish_prompt.fish"
ln -s "/nonexistent/dangling/prompt.fish" "$t3_prompt"
set -l t3_backup "$t3_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo obsolete_backup; end" >"$t3_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t3"
if test -L "$t3_prompt"; and test (readlink "$t3_prompt") = "/nonexistent/dangling/prompt.fish"; and test -f "$t3_backup"
    __gpy_test_pass "#667 matrix 3: unrelated dangling symlink and backup preserved"
else
    __gpy_test_fail "#667 matrix 3: dangling symlink was not preserved"
end
rm -rf "$t3"

# 4. Actual GPY prompt symlink + backup: old prompt restored; GPY link removed (#667)
set -l t4 (mktemp -d)
set -l t4_fn "$t4/config/fish/functions"
set -l t4_gpy "$t4/config/fish/gpy/functions"
mkdir -p "$t4_fn" "$t4_gpy"
set -l t4_gpy_prompt "$t4_gpy/fish_prompt.fish"
printf '%s\n' "function fish_prompt; echo gpy; end" >"$t4_gpy_prompt"
set -l t4_prompt "$t4_fn/fish_prompt.fish"
ln -s "$t4_gpy_prompt" "$t4_prompt"
set -l t4_backup "$t4_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo restored_old; end" >"$t4_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t4"
if test -f "$t4_prompt"; and not test -L "$t4_prompt"; and test (cat "$t4_prompt") = "function fish_prompt; echo restored_old; end"; and not test -e "$t4_backup"
    __gpy_test_pass "#667 matrix 4: GPY prompt symlink removed and backup restored"
else
    __gpy_test_fail "#667 matrix 4: GPY prompt symlink restoration failed"
end
rm -rf "$t4"

# 5. GPY-owned regular prompt + backup: old prompt restored (#667)
set -l t5 (mktemp -d)
set -l t5_fn "$t5/config/fish/functions"
mkdir -p "$t5_fn"
set -l t5_prompt "$t5_fn/fish_prompt.fish"
printf '%s\n' "function fish_prompt; # gpy prompt implementation; end" >"$t5_prompt"
set -l t5_backup "$t5_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo restored_old; end" >"$t5_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t5"
if test -f "$t5_prompt"; and not test -L "$t5_prompt"; and test (cat "$t5_prompt") = "function fish_prompt; echo restored_old; end"; and not test -e "$t5_backup"
    __gpy_test_pass "#667 matrix 5: GPY-owned regular prompt removed and backup restored"
else
    __gpy_test_fail "#667 matrix 5: GPY regular prompt restoration failed"
end
rm -rf "$t5"

# 6. Absent destination with/without backup: restore only in former case (#667)
set -l t6a (mktemp -d)
set -l t6a_fn "$t6a/config/fish/functions"
mkdir -p "$t6a_fn"
set -l t6a_prompt "$t6a_fn/fish_prompt.fish"
set -l t6a_backup "$t6a_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo restored_from_absent; end" >"$t6a_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t6a"
if test -f "$t6a_prompt"; and test (cat "$t6a_prompt") = "function fish_prompt; echo restored_from_absent; end"; and not test -e "$t6a_backup"
    __gpy_test_pass "#667 matrix 6a: absent destination with backup restores backup"
else
    __gpy_test_fail "#667 matrix 6a: failed to restore backup into absent destination"
end
rm -rf "$t6a"

set -l t6b (mktemp -d)
set -l t6b_fn "$t6b/config/fish/functions"
mkdir -p "$t6b_fn"
set -l t6b_prompt "$t6b_fn/fish_prompt.fish"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t6b"
if not test -e "$t6b_prompt"; and not test -L "$t6b_prompt"
    __gpy_test_pass "#667 matrix 6b: absent destination without backup remains absent"
else
    __gpy_test_fail "#667 matrix 6b: empty prompt created when no backup was present"
end
rm -rf "$t6b"

# 7. Directory at destination + backup: neither replaced or consumed (#667)
set -l t7 (mktemp -d)
set -l t7_fn "$t7/config/fish/functions"
mkdir -p "$t7_fn"
set -l t7_prompt "$t7_fn/fish_prompt.fish"
mkdir -p "$t7_prompt"
set -l t7_backup "$t7_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo backup_content; end" >"$t7_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t7"
if test -d "$t7_prompt"; and test -f "$t7_backup"
    __gpy_test_pass "#667 matrix 7: directory at prompt destination preserved, backup unconsumed"
else
    __gpy_test_fail "#667 matrix 7: directory destination was touched"
end
rm -rf "$t7"

# 8. Repeat uninstall after successful restoration (#667)
set -l t8 (mktemp -d)
set -l t8_fn "$t8/config/fish/functions"
mkdir -p "$t8_fn"
set -l t8_prompt "$t8_fn/fish_prompt.fish"
set -l t8_backup1 "$t8_fn/fish_prompt.fish.backup.20260201_000000"
set -l t8_backup2 "$t8_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo custom_prompt_newest; end" >"$t8_backup1"
printf '%s\n' "function fish_prompt; echo custom_prompt_older; end" >"$t8_backup2"
# First run restores newest backup into absent destination
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t8"
if test -f "$t8_prompt"; and test (cat "$t8_prompt") = "function fish_prompt; echo custom_prompt_newest; end"; and test -f "$t8_backup2"
    # Second run should preserve the restored prompt and leave the older backup untouched
    __gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t8"
    if test -f "$t8_prompt"; and test (cat "$t8_prompt") = "function fish_prompt; echo custom_prompt_newest; end"; and test -f "$t8_backup2"
        __gpy_test_pass "#667 matrix 8: repeat uninstall preserves restored prompt and unconsumed backup"
    else
        __gpy_test_fail "#667 matrix 8: repeat uninstall consumed extra backup or mutated prompt"
    end
else
    __gpy_test_fail "#667 matrix 8: initial restoration failed"
end
rm -rf "$t8"

# 9. Older .gpy-backup. plus newer .backup.: restore newer .backup. (#670)
set -l t9 (mktemp -d)
set -l t9_fn "$t9/config/fish/functions"
mkdir -p "$t9_fn"
set -l t9_prompt "$t9_fn/fish_prompt.fish"
set -l t9_older "$t9_fn/fish_prompt.fish.gpy-backup.20260101_000000"
set -l t9_newer "$t9_fn/fish_prompt.fish.backup.20260201_000000"
printf '%s\n' "function fish_prompt; echo older_gpy; end" >"$t9_older"
printf '%s\n' "function fish_prompt; echo newer_backup; end" >"$t9_newer"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t9"
if test -f "$t9_prompt"; and test (cat "$t9_prompt") = "function fish_prompt; echo newer_backup; end"; and test -f "$t9_older"; and not test -e "$t9_newer"
    __gpy_test_pass "#670 matrix 1: older .gpy-backup. + newer .backup. restores newer .backup."
else
    __gpy_test_fail "#670 matrix 1: wrong backup restored"
end
rm -rf "$t9"

# 10. Newer .gpy-backup. plus older .backup.: restore newer .gpy-backup. (#670)
set -l t10 (mktemp -d)
set -l t10_fn "$t10/config/fish/functions"
mkdir -p "$t10_fn"
set -l t10_prompt "$t10_fn/fish_prompt.fish"
set -l t10_older "$t10_fn/fish_prompt.fish.backup.20260101_000000"
set -l t10_newer "$t10_fn/fish_prompt.fish.gpy-backup.20260201_000000"
printf '%s\n' "function fish_prompt; echo older_backup; end" >"$t10_older"
printf '%s\n' "function fish_prompt; echo newer_gpy; end" >"$t10_newer"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t10"
if test -f "$t10_prompt"; and test (cat "$t10_prompt") = "function fish_prompt; echo newer_gpy; end"; and test -f "$t10_older"; and not test -e "$t10_newer"
    __gpy_test_pass "#670 matrix 2: newer .gpy-backup. + older .backup. restores newer .gpy-backup."
else
    __gpy_test_fail "#670 matrix 2: wrong backup restored"
end
rm -rf "$t10"

# 11. Multiple backups of either convention alone: greatest suffix wins (#670)
set -l t11 (mktemp -d)
set -l t11_fn "$t11/config/fish/functions"
mkdir -p "$t11_fn"
set -l t11_prompt "$t11_fn/fish_prompt.fish"
set -l t11_b1 "$t11_fn/fish_prompt.fish.backup.20260101_100000"
set -l t11_b2 "$t11_fn/fish_prompt.fish.backup.20260102_100000"
set -l t11_b3 "$t11_fn/fish_prompt.fish.backup.20260103_100000"
printf '%s\n' "function fish_prompt; echo b1; end" >"$t11_b1"
printf '%s\n' "function fish_prompt; echo b2; end" >"$t11_b2"
printf '%s\n' "function fish_prompt; echo b3_greatest; end" >"$t11_b3"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t11"
if test -f "$t11_prompt"; and test (cat "$t11_prompt") = "function fish_prompt; echo b3_greatest; end"; and test -f "$t11_b1"; and test -f "$t11_b2"; and not test -e "$t11_b3"
    __gpy_test_pass "#670 matrix 3: greatest timestamp suffix wins among single convention"
else
    __gpy_test_fail "#670 matrix 3: greatest suffix selection failed"
end
rm -rf "$t11"

# 12. Same suffix tie policy: .gpy-backup. wins tie (#670)
set -l t12 (mktemp -d)
set -l t12_fn "$t12/config/fish/functions"
mkdir -p "$t12_fn"
set -l t12_prompt "$t12_fn/fish_prompt.fish"
set -l t12_plain "$t12_fn/fish_prompt.fish.backup.20260101_000000"
set -l t12_gpy "$t12_fn/fish_prompt.fish.gpy-backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo plain_tie; end" >"$t12_plain"
printf '%s\n' "function fish_prompt; echo gpy_tie_winner; end" >"$t12_gpy"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t12"
if test -f "$t12_prompt"; and test (cat "$t12_prompt") = "function fish_prompt; echo gpy_tie_winner; end"; and test -f "$t12_plain"; and not test -e "$t12_gpy"
    __gpy_test_pass "#670 matrix 4: same suffix tie gives victory to .gpy-backup."
else
    __gpy_test_fail "#670 matrix 4: same suffix tie policy failed"
end
rm -rf "$t12"

# 13. Malformed suffix and matching directory candidates ignored (#670)
set -l t13 (mktemp -d)
set -l t13_fn "$t13/config/fish/functions"
mkdir -p "$t13_fn"
set -l t13_prompt "$t13_fn/fish_prompt.fish"
set -l t13_bad_suffix "$t13_fn/fish_prompt.fish.backup.invalid_stamp"
printf '%s\n' "function fish_prompt; echo bad_suffix; end" >"$t13_bad_suffix"
set -l t13_dir_candidate "$t13_fn/fish_prompt.fish.backup.20260101_000000"
mkdir -p "$t13_dir_candidate"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t13"
if not test -e "$t13_prompt"; and not test -L "$t13_prompt"; and test -f "$t13_bad_suffix"; and test -d "$t13_dir_candidate"
    __gpy_test_pass "#670 matrix 5: malformed suffix and directory candidates untouched"
else
    __gpy_test_fail "#670 matrix 5: invalid candidate was improperly selected or modified"
end
rm -rf "$t13"

# 13b. A symlink backup of a user's own prompt is restored as the symlink
# (#744). Installers now move a symlinked (dotfiles) prompt aside as-is,
# which #670 predates: it assumed every backup was a regular file.
set -l t13b (mktemp -d)
set -l t13b_fn "$t13b/config/fish/functions"
mkdir -p "$t13b_fn" "$t13b/dotfiles"
set -l t13b_prompt "$t13b_fn/fish_prompt.fish"
set -l t13b_target "$t13b/dotfiles/fish_prompt.fish"
printf '%s\n' "function fish_prompt; echo dotfiles; end" >"$t13b_target"
ln -s "$t13b_target" "$t13b_fn/fish_prompt.fish.gpy-backup.20260101_000000"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t13b"
if test -L "$t13b_prompt"; and test (readlink "$t13b_prompt") = "$t13b_target"; and test -f "$t13b_target"
    __gpy_test_pass "#744: a symlink backup is restored as the user's symlink"
else
    __gpy_test_fail "#744: symlink backup was not restored as a symlink"
end
rm -rf "$t13b"

# 14. Fixture root includes spaces: correct restoration, no split-path (#670)
set -l t14 (mktemp -d "/tmp/gpy test spaces.XXXXXX")
set -l t14_fn "$t14/config/fish/functions"
mkdir -p "$t14_fn"
set -l t14_prompt "$t14_fn/fish_prompt.fish"
set -l t14_backup "$t14_fn/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' "function fish_prompt; echo space_path_restored; end" >"$t14_backup"
__gpy_test_uninstall "$isolated_bin/uninstall.fish" "$t14"
if test -f "$t14_prompt"; and test (cat "$t14_prompt") = "function fish_prompt; echo space_path_restored; end"; and not test -e "$t14_backup"
    __gpy_test_pass "#670 matrix 6: path with spaces restores correctly"
else
    __gpy_test_fail "#670 matrix 6: space path restoration failed"
end
rm -rf "$t14"

rm -rf "$isolated_bin"

# ===========================================================================

if test $failed -eq 1
    exit 1
end

echo "✅ scripts/uninstall.fish runs standalone with no repo checkout and no read -p bug"
