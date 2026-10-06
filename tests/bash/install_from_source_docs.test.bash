#!/usr/bin/env bash
# tests/bash/install_from_source_docs.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The documented from-source install, executed verbatim from a clean checkout
# (#635, pinned for #649).
#
# README.md's "From source" section told users to `git clone` and run
# `bash install.sh`, which cannot work: install.sh is the release-archive
# installer and dies on the missing, checksummed bin/ payload a checkout does
# not carry. Nothing executed the instructions, so nothing noticed. This test
# extracts `git archive HEAD` into a scratch directory, runs every fenced
# `bash` block of README.md's "From source" section and docs/INSTALL.md's
# "Install Locally" section against a sandboxed HOME/XDG_*, and asserts the
# result is an installed, runnable agent and CLI plus the shell files for
# Fish, Zsh and Bash. The commands are read from the docs at run time, so a
# doc edit that breaks the path fails here.
#
# Cost: this builds the agent in release mode, like a real user would. The
# scratch checkout's gpy-agent/target is symlinked at this repo's, so the
# dependency graph is warm and only the crate itself is rebuilt (~2 min);
# a truly cold build is what #652's clean-container job measures. Because of
# that cost the test runs under CI or GPY_GATE_RELEASE=1 (the same switch
# that gates the release-build leg in scripts/quality-check.sh) and reports
# a SKIP otherwise -- never a silent pass.
#
# Asserts:
#   (a) README's from-source block exits 0
#   (b) ~/.local/bin/gpy-agent --version and ~/.local/bin/gpy --version run
#   (c) the Fish integration is installed under $XDG_CONFIG_HOME/fish
#   (d) INSTALL.md's Zsh/Bash block exits 0 and both integrations source
#       cleanly in their shells
#   (d2) the rc lines those steps wrote sit inside `# >>> gpy-init >>>` blocks
#       and scripts/uninstall.sh leaves no gpy line behind (#809); and every
#       rc append in docs/INSTALL.md is marker-wrapped
#   (e) nothing was written outside the sandbox HOME

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if [[ -z "${CI:-}" && -z "${GPY_GATE_RELEASE:-}" ]]; then
    test_skip "from-source install builds a release binary (~2 min); set GPY_GATE_RELEASE=1 to run it locally (CI always runs it)"
fi
test_require_command git
test_require_command cargo "cargo not installed; the documented from-source path needs Rust"
test_require_command fish "fish not installed; README's from-source path uses install-dev.fish"
test_require_command zsh
test_require_command bash

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# --- extract the documented commands ----------------------------------------

