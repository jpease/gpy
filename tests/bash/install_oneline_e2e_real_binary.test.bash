#!/usr/bin/env bash
# tests/bash/install_oneline_e2e_real_binary.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh -> first prompt -> uninstall, per shell, with the real
# binaries. A fake `curl` serves release assets from the debug build.
# Upgrade is covered in install_e2e_real_binary.test.bash.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command zsh
test_require_command python3

GPY_CLI="$ROOT/gpy-agent/target/debug/gpy"
GPY_AGENT="$ROOT/gpy-agent/target/debug/gpy-agent"
if [ ! -x "$GPY_CLI" ] || [ ! -x "$GPY_AGENT" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
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
    *) test_skip "install-oneline.sh does not support $(uname -s)" ;;
esac

# `-o FILE` downloads; a bare URL prints to stdout (.sha256 sidecars).
write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
ROOT="$ROOT"
AGENT_BIN="$GPY_AGENT"
CLI_BIN="$GPY_CLI"
AGENT_ASSET="$AGENT_ASSET"
CLI_ASSET="$CLI_ASSET"
SHELL_ARCHIVE="$SHELL_ARCHIVE"
EOF
    cat >>"$1/curl" <<'EOF'
out=""; url=""; prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then out="$arg"; else case "$arg" in -*) ;; *) url="$arg" ;; esac; fi
    prev="$arg"
done
digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
source_for() {
    case "$1" in
        */releases/download/*/gpy-shell-files.tar.gz) echo "$SHELL_ARCHIVE" ;;
        */releases/download/*/"$AGENT_ASSET") echo "$AGENT_BIN" ;;
        */releases/download/*/"$CLI_ASSET") echo "$CLI_BIN" ;;
        *) echo "" ;;
    esac
}
case "$url" in
    *.sha256)
        src="$(source_for "${url%.sha256}")"
        [ -n "$src" ] && [ -f "$src" ] || exit 22
        base="${url##*/}"
        printf '%s  %s\n' "$(digest_of "$src")" "${base%.sha256}"
        exit 0
        ;;
esac
src="$(source_for "$url")"
[ -n "$src" ] && [ -f "$src" ] || exit 22
if [ -n "$out" ]; then
    mkdir -p "$(dirname "$out")"
    cp "$src" "$out"
else
    cat "$src"
fi
exit 0
EOF
    chmod +x "$1/curl"
}

