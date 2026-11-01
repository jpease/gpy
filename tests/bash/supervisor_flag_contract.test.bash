#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
# tests/bash/supervisor_flag_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# One `[agent.supervisor] enabled` contract across Bash, Zsh and Fish:
#   AGENT=1 SUP=0 -> one startup start; no restart on a later health check
#   AGENT=0       -> nothing starts
#   AGENT=1 SUP=1 -> every shell starts the agent at startup
# A stub `gpy-agent` logs every `start`; its socket never exists.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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

start_count_at_least() {
    [[ "$(wc -l <"$CALL_LOG" 2>/dev/null | tr -d ' ')" -ge "$1" ]]
}

# Count `gpy-agent start` calls after a bounded wait for a backgrounded start.
starts() {
    shell_e2e_poll 1 start_count_at_least "$1" || true
    # Let a wrongly-started extra call land before counting.
    sleep 0.2
    wc -l <"$CALL_LOG" | tr -d ' '
}

# run_shell SHELL AGENT_ENABLED SUPERVISOR_ENABLED: one prompt cycle plus a forced
# health check; leaves the start-call count in $CALL_LOG.
run_shell() {
    local shell="$1" agent="$2" supervisor="$3"
    : >"$CALL_LOG"
    local -a env_args=(
        "PATH=$TMP_DIR/bin:$PATH"
        # A sandbox HOME/XDG so a shell can never find (and run) a real install.
        "HOME=$TMP_DIR/home"
        "XDG_CONFIG_HOME=$TMP_DIR/home/.config"
        "GPY_AGENT_ENABLED=$agent"
        "GPY_AGENT_SUPERVISOR_ENABLED=$supervisor"
        "GPY_AGENT_SOCKET_PATH=$TMP_DIR/dead.sock"
        "XDG_CACHE_HOME=$TMP_DIR/cache"
        "GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0"
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

# Exactly 1 start with the supervisor off proves the health check started nothing;
# with it on the count is only bounded below.
scenarios=(
    "1 1 1+ supervisor on: agent started at startup"
    "1 0 1 supervisor off: agent still started at startup, no restart"
    "0 1 0 agent off, supervisor on: nothing started"
    "0 0 0 agent off, supervisor off: nothing started"
)

for shell in bash zsh fish; do
    if ! command -v "$shell" >/dev/null 2>&1; then
        test_skip_part "$shell not installed"
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

test_finish "supervisor flag contract"
