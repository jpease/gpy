#!/usr/bin/env zsh
# tests/zsh/path_parity.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh's path resolution agrees with the binary's (#647 row 6).
#
# `gpy debug paths --format kv` is the agent's own resolver; Zsh resolves
# the same paths in zsh/core/ipc.zsh (`__gpy_debug_paths`). The two must
# agree for every key Zsh owns, across the environment shapes users actually
# have (all XDG_* set; XDG_RUNTIME_DIR unset; every XDG_* unset), and every
# key Zsh does not own must say `<unimplemented>` -- a declared gap, not an
# accident. tests/fish/path_parity.test.fish runs the full 24-case matrix for
# all three shells; this file is the Zsh suite's own guard so a
# zsh/core/ipc.zsh edit fails here first.
#
# The instant-cache key encoding is checked the same way, against reality:
# the file the agent writes for a repository must be exactly
# `<shell key for that repo>.git*.zsh`, so the hand-copied vectors the
# suite used to carry can no longer drift from the Rust encoder unnoticed.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

GPY_BIN="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$GPY_BIN" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
fi

# The file each case sources before calling __gpy_debug_paths. Cases whose HOME
# is empty resolve the passwd home, so they source only the core resolvers:
# gpy.zsh would also read that real home's config and may start an agent.
PARITY_SRC=zsh/gpy.zsh

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# Keys Zsh resolves itself; every other key must be <unimplemented>.
OWNED="runtime_root socket shell_registry_dir cache_root instant_prompts_dir"
ALL_KEYS="runtime_root socket shell_registry_dir cache_root instant_prompts_dir theme_export_file config_path config_candidates theme_dir"

# `compare CASE ENV...` runs both resolvers under `env -i ENV...`.
compare() {
    case_name="$1"
    shift
    agent="$(env -i PATH="$PATH" "$@" "$GPY_BIN" debug paths --format kv)"
    shell="$(env -i PATH="$PATH" "$@" zsh -c 'source "$1/$2"; __gpy_debug_paths' _ "$ROOT" "$PARITY_SRC")"
    for key in ${=ALL_KEYS}; do
        a="$(printf '%s\n' "$agent" | sed -n "s/^$key=//p")"
        s="$(printf '%s\n' "$shell" | sed -n "s/^$key=//p")"
        case " $OWNED " in
            *" $key "*)
                if [ "$a" = "$s" ] && [ -n "$a" ]; then
                    pass "$case_name: $key = $a"
                else
                    fail "$case_name: $key differs -- agent '$a', zsh '$s'"
                fi
                ;;
            *)
                if [ "$s" = "<unimplemented>" ]; then
                    pass "$case_name: $key is a declared gap (<unimplemented>)"
                else
                    fail "$case_name: $key is not owned by zsh but resolved '$s'; declare it in OWNED and in tests/fish/path_parity.test.fish's GPY_PP_OWNERS"
                fi
                ;;
        esac
    done
}

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-path-parity.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
# The theme export sets the agent and supervisor flags from config.toml after
# gpy.zsh loads (#762), so disable the agent in every config dir the cases
# resolve to (the supervisor flag alone only stops restarts, #842); otherwise
# sourcing gpy.zsh starts an agent.
for cfg in "$tmp/config" "$tmp/home/.config"; do
    mkdir -p "$cfg/gpy"
    printf '[agent]\nenabled = false\n\n[agent.supervisor]\nenabled = false\n' >"$cfg/gpy/config.toml"
done
compare xdg_all_set HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run"
compare runtime_unset HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache"
compare xdg_unset HOME="$tmp/home"
# #774: Windows-shaped XDG_* values are relative on Unix; every resolver ignores them.
compare xdg_windows_shaped HOME="$tmp/home" XDG_CONFIG_HOME='\\srv\cfg' XDG_CACHE_HOME='C:/x' XDG_RUNTIME_DIR='C:\x'
# GPY_AGENT_SOCKET_PATH overrides only the socket, never the runtime root.
compare socket_override HOME="$tmp/home" XDG_RUNTIME_DIR="$tmp/run" GPY_AGENT_SOCKET_PATH="$tmp/custom.sock"
# #845: an empty HOME is "unset" -- the agent, Fish and Bash take the passwd
# home, and Zsh must not build a filesystem-root `/.cache/gpy` path instead.
PARITY_SRC=zsh/core/ipc.zsh
compare home_empty HOME= XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run"
compare home_empty_cache_unset HOME= XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run"
compare home_unset_cache_unset XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run"
PARITY_SRC=zsh/gpy.zsh

