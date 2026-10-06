#!/usr/bin/env bash
# tests/bash/install_e2e_real_binary.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install.sh -> first prompt -> upgrade -> uninstall, with the real binaries
# (#649, pins #642).
#
# Every other installer test substitutes `#!/bin/sh\nexit 0` for the
# binaries, so `gpy-agent init` writing config.toml, `gpy-agent start`
# daemonising against the sandboxed XDG directories, `gpy completions fish`
# producing loadable output at the installed path, and the first shell
# loading through the rc block were never exercised together. This test
# builds a package shaped like an extracted gpy-release.tar.gz around the
# debug binaries and drives the whole lifecycle in a sandboxed HOME:
#
#   install   exit 0; config.toml written with show_icons = false (the
#             GPY_NERD_FONT=none override); structural completions pass
#             `fish -n`; the installed agent answers `status` on the
#             sandboxed socket; a real `fish -i` on a pty renders a prompt
#             naming the repository through config.fish's gpy-init block,
#             with no gpy[init] error; a pre-existing fish_prompt.fish is
#             backed up
#   upgrade   agent.version rewritten as 0.0.0-old and config.toml edited;
#             the installer re-run leaves config.toml byte-identical, backs
#             the old binary up, and replaces the running agent (new PID,
#             agent.version now the binary's version)
#   uninstall scripts/uninstall.fish (shipped in the archive) restores the
#             user's fish_prompt.fish byte-identically, stops the agent and
#             the supervisor, and leaves nothing named *gpy* under HOME or
#             any XDG directory
#
# Nothing here touches the checkout: the package, HOME and every XDG
# directory live under the GPY test root the harness owns (#619).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command lsof "lsof is needed to find the PID bound to the agent socket"

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

GPY_CLI="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$GPY_CLI" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy) || {
        echo "FAIL: could not build the gpy CLI"
        exit 1
    }
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
else
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
            aarch64 | arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    darwin)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
            arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    *) test_skip "install.sh does not support $(uname -s)" ;;
esac

# --- the package ---------------------------------------------------------------------
# Shaped like scripts/package-release.sh's stage: bin/ with asset-named
# binaries plus sidecars, the fish/ tree, install.sh, and scripts/uninstall.*.
PKG="$SHELL_E2E_ROOT/pkg"
mkdir -p "$PKG/bin" "$PKG/scripts"
cp -R "$ROOT/fish" "$PKG/fish"
cp "$ROOT/install.sh" "$PKG/install.sh"
cp "$ROOT/scripts/uninstall.fish" "$ROOT/scripts/uninstall.sh" "$PKG/scripts/"
cp "$SHELL_E2E_AGENT_BIN" "$PKG/bin/$AGENT_ASSET"
cp "$GPY_CLI" "$PKG/bin/$CLI_ASSET"
for asset in "$AGENT_ASSET" "$CLI_ASSET"; do
    printf '%s  %s\n' "$(sha256_of "$PKG/bin/$asset")" "$asset" >"$PKG/bin/$asset.sha256"
done

# The installed binaries must be the ones every later command resolves.
export PATH="$HOME/.local/bin:$PATH"
FISH_CONFIG="$XDG_CONFIG_HOME/fish"
mkdir -p "$FISH_CONFIG/functions"
USER_PROMPT="$FISH_CONFIG/functions/fish_prompt.fish"
printf 'function fish_prompt\n    echo "user-prompt> "\nend\n' >"$USER_PROMPT"
cp "$USER_PROMPT" "$SHELL_E2E_ROOT/fish_prompt.orig"

agent_pid() { lsof -t -- "$GPY_AGENT_SOCKET_PATH" 2>/dev/null | head -n 1; }
installed_agent_responding() {
    "$HOME/.local/bin/gpy-agent" status 2>/dev/null | grep -q 'Running and Responding'
}

# =====================================================================================
echo "--- install ---"
(cd "$PKG" && bash ./install.sh) >"$SHELL_E2E_ROOT/install1.log" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
    pass "install.sh exited 0"
else
    fail "install.sh exited $rc"
    sed -n '1,60p' "$SHELL_E2E_ROOT/install1.log"
