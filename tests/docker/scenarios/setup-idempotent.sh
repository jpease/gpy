#!/usr/bin/env bash
# tests/docker/scenarios/setup-idempotent.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A second `gpy setup` changes nothing: for fish, zsh and bash, the files under
# $HOME (contents, modes, link targets) and the rc file's mtime are the same
# before and after it, and it says GPY is already configured. An rc block
# whose end marker is gone is reported as incomplete, never as configured,
# with exit 5 (SETUP_INCOMPLETE), and the rc file is left alone.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

# `snapshot` -- every file and link under $HOME except caches, logs and the
# runtime directory, with its mode and its content digest or link target.
snapshot() {
    (cd "$HOME" && find . \( -path ./.cache -o -path './.local/state' \) -prune -o \
        \( -type f -o -type l \) -print | LC_ALL=C sort | while IFS= read -r path; do
        if [[ -L "$path" ]]; then
            printf '%s link %s\n' "$path" "$(readlink "$path")"
        else
            printf '%s %s %s\n' "$path" "$(stat -c %a "$path")" "$(sha256sum <"$path" | cut -c1-64)"
        fi
    done)
}

declare -A RC=(
    [fish]="$HOME/.config/fish/config.fish"
    [zsh]="$HOME/.zshrc"
    [bash]="$HOME/.bashrc"
)

step "install $GPY_RELEASE_VERSION from the archive"
install_new bash
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

for shell in fish zsh bash; do
    step "$shell: a second gpy setup"
    rc="${RC[$shell]}"
    setup() { "$LOCAL_BIN/gpy" setup --shell "$shell" --non-interactive --from "$SHELL_TREES"; }
    setup >"$WORK/$shell-first.log" 2>&1 || { show_log "$WORK/$shell-first.log"; abort "the first gpy setup --shell $shell failed"; }
    [[ -f "$rc" ]] || abort "the first gpy setup --shell $shell wrote no $rc"
    before="$(snapshot)"
    rc_mtime="$(stat -c %Y.%y "$rc")"
    sleep 1.1
    if setup >"$WORK/$shell-second.log" 2>&1; then
        pass "the second gpy setup --shell $shell exits 0"
    else
        fail "the second gpy setup --shell $shell failed"
        show_log "$WORK/$shell-second.log"
    fi
    after="$(snapshot)"
    if [[ "$before" == "$after" ]]; then
        pass "the second $shell setup changed no file under \$HOME"
    else
        fail "the second $shell setup changed files under \$HOME:"
        diff <(echo "$before") <(echo "$after") | sed 's/^/    | /' | head -n 40
    fi
    if [[ "$(stat -c %Y.%y "$rc")" == "$rc_mtime" ]]; then
        pass "$rc was not rewritten"
    else
        fail "$rc was rewritten (mtime changed)"
    fi
    if grep -qxF "GPY already configured in $rc" "$WORK/$shell-second.log"; then
        pass "the second $shell setup says GPY is already configured in $rc"
    else
        fail "the second $shell setup does not say 'GPY already configured in $rc'"
        show_log "$WORK/$shell-second.log"
    fi
    if grep -q 'Added GPY initialization\|Updated the GPY block\|Moved your existing' "$WORK/$shell-second.log"; then
        fail "the second $shell setup reports a change: $(grep 'Added GPY initialization\|Updated the GPY block\|Moved your existing' "$WORK/$shell-second.log")"
    fi
done

for shell in fish zsh bash; do
    step "$shell: an rc block cut off after its start marker"
    rc="${RC[$shell]}"
    # Keep everything up to the begin marker and the line after it: the end
    # marker is gone, as when an editor or a sed script truncated the file.
    awk '/^# >>> gpy-init >>>$/ { print; getline; print; exit } { print }' "$rc" >"$WORK/truncated"
    grep -qxF '# >>> gpy-init >>>' "$WORK/truncated" || abort "no begin marker in $rc to truncate"
    if grep -qxF '# <<< gpy-init <<<' "$WORK/truncated"; then abort "the truncated $rc still has its end marker"; fi
    cat "$WORK/truncated" >"$rc"
    cp "$rc" "$WORK/truncated.before"
    rc_status=0
    "$LOCAL_BIN/gpy" setup --shell "$shell" --non-interactive --from "$SHELL_TREES" \
        >"$WORK/$shell-trunc.out" 2>"$WORK/$shell-trunc.err" || rc_status=$?
    if (( rc_status == 5 )); then
        pass "gpy setup --shell $shell exits 5 for the incomplete block in $rc"
    else
        fail "gpy setup --shell $shell exited $rc_status for the incomplete block in $rc, expected 5"
    fi
    if grep -qF "the GPY block in $rc is incomplete" "$WORK/$shell-trunc.err"; then
        pass "gpy setup --shell $shell reports the block in $rc as incomplete (exit $rc_status)"
    else
        fail "gpy setup --shell $shell does not report the block in $rc as incomplete (exit $rc_status)"
        show_log "$WORK/$shell-trunc.err"
    fi
    if grep -q 'already configured' "$WORK/$shell-trunc.out" "$WORK/$shell-trunc.err"; then
        fail "gpy setup --shell $shell calls the truncated block 'already configured'"
    else
        pass "gpy setup --shell $shell does not call the truncated block configured"
    fi
    if cmp -s "$rc" "$WORK/truncated.before"; then
        pass "the truncated $rc was left as it was"
    else
        fail "gpy setup rewrote the truncated $rc: $(diff "$WORK/truncated.before" "$rc" | head -n 20)"
    fi
done

finish