# --- cache key encoding, against a file the agent wrote ----------------------------
echo "--- cache key encoding ---"
shell_e2e_init "$ROOT"
# zsh keeps the script-level EXIT trap above when shell_e2e_init (a function)
# sets its own, so the harness agent would outlive the test: re-arm one trap
# that stops it and removes $tmp (#836).
trap 'shell_e2e_cleanup; rm -rf "$tmp"' EXIT
printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n\n[agent.supervisor]\nenabled = false\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/zsh/gpy.zsh"
# A git render through the agent makes it write the instant cache for this repo.
__gpy_request git "$PWD" ansi true "" true >/dev/null
resolved="$(realpath "$SHELL_E2E_REPO" 2>/dev/null || echo "$SHELL_E2E_REPO")"
key="$(__gpy_path_to_cache_key "$resolved")"
written() { ls "$XDG_CACHE_HOME/gpy/instant-prompts/$key".git*.zsh >/dev/null 2>&1; }
if shell_e2e_poll 5 written; then
    pass "the agent's cache file name starts with zsh's key for the repo ($key)"
else
    fail "no cache file named from zsh's key '$key'; agent wrote: $(ls "$XDG_CACHE_HOME/gpy/instant-prompts" 2>/dev/null | head -n 3)"
fi
# #705: a repo whose path holds the Windows-reserved characters ? and | must still
# resolve to the file the agent wrote (the agent escapes only the five documented tokens).
odd_repo="$SHELL_E2E_ROOT/what?repo|x"
mkdir -p "$odd_repo"
git -C "$odd_repo" init -q -b main
git -C "$odd_repo" config user.email test@example.com
git -C "$odd_repo" config user.name Test
git -C "$odd_repo" config core.fsmonitor false
git -C "$odd_repo" config commit.gpgsign false
git -C "$odd_repo" config core.hooksPath /dev/null
echo hello >"$odd_repo/tracked.txt"
git -C "$odd_repo" add tracked.txt
git -C "$odd_repo" commit -qm init
__gpy_request git "$odd_repo" ansi true "" true >/dev/null
odd_resolved="$(realpath "$odd_repo")"
odd_key="$(__gpy_path_to_cache_key "$odd_resolved")"
odd_written() { ls "$XDG_CACHE_HOME/gpy/instant-prompts/$odd_key".git*.zsh >/dev/null 2>&1; }
if shell_e2e_poll 5 odd_written; then
    pass "the agent's cache file name matches zsh's key for a repo with ? and | in its path ($odd_key)"
else
    fail "no cache file named from zsh's key '$odd_key'; agent wrote: $(ls "$XDG_CACHE_HOME/gpy/instant-prompts" 2>/dev/null | head -n 5)"
fi
# #771: a repo under a ~300-byte path has a key too long for one filename;
# the agent stores it chunked and the reader must find it.
long_repo="$SHELL_E2E_ROOT/long/${(l:80::a:)}/${(l:80::b:)}/${(l:80::c:)}"
mkdir -p "$long_repo"
git -C "$long_repo" init -q -b main
git -C "$long_repo" config user.email test@example.com
git -C "$long_repo" config user.name Test
git -C "$long_repo" config core.fsmonitor false
git -C "$long_repo" config commit.gpgsign false
git -C "$long_repo" config core.hooksPath /dev/null
echo hello >"$long_repo/tracked.txt"
git -C "$long_repo" add tracked.txt
git -C "$long_repo" commit -qm init
__gpy_request git "$long_repo" ansi true "" true >/dev/null
long_cached() {
    __gpy_read_instant_cache git "$long_repo" >/dev/null
    [ "$?" -ne 1 ]
}
if shell_e2e_poll 5 long_cached; then
    pass "zsh reads the agent's cache for a ${#long_repo}-byte repo path"
else
    fail "no instant cache readable for the ${#long_repo}-byte repo path $long_repo"
fi
# The encoder's escape rules, exercised on characters the repo path lacks.
odd="$tmp/under_score dir:with colons"
case "$(__gpy_path_to_cache_key "$odd")" in
    *"under__score_wdir_cwith_wcolons") pass "the encoder escapes _ / : and space injectively" ;;
    *) fail "unexpected encoding for '$odd': $(__gpy_path_to_cache_key "$odd")" ;;
esac

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh path resolution and cache keys agree with the binary"
