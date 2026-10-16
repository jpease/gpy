#!/usr/bin/env bash
# tests/bash/debug_trap_preserves_last_arg.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# gpy's DEBUG trap must not clobber `$_` (breaks `mkdir -p dir && cd $_`).

# shellcheck disable=SC2016
# Single quotes are load-bearing: the child shell expands `$_`/`$PWD`.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if (( BASH_VERSINFO[0] < 4 )); then
    test_skip "bash $BASH_VERSION installs no DEBUG trap (duration method none)"
fi

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-lastarg.XXXXXX")"
# Resolve symlinks (macOS /var -> /private/var) so it matches the child's $PWD.
T="$(cd "$T" && pwd -P)"
cleanup() { rm -rf "$T"; }
trap cleanup EXIT
# Child's config disables the supervisor; the theme export re-sets the env flag.
source "$ROOT/tests/lib/supervisor_off.bash" "$T"

# Scrub every GPY_* variable inherited from the developer's shell.
gpy_scrub=()
while IFS='=' read -r __gpy_env_name _; do
    case "$__gpy_env_name" in
        GPY_*) gpy_scrub+=(-u "$__gpy_env_name") ;;
    esac
done < <(env)

rc="$T/rc.bash"
printf 'source "%s/bash/gpy.bash"\n' "$ROOT" >"$rc"

out="$(printf '%s\n' \
    'mkdir -p "$T/a/b" && cd $_' \
    'echo "PWD=$PWD"' \
    'ls /tmp >/dev/null' \
    'echo "LAST=$_"' \
    'exit' |
    env "${gpy_scrub[@]}" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$T/missing.sock" \
        T="$T" HOME="$T" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config" \
        XDG_RUNTIME_DIR="$T/run" TMPDIR="$T" \
        PATH=/usr/bin:/bin TERM=dumb \
        "$BASH" --noprofile --rcfile "$rc" -i 2>&1)"

if [[ "$out" == *"cd: __gpy_debug_trap"* ]]; then
    fail "\`cd \$_\` expanded to __gpy_debug_trap"
fi
if [[ "$out" == *"PWD=$T/a/b"* ]]; then
    pass "mkdir -p \"\$T/a/b\" && cd \$_ changed into \$T/a/b"
else
    fail "\`mkdir -p \"\$T/a/b\" && cd \$_\` did not change into $T/a/b"
fi
if [[ "$out" == *"LAST=/tmp"* ]]; then
    pass "\$_ after \`ls /tmp\` is /tmp"
else
    fail "\$_ after \`ls /tmp\` is not /tmp"
fi

if [ "$failures" -gt 0 ]; then
    echo "--- child output ---"
    printf '%s\n' "$out"
fi
test_finish
