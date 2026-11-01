#!/usr/bin/env bash
# tests/bash/install_e2e_real_binary.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install.sh -> first prompt -> upgrade -> uninstall, with the real debug
# binaries in a sandboxed HOME; the only installer test not using stub binaries.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command lsof "lsof is needed to find the PID bound to the agent socket"

shell_e2e_init "$ROOT"

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

# Shaped like scripts/package-release.sh output.
PKG="$SHELL_E2E_ROOT/pkg"
mkdir -p "$PKG/bin"
cp -R "$ROOT/fish" "$PKG/fish"
cp -R "$ROOT/bash" "$PKG/bash"
cp "$ROOT/install.sh" "$PKG/install.sh"
cp "$SHELL_E2E_AGENT_BIN" "$PKG/bin/$AGENT_ASSET"
cp "$GPY_CLI" "$PKG/bin/$CLI_ASSET"
for asset in "$AGENT_ASSET" "$CLI_ASSET"; do
    printf '%s  %s\n' "$(sha256_of "$PKG/bin/$asset")" "$asset" >"$PKG/bin/$asset.sha256"
done

export PATH="$HOME/.local/bin:$PATH"
FISH_CONFIG="$XDG_CONFIG_HOME/fish"
mkdir -p "$FISH_CONFIG/functions"
USER_PROMPT="$FISH_CONFIG/functions/fish_prompt.fish"
printf 'function fish_prompt\n    echo "user-prompt> "\nend\n' >"$USER_PROMPT"
cp "$USER_PROMPT" "$SHELL_E2E_ROOT/fish_prompt.orig"

agent_pid() { lsof -t -- "$GPY_AGENT_SOCKET_PATH" 2>/dev/null | head -n 1; }
installed_agent_responding() {
    "$HOME/.local/bin/gpy-agent" status >/dev/null 2>&1
}

echo "--- unsupported login shell: binaries only, no shell check ---"
# A stub CLI that logs its argv stands in for the real one in the two cases below.
STUB_PKG="$SHELL_E2E_ROOT/stub-pkg"
write_stub_pkg() {
    mkdir -p "$STUB_PKG/bin"
    cp "$ROOT/install.sh" "$STUB_PKG/install.sh"
    for asset in "$@"; do
        # shellcheck disable=SC2016  # the stub's own $@ and $0 must stay literal
        printf '#!/bin/sh\nprintf "%%s\\n" "$*" >>"$STUB_LOG"\nexit 0\n' >"$STUB_PKG/bin/$asset"
        chmod +x "$STUB_PKG/bin/$asset"
        printf '%s  %s\n' "$(sha256_of "$STUB_PKG/bin/$asset")" "$asset" >"$STUB_PKG/bin/$asset.sha256"
    done
}
write_stub_pkg "$AGENT_ASSET" "$CLI_ASSET"
STUB_HOME="$SHELL_E2E_ROOT/stub-home-sh"
STUB_LOG="$SHELL_E2E_ROOT/stub-sh.log"
mkdir -p "$STUB_HOME"
: >"$STUB_LOG"
(cd "$STUB_PKG" && env -u GPY_SHELL HOME="$STUB_HOME" SHELL=/bin/sh STUB_LOG="$STUB_LOG" bash ./install.sh) \
    >"$SHELL_E2E_ROOT/install-sh.log" 2>&1
if grep -qF 'setup' "$STUB_LOG"; then
    fail "an unsupported login shell still ran gpy setup: $(cat "$STUB_LOG")"
else
    pass "SHELL=/bin/sh without GPY_SHELL never runs gpy setup (--check or otherwise)"
fi

echo "--- Windows arm: setup --check is never called ---"
SHIM_BIN="$SHELL_E2E_ROOT/shim-bin"
mkdir -p "$SHIM_BIN"
cat >"$SHIM_BIN/uname" <<'EOF'
#!/bin/sh
case "$1" in
    -s) echo MINGW64_NT-10.0 ;;
    *) exec /usr/bin/uname "$@" ;;
esac
EOF
chmod +x "$SHIM_BIN/uname"
rm -rf "$STUB_PKG"
write_stub_pkg gpy-agent-windows-x86_64.exe gpy-windows-x86_64.exe
STUB_HOME="$SHELL_E2E_ROOT/stub-home-win"
STUB_LOG="$SHELL_E2E_ROOT/stub-win.log"
mkdir -p "$STUB_HOME"
: >"$STUB_LOG"
(cd "$STUB_PKG" && env HOME="$STUB_HOME" SHELL=/bin/bash STUB_LOG="$STUB_LOG" PATH="$SHIM_BIN:$PATH" bash ./install.sh) \
    >"$SHELL_E2E_ROOT/install-win.log" 2>&1