run_shell() {
    shell="$1"
    echo "=== $shell ==="
    shell_e2e_init "$ROOT"
    export PATH="$HOME/.local/bin:$SHELL_E2E_ROOT/fakebin:$PATH"
    mkdir -p "$SHELL_E2E_ROOT/fakebin"
    SHELL_ARCHIVE="$SHELL_E2E_ROOT/gpy-shell-files.tar.gz"
    shell_e2e_build_shell_archive "$ROOT" "$SHELL_ARCHIVE" || { fail "$shell: could not build the shell archive"; return; }
    write_fake_curl "$SHELL_E2E_ROOT/fakebin"
    # The rc file each shell reads on an interactive start; the installer
    # writes its block to the same one (config.fish under XDG_CONFIG_HOME).
    case "$shell" in
        fish) rc_file="$XDG_CONFIG_HOME/fish/config.fish" ;;
        zsh) rc_file="$HOME/.zshrc" ;;
        bash) rc_file="$HOME/.bashrc" ;;
    esac
    mkdir -p "$(dirname "$rc_file")"
    printf '# user rc before gpy\n' >"$rc_file"
    cp "$rc_file" "$SHELL_E2E_ROOT/rc.orig"

    (cd "$SHELL_E2E_ROOT" && GPY_SHELL="$shell" GPY_VERSION=v9.9.9-test sh "$ROOT/install-oneline.sh") \
        >"$SHELL_E2E_ROOT/install.log" 2>&1
    rc=$?
    if [ "$rc" -eq 0 ]; then
        pass "$shell: install-oneline.sh exited 0"
    else
        fail "$shell: install-oneline.sh exited $rc"
        sed -n '1,60p' "$SHELL_E2E_ROOT/install.log"
    fi
    if grep -Eq 'Uninstall:.*/releases/download/v9\.9\.9-test/uninstall\.(sh|fish)' "$SHELL_E2E_ROOT/install.log"; then
        pass "$shell: the uninstall hint names the release's uninstall script"
    else
        fail "$shell: the uninstall hint does not name the release's uninstall script: $(grep -i uninstall "$SHELL_E2E_ROOT/install.log")"
    fi
    grep -qF '# >>> gpy-init >>>' "$rc_file" || fail "$shell: no gpy-init block in $rc_file"

    case "$shell" in
        fish)
            comp="$XDG_CONFIG_HOME/fish/completions/gpy.fish"
            if [ -s "$comp" ] && fish -n "$comp" 2>/dev/null; then
                pass "$shell: structural completions parse"
            else
                fail "$shell: structural completions missing or unparsable at $comp"
            fi
            ;;
        zsh)
            comp="$XDG_CONFIG_HOME/gpy/zsh/completions/_gpy"
            if [ -s "$comp" ] && zsh -n "$comp" 2>/dev/null; then
                pass "$shell: structural completions parse"
            else
                fail "$shell: structural completions missing or unparsable at $comp"
            fi
            ;;
        bash)
            comp="$XDG_CONFIG_HOME/gpy/bash/completions/gpy.bash"
            if [ -s "$comp" ] && bash -n "$comp" 2>/dev/null; then
                pass "$shell: structural completions parse"
            else
                fail "$shell: structural completions missing or unparsable at $comp"
            fi
            ;;
    esac

    responding() { "$HOME/.local/bin/gpy-agent" status 2>/dev/null | grep -q 'Running and Responding'; }
    if shell_e2e_poll 5 responding; then
        pass "$shell: the installed agent is running and responding"
    else
        fail "$shell: the installed agent does not respond"
    fi

    # The installer's --from is temporary; a later bare setup must still work.
    if "$HOME/.local/bin/gpy" setup --shell "$shell" --non-interactive >"$SHELL_E2E_ROOT/resetup.log" 2>&1; then
        pass "$shell: a bare gpy setup works after the install"
    else
        fail "$shell: a bare gpy setup failed after the install: $(tail -n 3 "$SHELL_E2E_ROOT/resetup.log")"
    fi

    cd "$SHELL_E2E_REPO" || exit 1
    case "$shell" in
        fish) python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- fish -i ;;
        zsh) python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- zsh -d -i ;;
        bash) python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- bash -i ;;
    esac || fail "$shell: could not spawn the shell on a pty"
    if shell_e2e_wait_for 'repo' 15 >/dev/null; then
        pass "$shell: the first prompt names the repository directory"
    else
        fail "$shell: no prompt naming the repository within 15 s"
    fi
    transcript="$(shell_e2e_transcript 0)"
    case "$transcript" in
        *"gpy[init]"* | *"command not found"* | *"No such file"* | *"parse error"* | *"syntax error"*)
            fail "$shell: the first prompt reported an error: $(printf '%s' "$transcript" | tail -n 5)" ;;
        *) pass "$shell: no error text around the first prompt" ;;
    esac
    shell_e2e_send 'exit\r'
    shell_e2e_stop_client
    cd "$SHELL_E2E_ROOT" || exit 1

    supervisor_pid="$(cat "$XDG_RUNTIME_DIR/gpy/supervisor.pid" 2>/dev/null || true)"
    printf '\n' | GPY_SHELL="$shell" sh "$ROOT/scripts/uninstall.sh" >"$SHELL_E2E_ROOT/uninstall.log" 2>&1
    rc=$?
    if [ "$rc" -eq 0 ]; then
        pass "$shell: the uninstaller exited 0"
    else
        fail "$shell: the uninstaller exited $rc"
        cat "$SHELL_E2E_ROOT/uninstall.log"
    fi
    if cmp -s "$rc_file" "$SHELL_E2E_ROOT/rc.orig"; then
        pass "$shell: the rc file is byte-identical after uninstall"
    else
        fail "$shell: the rc file changed: $(cat "$rc_file")"
    fi
    if "$SHELL_E2E_AGENT_BIN" status >/dev/null 2>&1; then
        fail "$shell: gpy-agent status still succeeds after uninstall"
    else
        pass "$shell: gpy-agent status fails after uninstall"
    fi
    if [ -n "$supervisor_pid" ] && kill -0 "$supervisor_pid" 2>/dev/null; then
        fail "$shell: the supervisor loop $supervisor_pid is still running"
        kill "$supervisor_pid" 2>/dev/null || true
    fi
    leftovers="$(find "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" -iname '*gpy*' 2>/dev/null)"
    if [ -z "$leftovers" ]; then
        pass "$shell: nothing named *gpy* remains under HOME or the XDG directories"
    else
        fail "$shell: uninstall left behind:"
        printf '%s\n' "$leftovers"
    fi
    shell_e2e_cleanup
    trap - EXIT
}

run_shell fish
run_shell zsh
run_shell bash

test_finish "install-oneline.sh installs, renders a first prompt and uninstalls for fish, zsh and bash"