fi
[ -x "$HOME/.local/bin/gpy-agent" ] || fail "no agent binary at ~/.local/bin/gpy-agent"
[ -x "$HOME/.local/bin/gpy" ] || fail "no CLI binary at ~/.local/bin/gpy"

CONFIG="$XDG_CONFIG_HOME/gpy/config.toml"
if [ -f "$CONFIG" ] && grep -q '^show_icons = false' "$CONFIG"; then
    pass "gpy-agent init wrote config.toml with show_icons = false"
else
    fail "config.toml missing or without show_icons = false: $(cat "$CONFIG" 2>/dev/null)"
fi

COMPLETIONS="$FISH_CONFIG/completions/gpy.fish"
if [ -s "$COMPLETIONS" ] && fish -n "$COMPLETIONS" 2>/dev/null; then
    pass "structural completions were generated by the installed gpy and parse"
else
    fail "structural completions missing or unparsable at $COMPLETIONS"
fi
grep -q '__gpy_complete_cached' "$COMPLETIONS" || fail "dynamic completions glue was not appended to gpy.fish"
[ ! -e "$FISH_CONFIG/completions/gpy-dynamic.fish" ] || fail "standalone gpy-dynamic.fish (never autoloaded) was installed"

if shell_e2e_poll 5 installed_agent_responding; then
    pass "the installed agent is running and responding"
else
    fail "the installed agent does not respond: $("$HOME/.local/bin/gpy-agent" status 2>&1)"
fi
if "$HOME/.local/bin/gpy-agent" status 2>/dev/null | grep -qF "Socket Path: $GPY_AGENT_SOCKET_PATH"; then
    pass "the agent bound the sandboxed socket"
else
    fail "the agent did not report the sandboxed socket: $("$HOME/.local/bin/gpy-agent" status 2>&1 | grep 'Socket')"
fi
first_pid="$(agent_pid)"
[ -n "$first_pid" ] || fail "no process holds the agent socket"

backup_count=$(find "$FISH_CONFIG/functions" -name 'fish_prompt.fish.gpy-backup.*' | wc -l | tr -d ' ')
if [ "$backup_count" -eq 1 ]; then
    pass "the pre-existing fish_prompt.fish was backed up"
else
    fail "expected one fish_prompt.fish.gpy-backup.*, found $backup_count"
fi
if [ -L "$USER_PROMPT" ] && [ "$(readlink "$USER_PROMPT")" = "$FISH_CONFIG/gpy/functions/fish_prompt.fish" ]; then
    pass "fish_prompt.fish is a symlink to the installed prompt"
else
    fail "fish_prompt.fish is not the GPY symlink"
fi

# The first shell: a real interactive fish reading the sandboxed config.fish.
echo "--- first prompt through config.fish ---"
cd "$SHELL_E2E_REPO" || exit 1
python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- fish -i || fail "could not spawn fish on a pty"
if shell_e2e_wait_for 'repo' 15 >/dev/null; then
    pass "the first prompt names the repository directory"
else
    fail "no prompt naming the repository within 15 s"
fi
transcript="$(shell_e2e_transcript 0)"
case "$transcript" in
    *"gpy[init]"* | *"user-prompt>"*) fail "the first prompt went wrong: $(printf '%s' "$transcript" | tail -n 5)" ;;
    *) pass "no gpy[init] error and the user's old prompt is not in use" ;;
esac
shell_e2e_send 'exit\r'
shell_e2e_stop_client
cd "$SHELL_E2E_ROOT" || exit 1

# =====================================================================================
echo "--- upgrade ---"
# The harness overrides the socket path, so the agent records its version next
# to the socket rather than in the runtime root's agent.version (#780).
VERSION_FILE="$GPY_AGENT_SOCKET_PATH.version"
[ -f "$VERSION_FILE" ] || fail "no version marker at $VERSION_FILE"
printf '0.0.0-old\n' >"$VERSION_FILE"
sed -i.bak 's/^show_icons = false/show_icons = true/' "$CONFIG" && rm -f "$CONFIG.bak"
cp "$CONFIG" "$SHELL_E2E_ROOT/config.before-upgrade"

