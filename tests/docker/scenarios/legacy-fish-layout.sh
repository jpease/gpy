#!/usr/bin/env bash
# tests/docker/scenarios/legacy-fish-layout.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A user with their own fish_prompt and config.fish installs v0.8.0, the last
# release with the pre-0.9 layout (the Fish tree in ~/.config/fish/gpy, the
# bundled plugins in ~/.config/gpy). No published installer put a loader in
# ~/.config/fish/conf.d: the checkout scripts of that era did (install-dev.fish
# and scripts/update.fish copied fish/conf.d/gpy_init.fish there), so the
# scenario adds that copy the same way. Upgrading to the fresh build must
# retire that loader, report it, and list the old directories to delete;
# `gpy uninstall` must then hand back the user's own files byte for byte, and
# name the moved-aside loader it leaves in place.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

OLD_TAG=v0.8.0
FISH_DIR="$HOME/.config/fish"
LOADER="$FISH_DIR/conf.d/gpy_init.fish"
PROMPT="$FISH_DIR/functions/fish_prompt.fish"
CONFIG="$FISH_DIR/config.fish"

step "the user's own Fish files"
mkdir -p "$FISH_DIR/functions"
printf '%s\n' '# my prompt, not GPY'"'"'s' 'function fish_prompt' '    printf "mine> "' 'end' >"$PROMPT"
printf '%s\n' '# my config.fish' 'set -gx EDITOR vi' 'alias ll "ls -l"' >"$CONFIG"
cp "$PROMPT" "$WORK/user-prompt.fish"
cp "$CONFIG" "$WORK/user-config.fish"
pass "seeded a user-owned fish_prompt.fish and config.fish"

step "install $OLD_TAG from its published assets"
install_release "$OLD_TAG" fish
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true
# The layout this scenario exists for; without it the rest proves nothing.
[[ -d "$FISH_DIR/gpy" ]] || abort "$OLD_TAG left no $FISH_DIR/gpy tree"
[[ -f "$FISH_DIR/gpy/conf.d/gpy_init.fish" ]] || abort "the $OLD_TAG tree has no conf.d/gpy_init.fish"
cmp -s "$PROMPT" "$WORK/user-prompt.fish" &&
    abort "$OLD_TAG did not replace the user's fish_prompt.fish, so there is nothing to restore"
# What install-dev.fish and scripts/update.fish of that era did.
mkdir -p "$FISH_DIR/conf.d"
cp "$FISH_DIR/gpy/conf.d/gpy_init.fish" "$LOADER"
pass "$OLD_TAG installed the pre-0.9 layout ($FISH_DIR/gpy), with its loader copied to $LOADER"
legacy_dirs=()
for dir in "$FISH_DIR/gpy" "$HOME/.config/gpy/bash" "$HOME/.config/gpy/zsh" "$HOME/.config/gpy/bundled-plugins"; do
    [[ -e "$dir" ]] && legacy_dirs+=("$dir")
done
echo "legacy directories on disk: ${legacy_dirs[*]}"

step "upgrade to $GPY_RELEASE_VERSION; its gpy setup retires the loader"
if upgrade_to_new fish "$WORK/upgrade.log"; then
    pass "install-oneline.sh $GPY_RELEASE_VERSION exited 0 over the $OLD_TAG install"
else
    show_log "$WORK/upgrade.log"
    abort "install-oneline.sh $GPY_RELEASE_VERSION failed over the $OLD_TAG install"
fi
if grep -qE "^(Removed|Moved) the old Fish loader $LOADER" "$WORK/upgrade.log"; then
    pass "gpy setup reports retiring the old loader: $(grep -E 'the old Fish loader' "$WORK/upgrade.log")"
else
    fail "gpy setup did not report retiring $LOADER"
    show_log "$WORK/upgrade.log"
fi
if [[ -e "$LOADER" || -L "$LOADER" ]]; then
    fail "$LOADER is still in conf.d, so Fish keeps loading the old tree"
