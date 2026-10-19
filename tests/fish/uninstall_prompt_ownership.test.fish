#!/usr/bin/env fish
# tests/fish/uninstall_prompt_ownership.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# scripts/uninstall.sh removes a regular-file fish_prompt.fish only when it is
# a copy of GPY's own prompt. A
# user's prompt that merely mentions gpy (comment, string, function name)
# survives byte-for-byte, as do its backups; GPY's prompt symlink and legacy
# regular-file copies are still removed and the newest backup restored into
# the then-absent destination. Every run gets its own HOME and XDG roots.

set -g repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -g failed 0

function fail --argument-names msg
    echo "❌ $msg"
    set -g failed 1
end

function pass --argument-names msg
    echo "✅ $msg"
end

set -g scratch (mktemp -d)
test_track_dir $scratch

# The first GPY prompt that installers copied in as a regular file
# (2025-08); the current one is read from the checkout.
set -g legacy_gpy_prompt $scratch/legacy_gpy_prompt.fish
printf '%s\n' \
    '# MAIN PROMPT FUNCTION' \
    'function fish_prompt' \
    '    echo' \
    '    for segment in $__enabled_segments' \
    '        if functions -q segment_{$segment}_detect; and segment_{$segment}_detect' \
    '            if functions -q segment_{$segment}_render' \
    '                segment_{$segment}_render' \
    '            end' \
    '        end' \
    '    end' \
    '    set_color normal' \
    '    printf "\n%s " "$__icon_prompt"' \
    end >$legacy_gpy_prompt

# A user's own prompt that names gpy in comments, strings and code; no
# trailing newline so a rewrite would show.
set -g user_prompt $scratch/user_prompt.fish
printf '%s' \
    '# Custom prompt; evaluated gpy but chose this instead' \n \
    '# for segment in $__enabled_segments  (gpy did it this way)' \n \
    'function fish_prompt' \n \
    '    set -l gpy_like "not gpy"' \n \
    '    printf "mine> "' \n \
    end >$user_prompt

set -g backup_bytes $scratch/backup.fish
printf '%s\n' 'function fish_prompt; echo restored_old; end' >$backup_bytes

set -g case_index 0

# Prepare a sandbox root and print it; the prompt lives in
# $root/config/fish/functions.
function new_root
    set -g case_index (math $case_index + 1)
    set -l root $scratch/case$case_index
    mkdir -p $root/home $root/config/fish/functions $root/cache $root/runtime
    echo $root
end

function run_uninstaller --argument-names kind root
    set -l script $repo_root/scripts/uninstall.$kind
    set -l runner sh
    printf '\n' | env -u ZDOTDIR -u GPY_BUNDLED_PLUGIN_DIR \
        HOME=$root/home \
        XDG_CONFIG_HOME=$root/config \
        XDG_CACHE_HOME=$root/cache \
        XDG_RUNTIME_DIR=$root/runtime \
        GPY_CONFIG_PATH=$root/config/gpy/config.toml \
        GPY_AGENT_SOCKET_PATH=$root/runtime/gpy.sock \
        $runner $script >$root/uninstall.log 2>&1
    or fail "uninstall.$kind exited $status: "(cat $root/uninstall.log)
end

for kind in sh
    # Unrelated regular prompt mentioning gpy, plus a backup: both untouched.
    set -l root (new_root)
    set -l fn $root/config/fish/functions
    cp $user_prompt $fn/fish_prompt.fish
    cp $backup_bytes $fn/fish_prompt.fish.backup.20260101_000000
    run_uninstaller $kind $root
    if test -f $fn/fish_prompt.fish; and not test -L $fn/fish_prompt.fish
        and cmp -s $user_prompt $fn/fish_prompt.fish
        and cmp -s $backup_bytes $fn/fish_prompt.fish.backup.20260101_000000
        pass "uninstall.$kind: a prompt that mentions gpy and its backup survive byte-for-byte"
    else
        fail "uninstall.$kind: the user's prompt or its backup changed: "(string join ', ' (ls -A $fn))
    end

    # Unrelated symlink plus a backup: link target and backup untouched.
    set root (new_root)
    set fn $root/config/fish/functions
    cp $user_prompt $root/dotfiles_prompt.fish
    ln -s $root/dotfiles_prompt.fish $fn/fish_prompt.fish
    cp $backup_bytes $fn/fish_prompt.fish.backup.20260101_000000
    run_uninstaller $kind $root
    if test -L $fn/fish_prompt.fish; and test (readlink $fn/fish_prompt.fish) = $root/dotfiles_prompt.fish
        and cmp -s $user_prompt $root/dotfiles_prompt.fish
        and cmp -s $backup_bytes $fn/fish_prompt.fish.backup.20260101_000000
        pass "uninstall.$kind: an unrelated prompt symlink and its backup survive"
    else
        fail "uninstall.$kind: the unrelated symlink or its backup changed: "(string join ', ' (ls -A $fn))
    end

    # GPY's prompt symlink: removed, backup restored into the absent destination.
    set root (new_root)
    set fn $root/config/fish/functions
    mkdir -p $root/config/fish/gpy/functions
    cp $repo_root/fish/functions/fish_prompt.fish $root/config/fish/gpy/functions/fish_prompt.fish
    ln -s $root/config/fish/gpy/functions/fish_prompt.fish $fn/fish_prompt.fish
    cp $backup_bytes $fn/fish_prompt.fish.backup.20260101_000000
    run_uninstaller $kind $root
    if test -f $fn/fish_prompt.fish; and not test -L $fn/fish_prompt.fish
        and cmp -s $backup_bytes $fn/fish_prompt.fish
        and not test -e $fn/fish_prompt.fish.backup.20260101_000000
        pass "uninstall.$kind: GPY's prompt symlink removed and the backup restored"
    else
        fail "uninstall.$kind: GPY symlink removal or restoration failed: "(string join ', ' (ls -A $fn))
    end

    # Regular-file copies of GPY's prompt (legacy installs): removed, and the
    # backup restored when there is one.
    for gpy_copy in $repo_root/fish/functions/fish_prompt.fish $legacy_gpy_prompt
        set root (new_root)
        set fn $root/config/fish/functions
        cp $gpy_copy $fn/fish_prompt.fish
        cp $backup_bytes $fn/fish_prompt.fish.gpy-backup.20260101_000000
        run_uninstaller $kind $root
        if test -f $fn/fish_prompt.fish; and not test -L $fn/fish_prompt.fish
            and cmp -s $backup_bytes $fn/fish_prompt.fish
            and not test -e $fn/fish_prompt.fish.gpy-backup.20260101_000000
            pass "uninstall.$kind: a regular-file GPY prompt ("(path basename $gpy_copy)") removed and the backup restored"
        else
            fail "uninstall.$kind: regular-file GPY prompt ("(path basename $gpy_copy)") not replaced by its backup: "(string join ', ' (ls -A $fn))
        end

        set root (new_root)
        set fn $root/config/fish/functions
        cp $gpy_copy $fn/fish_prompt.fish
        run_uninstaller $kind $root
        if not test -e $fn/fish_prompt.fish; and not test -L $fn/fish_prompt.fish
            pass "uninstall.$kind: a regular-file GPY prompt ("(path basename $gpy_copy)") with no backup is removed"
        else
            fail "uninstall.$kind: regular-file GPY prompt ("(path basename $gpy_copy)") left in place"
        end
    end
end

if test $failed -ne 0
    exit 1
end
echo "✅ uninstall.sh removes only GPY's own fish_prompt.fish"