(cd "$PKG" && bash ./install.sh) >"$SHELL_E2E_ROOT/install2.log" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
    pass "the installer re-run exited 0"
else
    fail "the installer re-run exited $rc"
    sed -n '1,60p' "$SHELL_E2E_ROOT/install2.log"
fi
if cmp -s "$CONFIG" "$SHELL_E2E_ROOT/config.before-upgrade"; then
    pass "config.toml is byte-identical across the upgrade"
else
    fail "config.toml changed across the upgrade"
    diff -u "$SHELL_E2E_ROOT/config.before-upgrade" "$CONFIG"
fi
if ls "$HOME/.local/bin/gpy-agent.backup."* >/dev/null 2>&1; then
    pass "the previous agent binary was backed up"
else
    fail "no gpy-agent.backup.* beside the installed binary"
fi
old_gone() { ! kill -0 "$first_pid" 2>/dev/null; }
if [ -n "$first_pid" ] && shell_e2e_poll 10 old_gone; then
    pass "the old agent process ($first_pid) was replaced"
else
    fail "the old agent process $first_pid is still alive after the upgrade"
fi
if shell_e2e_poll 5 installed_agent_responding; then
    pass "the upgraded agent is running and responding"
else
    fail "no responding agent after the upgrade"
fi
second_pid="$(agent_pid)"
if [ -z "$second_pid" ] || [ "$second_pid" = "$first_pid" ]; then
    fail "expected a new agent PID, got '$second_pid'"
fi
binary_version="$("$HOME/.local/bin/gpy-agent" --version | awk '{print $2}')"
if [ "$(tr -d '[:space:]' <"$VERSION_FILE")" = "$binary_version" ]; then
    pass "agent.version records the upgraded binary's version ($binary_version)"
else
    fail "agent.version is '$(cat "$VERSION_FILE")', expected $binary_version"
fi

# =====================================================================================
echo "--- uninstall ---"
SUPERVISOR_PIDFILE="$XDG_RUNTIME_DIR/gpy/supervisor.pid"
supervisor_pid="$(cat "$SUPERVISOR_PIDFILE" 2>/dev/null || true)"
printf '\n' | fish "$PKG/scripts/uninstall.fish" >"$SHELL_E2E_ROOT/uninstall.log" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
    pass "uninstall.fish exited 0"
else
    fail "uninstall.fish exited $rc"
    cat "$SHELL_E2E_ROOT/uninstall.log"
fi

if [ -f "$USER_PROMPT" ] && [ ! -L "$USER_PROMPT" ] && cmp -s "$USER_PROMPT" "$SHELL_E2E_ROOT/fish_prompt.orig"; then
    pass "the user's fish_prompt.fish was restored byte-identically"
else
    fail "fish_prompt.fish was not restored: $(ls -la "$FISH_CONFIG/functions" 2>&1)"
fi
if "$SHELL_E2E_AGENT_BIN" status >/dev/null 2>&1; then
    fail "gpy-agent status still succeeds after uninstall"
else
    pass "gpy-agent status fails after uninstall"
fi
if [ -n "$supervisor_pid" ]; then
    supervisor_gone() { ! kill -0 "$supervisor_pid" 2>/dev/null; }
    if shell_e2e_poll 5 supervisor_gone; then
        pass "the supervisor loop ($supervisor_pid) was stopped"
    else
        fail "the supervisor loop $supervisor_pid is still running"
        kill "$supervisor_pid" 2>/dev/null || true
    fi
else
    pass "no supervisor pid file was left to stop (none recorded)"
fi
leftovers="$(find "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" -iname '*gpy*' 2>/dev/null)"
if [ -z "$leftovers" ]; then
    pass "nothing named *gpy* remains under HOME or the XDG directories"
else
    fail "uninstall left behind:"
    printf '%s\n' "$leftovers"
fi
if grep -qF 'gpy-init' "$FISH_CONFIG/config.fish" 2>/dev/null; then
    fail "the gpy-init block is still in config.fish"
else
    pass "the gpy-init block was removed from config.fish"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: install.sh installs, upgrades and uninstalls with the real binaries"
