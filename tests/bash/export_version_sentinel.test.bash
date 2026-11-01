#!/usr/bin/env bash
# tests/bash/export_version_sentinel.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A cached theme export whose first line is not the version sentinel (the
# 0.8-style file an older agent wrote) is never sourced: its
# `export GPY_AGENT_ENABLED="0"` would read as user input. The shell runs the
# (stubbed) `gpy-agent theme export` instead, and GPY_AGENT_ENABLED stays unset.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP="$(mktemp -d "${TMPDIR:-/tmp}/gpy-sentinel.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/home" "$TMP/cache/gpy" "$TMP/config" "$TMP/run"
CALLS="$TMP/calls.log"

# The stub prints a current-format export for the shell in $2 (`--shell X`),
# and logs every call.
cat >"$TMP/bin/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >>"$CALLS"
if [ "\$1 \$2" = "theme export" ]; then
    case "\$4" in
        fish)
            echo 'set -g __gpy_export_version "1"'
            echo 'set -g __gpy_stub_export_ran "1"'
            ;;
        zsh)
            echo 'typeset -g __gpy_export_version="1"'
            echo 'typeset -g __gpy_stub_export_ran="1"'
            ;;
        *)
            echo '__gpy_export_version="1"'
            echo '__gpy_stub_export_ran="1"'
            ;;
    esac
fi
exit 0
EOF
chmod +x "$TMP/bin/gpy-agent"

# The 0.8-style files: no sentinel, an exported GPY_AGENT_ENABLED.
printf '%s\n' '# GPY Theme: old' 'export GPY_AGENT_ENABLED="0"' >"$TMP/cache/gpy/theme-export.bash"
printf '%s\n' '# GPY Theme: old' 'export GPY_AGENT_ENABLED="0"' >"$TMP/cache/gpy/theme-export.zsh"
printf '%s\n' '# GPY Theme: old' 'set -gx GPY_AGENT_ENABLED "0"' >"$TMP/cache/gpy/theme-export.fish"

# run SHELL: the shell loads GPY from this checkout with only the stub agent
# on PATH; prints `enabled=<GPY_AGENT_ENABLED or unset> ran=<stub marker>`.
run() {
    : >"$CALLS"
    set -- "$1" env -i PATH="$TMP/bin:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin" HOME="$TMP/home" TERM=dumb \
        XDG_CONFIG_HOME="$TMP/config" XDG_CACHE_HOME="$TMP/cache" XDG_RUNTIME_DIR="$TMP/run" TMPDIR="$TMP" \
        GPY_AGENT_SOCKET_PATH="$TMP/run/missing.sock" GPY_AGENT_SUPERVISOR_ENABLED=0 ROOT="$ROOT"
    local sh="$1"
    shift
    case "$sh" in
        bash)
            # shellcheck disable=SC2016 # expands in the child
            "$@" bash --noprofile --norc -c 'source "$ROOT/bash/gpy.bash" >/dev/null 2>&1; echo "enabled=${GPY_AGENT_ENABLED-unset} ran=${__gpy_stub_export_ran-no}"'
            ;;
        zsh)
            # shellcheck disable=SC2016 # expands in the child
            "$@" zsh -f -c 'source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1; print -r -- "enabled=${GPY_AGENT_ENABLED-unset} ran=${__gpy_stub_export_ran-no}"'
            ;;
        fish)
            # shellcheck disable=SC2016 # expands in the child
            "$@" fish --no-config -i -c 'source $ROOT/fish/core/init.fish >/dev/null 2>&1; echo "enabled="(set -q GPY_AGENT_ENABLED; and echo $GPY_AGENT_ENABLED; or echo unset)" ran="(set -q __gpy_stub_export_ran; and echo $__gpy_stub_export_ran; or echo no)' </dev/null
            ;;
    esac
}

for sh in bash zsh fish; do
    if ! command -v "$sh" >/dev/null 2>&1; then
        test_skip_part "$sh is not installed"
        continue
    fi
    out="$(run "$sh" 2>/dev/null | tail -n 1)"
    check "$sh: the sentinel-less cache is not sourced; GPY_AGENT_ENABLED stays unset" "enabled=unset ran=1" "$out"
    exports="$(grep -c '^theme export' "$CALLS")"
    check "$sh: gpy-agent theme export ran once instead" "1" "$exports"
done

test_finish "a theme export without the version sentinel is never sourced"
