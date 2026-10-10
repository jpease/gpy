#!/usr/bin/env bash
# scripts/wsl-smoke.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Smoke run for GPY under Windows Subsystem for Linux (#849).
#
# Run it INSIDE a WSL distribution, from a checkout whose debug agent is built
# (`cd gpy-agent && cargo build --locked`). It is what `.github/workflows/
# wsl-smoke.yml` runs on a Windows runner, and what to run by hand on a real
# WSL machine to record a manual smoke run. It asserts the behaviour the WSL
# section of docs/INSTALL.md documents, and prints (never asserts) the facts a
# reader needs to judge a `/mnt/<drive>` repository:
#
#   identity   the kernel and WSL_DISTRO_NAME say this is WSL, and the agent
#              agrees (`gpy-agent init` reports the fonts as undetectable, the
#              WSL answer, instead of scanning the Linux font directories)
#   git        `gpy-agent oneshot git` renders a repository on the Linux
#              filesystem and one on a Windows drive (drvfs/9p)
#   mounts     the filesystem type of each, and of $HOME (info only)
#   contract   the cross-shell contract suite (Fish, Bash and Zsh against a
#              real agent), with --contract; it needs fish, zsh, python3 and
#              socat
#
# Usage:  scripts/wsl-smoke.sh [--contract]

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT="${GPY_AGENT_BIN:-$ROOT/gpy-agent/target/debug/gpy-agent}"
RUN_CONTRACT=0
[[ "${1:-}" == "--contract" ]] && RUN_CONTRACT=1

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }
info() { echo "INFO: $*"; }

SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

[[ -x "$AGENT" ]] || { echo "wsl-smoke: no agent at $AGENT (build it first)" >&2; exit 2; }

# --- identity ---------------------------------------------------------------
release="$(cat /proc/sys/kernel/osrelease 2>/dev/null || true)"
info "kernel: $release"
info "WSL_DISTRO_NAME: ${WSL_DISTRO_NAME:-<unset>}"
info "WSL_INTEROP: ${WSL_INTEROP:-<unset>}"
for opener in xdg-open wslview; do
    info "$opener: $(command -v "$opener" || echo '<not installed>')"
done
if [[ "${release,,}" == *microsoft* || "${release,,}" == *wsl* || -n "${WSL_DISTRO_NAME:-}" ]]; then
    pass "this is a WSL distribution"
else
    fail "this does not look like WSL (kernel '$release'); run it inside a WSL distribution"
fi

# --- fonts: the Linux font directories say nothing about a Windows terminal --
font_home="$SANDBOX/font-home"
mkdir -p "$font_home/.config"
# A Nerd Font file in a Linux font directory must NOT make the agent claim glyphs
# work: the terminal is a Windows application that cannot see it.
mkdir -p "$font_home/.local/share/fonts"
: >"$font_home/.local/share/fonts/FiraCodeNerdFont-Regular.ttf"
init_out="$(env -u GPY_NERD_FONT HOME="$font_home" XDG_CONFIG_HOME="$font_home/.config" \
    XDG_DATA_HOME="$font_home/.local/share" "$AGENT" init --non-interactive 2>&1)"
if grep -q "your fonts could not be detected" <<<"$init_out"; then
    pass "init reports the fonts as undetectable under WSL"
else
    fail "init did not report the fonts as undetectable: $init_out"
fi
if grep -q 'show_icons = false' "$font_home/.config/gpy/config.toml" 2>/dev/null; then
    pass "init chose ASCII icons"
else
    fail "init did not write show_icons = false"
fi
override_out="$(GPY_NERD_FONT=nerd HOME="$font_home" XDG_CONFIG_HOME="$font_home/.config" \
    "$AGENT" init --non-interactive --force 2>&1)"
if grep -q 'show_icons = true' "$font_home/.config/gpy/config.toml" 2>/dev/null; then
    pass "GPY_NERD_FONT=nerd overrides the WSL answer"
else
    fail "GPY_NERD_FONT=nerd did not set show_icons = true: $override_out"
fi

# --- git: the Linux filesystem and a Windows drive ---------------------------
make_repo() {
    local dir="$1"
    mkdir -p "$dir" && git -C "$dir" init -q -b main \
        && git -C "$dir" -c user.name=smoke -c user.email=smoke@example.com \
            commit -q --allow-empty -m init
}

check_oneshot() {
    local label="$1" dir="$2" out
    if ! make_repo "$dir"; then
        fail "$label: could not create a repository at $dir"
        return
    fi
    info "$label: $dir on $(findmnt -n -o FSTYPE -T "$dir" 2>/dev/null || echo '?')"
    out="$("$AGENT" oneshot git --cwd "$dir" 2>&1)"
    if grep -q '"branch"' <<<"$out" && grep -q 'main' <<<"$out"; then
        pass "$label: oneshot git reports the branch"
    else
        fail "$label: oneshot git did not report the branch: $out"
    fi
}

info "HOME: $HOME on $(findmnt -n -o FSTYPE -T "$HOME" 2>/dev/null || echo '?')"
check_oneshot "Linux filesystem" "$SANDBOX/linux-repo"

drvfs_root=""
# GPY_WSL_DRVFS_DIR names a writable directory on a Windows drive (the workflow
# passes the Windows checkout); otherwise the checkout itself, then C: and D:.
for candidate in "${GPY_WSL_DRVFS_DIR:-}" "$ROOT" /mnt/c /mnt/d; do
    case "$candidate" in /mnt/[a-z]|/mnt/[a-z]/*)
        if [[ -w "$candidate" ]]; then drvfs_root="$candidate"; break; fi ;;
    esac
done
if [[ -n "$drvfs_root" ]]; then
    drvfs_dir="$(mktemp -d -p "$drvfs_root" gpy-wsl-smoke.XXXXXX)"
    trap 'rm -rf "$SANDBOX" "$drvfs_dir"' EXIT
    check_oneshot "Windows drive" "$drvfs_dir/repo"
else
    info "no writable /mnt/<drive> found; the Windows-drive repository check was not run"
fi

# --- the cross-shell contract suite -------------------------------------------
if (( RUN_CONTRACT )); then
    if (( EUID == 0 )); then
        # As root the shells draw the prompt character themselves, so the
        # contract rows that count character requests cannot pass.
        fail "the contract suite must run as an ordinary user, not root"
    fi
    if CI="${CI:-1}" bash "$ROOT/tests/bash/shell_contract.test.bash"; then
        pass "cross-shell contract suite"
    else
        fail "cross-shell contract suite"
    fi
fi

if (( failures )); then
    echo "=== WSL smoke FAILED: $failures check(s) ==="
    exit 1
fi
echo "=== WSL smoke passed ==="
