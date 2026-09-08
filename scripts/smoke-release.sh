#!/usr/bin/env bash
# scripts/smoke-release.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Run the binaries a release is about to publish (#652).
#
# release.yml built, packaged, attested and published without ever executing
# a built binary, so a tag could ship an agent that fails on start. This
# takes the packaged payload (the directory actions/download-artifact wrote,
# or any directory holding gpy-release.tar.gz) and, in a sandboxed HOME
# detached from any terminal:
#
#   Linux/macOS  extracts the archive, runs its install.sh, and asserts
#                 - `gpy-agent --version` and `gpy --version` report VERSION
#                 - `gpy-agent oneshot git` renders a non-empty line for a
#                   git repository (the no-daemon path)
#                 - the installed agent answers `status`
#                 - fish, zsh and bash each render one prompt through the
#                   installed integration (fish via config.fish's gpy-init
#                   block; zsh/bash from the archive's zsh/ and bash/ trees
#                   laid out as install-oneline.sh lays them out), with the
#                   repository's directory name in it and no gpy error text
#   Windows       runs `--version` on both .exe files and `gpy --help`
#                 (native Windows is CLI-only)
#
# Any non-zero exit or empty output fails the run. Usage:
#   scripts/smoke-release.sh --package <dir> --version vX.Y.Z [--shell fish,zsh,bash]

set -euo pipefail

PACKAGE_DIR=""
VERSION=""
SHELLS="fish,zsh,bash"

die() {
    echo "smoke-release: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PACKAGE_DIR="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --shell) SHELLS="$2"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done
[[ -n "$PACKAGE_DIR" ]] || die "--package <dir> is required"
[[ -n "$VERSION" ]] || die "--version vX.Y.Z is required"
BARE="${VERSION#v}"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# `poll SECS CMD...` -- every 100 ms until CMD exits 0 or SECS pass.
poll() {
    local attempts=$(( $1 * 10 ))
    shift
    while (( attempts > 0 )); do
        "$@" && return 0
        sleep 0.1
        attempts=$((attempts - 1))
    done
    return 1
}

# `check_version BIN NAME` -- `BIN --version` prints `NAME BARE`.
check_version() {
    local out
    if out="$("$1" --version 2>&1)" && [[ "$out" == "$2 $BARE" ]]; then
        pass "$2 --version reports $BARE"
    else
        fail "$2 --version: expected '$2 $BARE', got '${out:-<no output>}'"
    fi
}

SANDBOX="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gpy-smoke.XXXXXX")"
cleanup() {
    if [[ -n "${AGENT_BIN:-}" && -x "$AGENT_BIN" ]]; then
        "$AGENT_BIN" stop >/dev/null 2>&1 || true
    fi
    rm -rf "$SANDBOX"
}
trap cleanup EXIT