# Print the bodies of every ```bash fence between the heading `$2` and the
# next heading of the same or higher level in file `$1`.
doc_bash_blocks() {
    local file="$1" heading="$2"
    awk -v heading="$heading" '
        BEGIN { level = 0; inside = 0; fence = 0 }
        {
            if (!inside) {
                if ($0 == heading) {
                    inside = 1
                    match($0, /^#+/)
                    level = RLENGTH
                }
                next
            }
            if (fence) {
                if ($0 == "```") { fence = 0; print ""; next }
                print
                next
            }
            # A `# comment` inside a fence is handled above; only bare text
            # lines can be headings.
            if ($0 ~ /^#+ /) {
                match($0, /^#+/)
                if (RLENGTH <= level) { exit }
            }
            if ($0 == "```bash") { fence = 1; next }
        }
    ' "$file"
}

readme_block="$(doc_bash_blocks "$ROOT/README.md" "### From source")"
install_block="$(doc_bash_blocks "$ROOT/docs/INSTALL.md" "### Install Locally")"

[[ -n "$readme_block" ]] || fail "README.md has no bash block under '### From source'"
[[ -n "$install_block" ]] || fail "docs/INSTALL.md has no bash block under '### Install Locally'"
if [[ $failures -gt 0 ]]; then
    exit 1
fi

# Every documented rc append (Fish config.fish, Zsh, Bash) must sit in a
# fenced snippet that also writes the gpy-init markers, or scripts/uninstall.*
# leave a dangling `source` line behind (#809).
unmarked_appends="$(awk '
    /^```/ {
        if (infence) { if (append && !marked) printf "%s", snippet; infence = 0 }
        else { infence = 1; append = 0; marked = 0; snippet = "" }
        next
    }
    infence {
        snippet = snippet FNR ": " $0 "\n"
        if ($0 ~ />>[ ]*[^ ]*(\.zshrc|\.bashrc|config\.fish)/) append = 1
        if (index($0, "# >>> gpy-init >>>")) marked = 1
    }
' "$ROOT/docs/INSTALL.md")"
if [[ -n "$unmarked_appends" ]]; then
    fail "docs/INSTALL.md appends to an rc file without the gpy-init markers:
$unmarked_appends"
fi

# The README block clones from GitHub; a test must not touch the network, so
# the clone line is replaced by the archive extraction below and everything
# after `cd gpy` runs as written.
if ! grep -q '^git clone https://github.com/jpease/gpy.git$' <<<"$readme_block"; then
    fail "README from-source block no longer starts with the expected git clone line:
$readme_block"
fi
readme_commands="$(grep -v '^git clone ' <<<"$readme_block" | grep -v '^cd gpy$')"

# --- sandbox -------------------------------------------------------------------

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-from-source.XXXXXX")"
SRC="$SANDBOX/gpy"
SANDBOX_HOME="$SANDBOX/home"
mkdir -p "$SRC" "$SANDBOX_HOME" "$SANDBOX/runtime" "$SANDBOX/tmp"
chmod 700 "$SANDBOX/runtime"

# The sandboxed HOME hides the developer's Rust toolchain (rustup and mise
# both resolve through $HOME), so hand the real toolchain to the sandbox:
# the resolved cargo directory goes first on PATH, ahead of any version-
# manager shim that would need $HOME to work, and CARGO_HOME/RUSTUP_HOME keep
# pointing at the real ones.
real_cargo="$(rustup which cargo 2>/dev/null || command -v cargo)"
real_cargo_dir="$(dirname "$real_cargo")"
real_home="$HOME"

# Resolve every tool the documented commands launch by name BEFORE sealing,
# so the sandbox PATH can be built from these directories alone (#749).
tool_dirs=""
for tool in fish zsh bash git rsync; do
    tool_path="$(command -v "$tool" 2>/dev/null)" || continue
    tool_dir="$(dirname "$tool_path")"
    case ":$tool_dirs:" in
        *":$tool_dir:"*) ;;
        *) tool_dirs="$tool_dirs:$tool_dir" ;;
    esac
done
sandbox_path="$SANDBOX_HOME/.local/bin:$real_cargo_dir$tool_dirs:/usr/bin:/bin:/usr/sbin:/sbin"

# Every sandboxed command runs under `env -i "${sandbox_env[@]}"`: nothing
# from the caller (fish_user_paths, GPY_*, XDG_DATA_DIRS, PATH) leaks in.
sandbox_env=(
    "HOME=$SANDBOX_HOME"
    "XDG_CONFIG_HOME=$SANDBOX_HOME/.config"
    "XDG_CACHE_HOME=$SANDBOX_HOME/.cache"
    "XDG_RUNTIME_DIR=$SANDBOX/runtime"
    "TMPDIR=$SANDBOX/tmp"
    "PATH=$sandbox_path"
    "CARGO_HOME=${CARGO_HOME:-$real_home/.cargo}"
    "RUSTUP_HOME=${RUSTUP_HOME:-$real_home/.rustup}"
    "GPY_NERD_FONT=none"
    "RUSTC_WRAPPER="
    "TERM=${TERM:-dumb}"
    "LANG=${LANG:-C.UTF-8}"
)

# Stop exactly the agent this test started: ask through the sandbox's own
# socket (sandbox env, not the caller's), then TERM/KILL whatever still holds
# that socket. Never a name-based kill (#484).
stop_sandbox_agent() {
    local sock="$SANDBOX/runtime/gpy/gpy.sock" agent="$SANDBOX_HOME/.local/bin/gpy-agent" pid i
    if [[ -x "$agent" ]]; then
        env -i "${sandbox_env[@]}" "$agent" stop >/dev/null 2>&1 || true
    fi
    command -v lsof >/dev/null 2>&1 || return 0
    for ((i = 0; i < 30; i++)); do
        [[ -n "$(lsof -t "$sock" 2>/dev/null)" ]] || return 0
        sleep 0.1
    done
    for pid in $(lsof -t "$sock" 2>/dev/null); do kill -TERM "$pid" 2>/dev/null || true; done
    for ((i = 0; i < 30; i++)); do
        [[ -n "$(lsof -t "$sock" 2>/dev/null)" ]] || return 0
        sleep 0.1
    done
    for pid in $(lsof -t "$sock" 2>/dev/null); do kill -KILL "$pid" 2>/dev/null || true; done
}

cleanup() {
    # install-dev.fish starts the freshly installed agent to verify hot
    # reload; stop it before the sandbox goes away.
    stop_sandbox_agent
    rm -rf "$SANDBOX"
}
trap cleanup EXIT

# Tripwire: any gpy/gpy-agent the caller's PATH resolves outside the sandbox
# must be byte-for-byte the same file afterwards (#693 deleted real ones).
caller_binaries_snapshot() {
    local name found
    for name in gpy gpy-agent; do
        while IFS= read -r found; do
            case "$found" in "$SANDBOX"/*) continue ;; esac
            ls -li "$found" 2>&1
        done < <(type -aP "$name" 2>/dev/null)
    done
}
caller_binaries_before="$(caller_binaries_snapshot)"

git -C "$ROOT" archive --format=tar HEAD | tar -x -C "$SRC"
# Warm dependency cache (see the header); the crate itself still builds.
mkdir -p "$ROOT/gpy-agent/target"
ln -s "$ROOT/gpy-agent/target" "$SRC/gpy-agent/target"

run_documented() {
    local label="$1" commands="$2"
    echo "--- running documented commands: $label ---"
    local log="$SANDBOX/$label.log"
    if (cd "$SRC" && env -i "${sandbox_env[@]}" bash -e -c "$commands") >"$log" 2>&1; then
        return 0
    fi
    fail "$label commands failed; last 40 lines:"
    tail -n 40 "$log"
    return 1
}

# --- (a)-(c) README: fish install-dev.fish ----------------------------------------

run_documented "readme-from-source" "$readme_commands" || true

for bin in gpy-agent gpy; do
    if version="$(env "${sandbox_env[@]}" "$SANDBOX_HOME/.local/bin/$bin" --version 2>&1)"; then
        echo "  $bin: $version"
    else
        fail "$bin is not installed and runnable at ~/.local/bin ($version)"
    fi
done

fish_dir="$SANDBOX_HOME/.config/fish"
[[ -f "$fish_dir/gpy/core/init.fish" ]] || fail "Fish core files missing at $fish_dir/gpy/core"
[[ -f "$fish_dir/conf.d/gpy_init.fish" ]] || fail "Fish conf.d entry missing"
[[ -e "$fish_dir/functions/fish_prompt.fish" ]] || fail "Fish prompt function missing"
[[ -f "$fish_dir/completions/gpy.fish" ]] || fail "Fish completions were not generated"

# --- (d) INSTALL.md: Zsh and Bash ---------------------------------------------------

run_documented "install-md-install-locally" "$install_block" || true

[[ -f "$SANDBOX_HOME/.config/gpy/zsh/gpy.zsh" ]] || fail "Zsh integration not copied"
[[ -f "$SANDBOX_HOME/.config/gpy/bash/gpy.bash" ]] || fail "Bash integration not copied"
grep -q 'source ~/.config/gpy/zsh/gpy.zsh' "$SANDBOX_HOME/.zshrc" 2>/dev/null || fail ".zshrc was not updated"
grep -q 'source ~/.config/gpy/bash/gpy.bash' "$SANDBOX_HOME/.bashrc" 2>/dev/null || fail ".bashrc was not updated"

# Each integration must source without error in its shell; the agent is kept
# off so sourcing never spawns a daemon from inside this test. The theme
# export re-sets the supervisor flag from config.toml (#836), so these two
# runs also read a supervisor-off config through an empty cache of their own.
mkdir -p "$SANDBOX/supervisor-off"
(source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX/supervisor-off")
supervisor_off_env=("XDG_CONFIG_HOME=$SANDBOX/supervisor-off/config" "XDG_CACHE_HOME=$SANDBOX/supervisor-off/cache")
if ! env -i "${sandbox_env[@]}" "${supervisor_off_env[@]}" GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    zsh -c 'source ~/.config/gpy/zsh/gpy.zsh && whence -w __gpy_debug_paths >/dev/null' >"$SANDBOX/zsh-source.log" 2>&1; then
    fail "sourcing the installed Zsh integration failed: $(cat "$SANDBOX/zsh-source.log")"
fi
if ! env -i "${sandbox_env[@]}" "${supervisor_off_env[@]}" GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
    bash -c 'source ~/.config/gpy/bash/gpy.bash && declare -F __gpy_debug_paths >/dev/null' >"$SANDBOX/bash-source.log" 2>&1; then
    fail "sourcing the installed Bash integration failed: $(cat "$SANDBOX/bash-source.log")"
fi

# --- (d2) the documented rc lines are marker-wrapped and uninstallable (#809) ---

for rc in .zshrc .bashrc; do
    grep -qF '# >>> gpy-init >>>' "$SANDBOX_HOME/$rc" 2>/dev/null || fail "$rc line is not wrapped in the gpy-init markers"
    grep -qF '# <<< gpy-init <<<' "$SANDBOX_HOME/$rc" 2>/dev/null || fail "$rc block has no closing gpy-init marker"
done
for shell_name in zsh bash; do
    if ! printf '\n' | env -i "${sandbox_env[@]}" GPY_SHELL="$shell_name" sh "$SRC/scripts/uninstall.sh" >"$SANDBOX/uninstall-$shell_name.log" 2>&1; then
        fail "scripts/uninstall.sh (GPY_SHELL=$shell_name) failed: $(cat "$SANDBOX/uninstall-$shell_name.log")"
    fi
done
for rc in .zshrc .bashrc; do
    if [[ -f "$SANDBOX_HOME/$rc" ]] && grep -q gpy "$SANDBOX_HOME/$rc"; then
        fail "uninstall left a gpy line in $rc: $(grep gpy "$SANDBOX_HOME/$rc")"
    fi
done

# --- (e) nothing escaped the sandbox ---------------------------------------------------

if [[ -n "$(git -C "$ROOT" status --porcelain -- bin 2>/dev/null)" ]]; then
    fail "the from-source install wrote into this checkout's bin/"
fi

if [[ "$(caller_binaries_snapshot)" != "$caller_binaries_before" ]]; then
    fail "the from-source install changed or removed a gpy/gpy-agent outside the sandbox (before/after):
$caller_binaries_before
--
$(caller_binaries_snapshot)"
fi

if [[ $failures -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: documented from-source install works from a clean checkout"