else
    pass "$LOADER is gone from conf.d"
fi
if grep -qF 'belong to an older GPY layout' "$WORK/upgrade.log"; then
    pass "gpy setup says the old directories are no longer used"
else
    fail "gpy setup did not report the old layout's directories"
fi
# setup prints each as `  rm -rf <shell-quoted path>`.
for dir in "${legacy_dirs[@]}"; do
    if grep -qxE "  rm -rf '?${dir//./\\.}'?" "$WORK/upgrade.log"; then
        pass "gpy setup lists '$dir' for removal"
    else
        fail "gpy setup does not list '$dir' for removal"
    fi
done
moved_loader="$(find "$FISH_DIR/conf.d" -maxdepth 1 -name 'gpy_init.fish.backup.*' | head -n 1)"
if [[ -n "$moved_loader" ]] && cmp -s "$moved_loader" "$FISH_DIR/gpy/conf.d/gpy_init.fish"; then
    pass "the moved-aside loader keeps the old bytes, under a name Fish does not load"
else
    fail "no intact copy of the old loader beside conf.d/gpy_init.fish: $(ls -la "$FISH_DIR/conf.d" 2>&1)"
fi
fish_out="$(fish -i -c 'fish_prompt' </dev/null 2>&1 || true)"
case "$fish_out" in
    *"mine>"* | *rror* | *"gpy[init]"* | "") fail "fish does not render GPY's prompt after the upgrade: $fish_out" ;;
    *) pass "fish renders GPY's prompt from the new layout" ;;
esac

step "gpy uninstall hands back the user's files"
uninstall_rc=0
"$LOCAL_BIN/gpy" uninstall --yes >"$WORK/uninstall.log" 2>&1 || uninstall_rc=$?
if (( uninstall_rc == 0 )); then
    pass "gpy uninstall --yes exits 0"
else
    fail "gpy uninstall --yes exited $uninstall_rc"
    show_log "$WORK/uninstall.log"
fi
if [[ -f "$PROMPT" && ! -L "$PROMPT" ]] && cmp -s "$PROMPT" "$WORK/user-prompt.fish"; then
    pass "fish_prompt.fish is the user's own, byte for byte"
else
    fail "fish_prompt.fish is not the user's original: $(ls -la "$FISH_DIR/functions" 2>&1)"
fi
if cmp -s "$CONFIG" "$WORK/user-config.fish"; then
    pass "config.fish is the user's own, byte for byte"
else
    fail "config.fish differs from the user's original: $(diff "$WORK/user-config.fish" "$CONFIG" 2>&1 | head -n 20)"
fi
grep -F "${moved_loader:-no moved loader}" "$WORK/uninstall.log" >"$WORK/loader-line" || true
if [[ -n "$moved_loader" ]] && grep -qF 'Fish does not load them' "$WORK/loader-line"; then
    pass "gpy uninstall names the moved-aside loader as not loaded by Fish"
else
    fail "gpy uninstall does not name the moved-aside loader '$moved_loader'"
    show_log "$WORK/uninstall.log"
fi
if [[ -n "$moved_loader" && -f "$moved_loader" ]]; then
    pass "gpy uninstall left the moved-aside loader in place"
else
    fail "gpy uninstall deleted the moved-aside loader '$moved_loader'"
fi
# The loader setup moved aside is not GPY's to delete (it was not a link to a
# GPY tree), so uninstall leaves it; Fish loads only *.fish from conf.d.
leftover="$(find "$FISH_DIR" "$HOME/.local/share/gpy" -name '*gpy*' ! -name 'gpy_init.fish.backup.*' 2>/dev/null || true)"
if [[ -z "$leftover" ]]; then
    pass "no GPY file is left under $FISH_DIR or ~/.local/share/gpy"
else
    fail "GPY files left after uninstall: $(echo "$leftover" | tr '\n' ' ')"
    show_log "$WORK/uninstall.log"
fi

finish