# --- Windows: CLI-only ---------------------------------------------------------------
if [[ "${RUNNER_OS:-$(uname -s)}" == Windows* || "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* ]]; then
    echo "--- Windows: the packaged CLI binaries run ---"
    bin_dir="$PACKAGE_DIR/gpy-release/bin"
    [[ -d "$bin_dir" ]] || die "no gpy-release/bin under $PACKAGE_DIR"
    agent_exe="$bin_dir/gpy-agent-windows-x86_64.exe"
    cli_exe="$bin_dir/gpy-windows-x86_64.exe"
    [[ -f "$agent_exe" ]] || fail "missing $agent_exe"
    [[ -f "$cli_exe" ]] || fail "missing $cli_exe"
    if (( failures == 0 )); then
        check_version "$agent_exe" gpy-agent
        check_version "$cli_exe" gpy
        if "$cli_exe" --help 2>&1 | grep -q 'Usage:'; then
            pass "gpy --help prints usage"
        else
            fail "gpy --help printed no usage"
        fi
    fi
    (( failures == 0 )) || { echo "FAILED: $failures assertion(s)"; exit 1; }
    echo "PASS: the packaged Windows binaries run"
    exit 0
fi

# --- Unix: install from the archive and render a prompt in every shell ---------------
echo "--- the documented IPC transport is present ---"
if command -v socat >/dev/null 2>&1 || command -v nc >/dev/null 2>&1; then
    pass "socat or nc is installed (docs/INSTALL.md, IPC transport dependencies)"
else
    fail "neither socat nor nc is installed; Fish and Bash would only get the degraded oneshot prompt"
fi

echo "--- extract the archive ---"
archive="$PACKAGE_DIR/gpy-release.tar.gz"
[[ -f "$archive" ]] || die "no gpy-release.tar.gz under $PACKAGE_DIR"
mkdir -p "$SANDBOX/extract"
tar -xzf "$archive" -C "$SANDBOX/extract"
PKG="$SANDBOX/extract/gpy-release"
[[ -f "$PKG/install.sh" ]] || die "the archive has no install.sh"

export HOME="$SANDBOX/home"
export XDG_CONFIG_HOME="$SANDBOX/config"
export XDG_CACHE_HOME="$SANDBOX/cache"
export XDG_RUNTIME_DIR="$SANDBOX/runtime"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/gpy.sock"
export GPY_NERD_FONT=none
export GPY_AGENT_SUPERVISOR_ENABLED=0
# A runner has no terminal; fish warns and picks a fallback without this.
export TERM="${TERM:-xterm-256color}"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE 2>/dev/null || true

echo "--- install.sh from the archive ---"
if (cd "$PKG" && bash ./install.sh </dev/null) >"$SANDBOX/install.log" 2>&1; then
    pass "install.sh exited 0"
else
    fail "install.sh failed:"
    sed -n '1,60p' "$SANDBOX/install.log"
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
AGENT_BIN="$HOME/.local/bin/gpy-agent"
CLI_BIN="$HOME/.local/bin/gpy"
export PATH="$HOME/.local/bin:$PATH"
[[ -x "$AGENT_BIN" ]] || fail "install.sh did not install gpy-agent"
[[ -x "$CLI_BIN" ]] || fail "install.sh did not install gpy"

echo "--- the binaries report the release version ---"
check_version "$AGENT_BIN" gpy-agent
check_version "$CLI_BIN" gpy

echo "--- a repository to render ---"
REPO="$SANDBOX/repo"
mkdir -p "$REPO"
git -C "$REPO" init -q -b main
git -C "$REPO" config user.email smoke@example.com
git -C "$REPO" config user.name Smoke
git -C "$REPO" config commit.gpgsign false
echo hello >"$REPO/tracked.txt"
git -C "$REPO" add tracked.txt
git -C "$REPO" commit -qm init

echo "--- oneshot render without the daemon ---"
oneshot="$(GPY_AGENT_SOCKET_PATH="$SANDBOX/no-such.sock" "$AGENT_BIN" oneshot git --cwd "$REPO" --format ansi 2>&1 || true)"
if [[ -n "$oneshot" && "$oneshot" == *main* ]]; then
    pass "gpy-agent oneshot git renders the branch"
else
    fail "gpy-agent oneshot git printed '${oneshot:-<nothing>}'"
fi

echo "--- the installed agent answers ---"
agent_responding() { "$AGENT_BIN" status 2>/dev/null | grep -q 'Running and Responding'; }
if poll 10 agent_responding; then
    pass "gpy-agent status: Running and Responding"
else
    fail "the installed agent does not respond: $("$AGENT_BIN" status 2>&1 | head -n 3)"
fi

# The prompt text with every terminal escape stripped.
strip_escapes() {
    python3 -c '
import re, sys
t = sys.stdin.read()
t = re.sub(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\\\)", "", t)   # OSC
t = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", t)             # CSI
t = re.sub(r"\x1b\([A-Za-z0-9]", "", t)                       # charset select ESC ( B
t = re.sub(r"\\\[|\\\]", "", t)                              # bash \[ \]
t = re.sub("[\uE0B0-\uE0BF]", "", t)                        # powerline chevrons
sys.stdout.write(t)
'
}

# `assert_prompt SHELL TEXT` -- the rendered prompt names the repo and has no error text.
assert_prompt() {
    local shell="$1" text="$2" plain
    plain="$(printf '%s' "$text" | strip_escapes)"
    if [[ -z "$plain" ]]; then
        fail "$shell rendered an empty prompt"
        return
    fi
    case "$plain" in
        *"gpy[init]"* | *"command not found"* | *"No such file"* | *"error"* | *"Error"*)
            fail "$shell prompt carries error text: $plain" ;;
        *repo*)
            pass "$shell renders a prompt naming the repository" ;;
        *)
            fail "$shell prompt does not name the repository: $plain" ;;
    esac
}

cd "$REPO"
case ",$SHELLS," in *,fish,*)
    echo "--- fish: one prompt through config.fish's gpy-init block ---"
    if command -v fish >/dev/null 2>&1; then
        fish_out="$(fish -i -c 'fish_prompt' </dev/null 2>&1 || true)"
        assert_prompt fish "$fish_out"
    else
        fail "fish is not installed on this runner"
    fi
    ;;
esac
case ",$SHELLS," in *,zsh,*)
    echo "--- zsh: one prompt from the archive's zsh/ tree ---"
    if command -v zsh >/dev/null 2>&1; then
        mkdir -p "$XDG_CONFIG_HOME/gpy"
        cp -R "$PKG/zsh" "$XDG_CONFIG_HOME/gpy/zsh"
        mkdir -p "$XDG_CONFIG_HOME/gpy/zsh/completions"
        "$CLI_BIN" completions zsh >"$XDG_CONFIG_HOME/gpy/zsh/completions/_gpy" 2>/dev/null || true
        printf 'source "%s/gpy/zsh/gpy.zsh"\n' "$XDG_CONFIG_HOME" >"$HOME/.zshrc"
        zsh_out="$(zsh -i -c '__gpy_precmd; print -r -- "$PROMPT"' </dev/null 2>&1 || true)"
        assert_prompt zsh "$zsh_out"
    else
        fail "zsh is not installed on this runner"
    fi
    ;;
esac
case ",$SHELLS," in *,bash,*)
    echo "--- bash: one prompt from the archive's bash/ tree ---"
    mkdir -p "$XDG_CONFIG_HOME/gpy"
    cp -R "$PKG/bash" "$XDG_CONFIG_HOME/gpy/bash"
    mkdir -p "$XDG_CONFIG_HOME/gpy/bash/completions"
    "$CLI_BIN" completions bash >"$XDG_CONFIG_HOME/gpy/bash/completions/gpy.bash" 2>/dev/null || true
    printf 'source "%s/gpy/bash/gpy.bash"\n' "$XDG_CONFIG_HOME" >"$HOME/.bashrc"
    bash_out="$(bash -i -c '__gpy_precmd; printf "%s" "$PS1"' </dev/null 2>&1 || true)"
    assert_prompt bash "$bash_out"
    ;;
esac
cd "$SANDBOX"

if (( failures > 0 )); then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the packaged binaries install, start and render a prompt in every shell"
