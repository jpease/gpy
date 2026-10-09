#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
# tests/bash/supervisor_flag_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #842: ONE contract for `[agent.supervisor] enabled`, run through Bash, Zsh
# and Fish with the same environment:
#
#   * GPY_AGENT_ENABLED=1, GPY_AGENT_SUPERVISOR_ENABLED=0
#       -> the shell still starts the agent once at startup (Bash/Zsh at
#          source time, Fish at the first prompt); the flag controls
#          mid-session restarts only, so a later health check starts nothing.
#   * GPY_AGENT_ENABLED=0 (supervisor on or off)
#       -> no shell starts anything.
#   * GPY_AGENT_ENABLED=1, GPY_AGENT_SUPERVISOR_ENABLED=1
#       -> baseline: every shell starts the agent at startup.
#
# This replaces the per-shell tests that pinned opposite behaviour. A stub
# `gpy-agent` on PATH logs every `start`; its socket never exists, so the
# agent always looks dead. Shells that are not installed are skipped (a
# failure under CI, see test_skip in tests/lib/shell_e2e.sh).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

FAILED=0
check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/bin" "$TMP_DIR/cache" "$TMP_DIR/work" "$TMP_DIR/home/.config"
CALL_LOG="$TMP_DIR/calls"

cat >"$TMP_DIR/bin/gpy-agent" <<STUB
#!/usr/bin/env bash
[[ "\$1" == "start" ]] && printf 'start\n' >>"$CALL_LOG"
exit 1
STUB
chmod +x "$TMP_DIR/bin/gpy-agent"

# Number of `gpy-agent start` calls after waiting for a backgrounded start to
# land (bounded poll: up to ~1 s, or until the expected count is reached).
starts() {
    local expected="$1" i=0 n
    while ((i < 10)); do
        n="$(wc -l <"$CALL_LOG" 2>/dev/null | tr -d ' ')"
        [[ "$n" -ge "$expected" ]] && break
        sleep 0.1
        i=$((i + 1))
    done
    # Let a wrongly-started extra call land before counting.
    sleep 0.2
    wc -l <"$CALL_LOG" | tr -d ' '
}

# run_shell SHELL AGENT_ENABLED SUPERVISOR_ENABLED
# Starts the shell's integration, runs one prompt cycle plus one forced
# health check, and leaves the number of start calls in $CALL_LOG.
run_shell() {
    local shell="$1" agent="$2" supervisor="$3"
    : >"$CALL_LOG"
    local -a env_args=(
        "PATH=$TMP_DIR/bin:$PATH"
        # A sandbox HOME/XDG so Fish's detached supervisor loop can never find
        # (and run) a real install.
        "HOME=$TMP_DIR/home"
        "XDG_CONFIG_HOME=$TMP_DIR/home/.config"
        "GPY_AGENT_ENABLED=$agent"
        "GPY_AGENT_SUPERVISOR_ENABLED=$supervisor"
        "GPY_AGENT_SOCKET_PATH=$TMP_DIR/dead.sock"
        "XDG_CACHE_HOME=$TMP_DIR/cache"
        "GPY_AGENT_START_DELAY_MS=0"
        "GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS=0"
    )
    case "$shell" in
        bash)
            (cd "$TMP_DIR/work" && env "${env_args[@]}" bash -c '
                source "$1/bash/gpy.bash"
                __enabled_segments="directory"
                __gpy_precmd
                __gpy_supervisor_last_check_time=0
                __gpy_supervisor_check
            ' _ "$ROOT" >/dev/null 2>&1)
            ;;
        zsh)
            (cd "$TMP_DIR/work" && env "${env_args[@]}" zsh -c '
                source "$1/zsh/gpy.zsh"
                __enabled_segments=(directory)
                __gpy_precmd
                __gpy_supervisor_last_check_time=0
                (( $+functions[__gpy_supervisor_check] )) && __gpy_supervisor_check
            ' _ "$ROOT" >/dev/null 2>&1)
            ;;
        fish)
            (cd "$TMP_DIR/work" && env "${env_args[@]}" fish --no-config -c '
                source "$argv[1]/fish/core/init.fish"
                source "$argv[1]/fish/core/ipc.fish"
                emit fish_prompt
                emit fish_prompt
            ' "$ROOT" >/dev/null 2>&1)
            ;;
    esac
}

# "AGENT SUPERVISOR EXPECTED_STARTS label". Exactly 1 start in the supervisor-
# off case proves the startup start ran and the later health check did not.
# With the supervisor on the count is only bounded below (a restart may add more).
scenarios=(
    "1 1 1+ supervisor on: agent started at startup"
    "1 0 1 supervisor off: agent still started at startup, no restart"
    "0 1 0 agent off, supervisor on: nothing started"
    "0 0 0 agent off, supervisor off: nothing started"
)

for shell in bash zsh fish; do
    if ! command -v "$shell" >/dev/null 2>&1; then
        echo "SKIP: $shell not installed"
        [[ -n "${CI:-}" ]] && FAILED=1
        continue
    fi
    echo "--- $shell ---"
    for scenario in "${scenarios[@]}"; do
        read -r agent supervisor expected label <<<"$scenario"
        run_shell "$shell" "$agent" "$supervisor"
        got="$(starts "${expected%+}")"
        if [[ "$expected" == *+ ]]; then
            [[ "$got" -ge "${expected%+}" ]] && got="$expected"
        fi
        check "$shell, $label" "$expected" "$got"
    done
done

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== supervisor flag contract passed ==="