if [ -s "$STUB_LOG" ] && ! grep -qF 'setup --check' "$STUB_LOG"; then
    pass "the MINGW arm installs without calling setup --check"
else
    fail "MINGW arm: stub log empty or contains setup --check: $(cat "$STUB_LOG")"
    sed -n '1,30p' "$SHELL_E2E_ROOT/install-win.log"
fi
rm -rf "$STUB_PKG"

echo "--- install ---"
(cd "$PKG" && GPY_SHELL=fish bash ./install.sh) >"$SHELL_E2E_ROOT/install1.log" 2>&1
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
    pass "gpy setup wrote config.toml with show_icons = false"
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
if "$HOME/.local/bin/gpy-agent" status 2>/dev/null | grep -qF "socket: $GPY_AGENT_SOCKET_PATH"; then
    pass "the agent bound the sandboxed socket"
else
    fail "the agent did not report the sandboxed socket: $("$HOME/.local/bin/gpy-agent" status 2>&1 | grep 'socket')"
fi
first_pid="$(agent_pid)"
[ -n "$first_pid" ] || fail "no process holds the agent socket"

backup_count=$(find "$FISH_CONFIG/functions" -name 'fish_prompt.fish.backup.*' | wc -l | tr -d ' ')
if [ "$backup_count" -eq 1 ]; then
    pass "the pre-existing fish_prompt.fish was backed up"
else
    fail "expected one fish_prompt.fish.backup.*, found $backup_count"
fi
if [ -L "$USER_PROMPT" ] && [ "$(readlink "$USER_PROMPT")" = "$XDG_DATA_HOME/gpy/fish/functions/fish_prompt.fish" ]; then
    pass "fish_prompt.fish is a symlink to the installed prompt"
else
    fail "fish_prompt.fish is not the GPY symlink"
fi

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

echo "--- upgrade ---"
# Swap the real daemon for a stub that reports an old version, so the
# installer's `gpy-agent start` has an older agent to replace. The supervisor
# is left alone: uninstall must still stop it.
"$HOME/.local/bin/gpy-agent" stop >/dev/null 2>&1
shell_e2e_stub_listener "$GPY_AGENT_SOCKET_PATH" reply --reply '{"status":"ok"}' \
    --route '"op":"status"' '{"AgentStatus":{"version":"0.0.0-old","protocol_version":2,"watched_repos":0,"registered_clients":0,"cache_entries":0}}' \
    --lifetime 120 --requests "$SHELL_E2E_ROOT/old-agent.requests"
sed -i.bak 's/^show_icons = false/show_icons = true/' "$CONFIG" && rm -f "$CONFIG.bak"
cp "$CONFIG" "$SHELL_E2E_ROOT/config.before-upgrade"

(cd "$PKG" && GPY_SHELL=fish bash ./install.sh) >"$SHELL_E2E_ROOT/install2.log" 2>&1
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
if shell_e2e_poll 5 installed_agent_responding; then
    pass "the upgraded agent is running and responding"
else
    fail "no responding agent after the upgrade"
fi
binary_version="$("$HOME/.local/bin/gpy-agent" --version | awk '{print $2}')"
if grep -q '"op":"shutdown"' "$SHELL_E2E_ROOT/old-agent.requests" 2>/dev/null; then
    pass "the old agent was asked to shut down"
else
    fail "the old agent never received a shutdown request"
fi
if "$HOME/.local/bin/gpy-agent" status 2>/dev/null | grep -q "^version: $binary_version\$"; then
    pass "the upgraded agent reports the binary's version ($binary_version)"
else
    fail "the upgraded agent does not report version $binary_version"
fi

echo "--- uninstall ---"
SUPERVISOR_PIDFILE="$XDG_RUNTIME_DIR/gpy/supervisor.pid"
supervisor_pid="$(cat "$SUPERVISOR_PIDFILE" 2>/dev/null || true)"
"$HOME/.local/bin/gpy" uninstall --yes --purge-config >"$SHELL_E2E_ROOT/uninstall.log" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
    pass "gpy uninstall exited 0"
else
    fail "gpy uninstall exited $rc"
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
leftovers="$(find "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" -iname '*gpy*' 2>/dev/null)"
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

test_finish "install.sh installs, upgrades and uninstalls with the real binaries"
