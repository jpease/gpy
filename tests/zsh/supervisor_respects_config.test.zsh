#!/usr/bin/env zsh
# tests/zsh/supervisor_respects_config.test.zsh
#
# Regression test for #762: the zsh per-prompt supervisor check must honour
# the [agent.supervisor] values the theme export sets after zsh/gpy.zsh loads
# (enabled, check_interval_seconds, max_restart_attempts).
#
# A stub gpy-agent stands in for the agent: it logs argv, always fails
# `status`, and prints a theme export whose supervisor flag comes from
# STUB_SUPERVISOR_ENABLED.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-sup-config.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/home" "$tmp/cfg" "$tmp/cache" "$tmp/run"

cat >"$tmp/bin/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >> "$tmp/calls.log"
case "\$1 \$2" in
  "theme export")
    echo 'export GPY_AGENT_ENABLED="1"'
    echo "export GPY_AGENT_SUPERVISOR_ENABLED=\"\$STUB_SUPERVISOR_ENABLED\""
    ;;
  status*) exit 1 ;;
esac
exit 0
EOF
chmod +x "$tmp/bin/gpy-agent"

failed=0

# Runs zsh/gpy.zsh in a clean zsh with the stub first on PATH, truncates the
# call log, then evaluates $2 (calls to __gpy_supervisor_check). Paths and the
# body reach the child as data (env and $1), never interpolated into -c code.
# $1: STUB_SUPERVISOR_ENABLED; $3..: extra VAR=value environment.
run_case() {
    local sup=$1 body=$2
    shift 2
    : >"$tmp/calls.log"
    env -i PATH="$tmp/bin:/usr/bin:/bin" HOME="$tmp/home" TERM=dumb \
        XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
        XDG_RUNTIME_DIR="$tmp/run" TMPDIR="$tmp" CALLS_LOG="$tmp/calls.log" \
        STUB_SUPERVISOR_ENABLED="$sup" ROOT="$ROOT" "$@" \
        zsh -f -c '__case_body=$1; shift
            source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1
            : >"$CALLS_LOG"
            eval "$__case_body"
            sleep 0.5' zsh "$body" >/dev/null 2>&1
}

count_calls() {
    grep -c "^$1" "$tmp/calls.log" 2>/dev/null
}

# Case 1: the export disables the supervisor after gpy.zsh loaded.
run_case 0 '__gpy_supervisor_check; __gpy_supervisor_check; __gpy_supervisor_check' \
    GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS=0
starts=$(count_calls start)
if [[ "$starts" == 0 ]]; then
    echo "PASS: [agent.supervisor] enabled = false stops the per-prompt check"
else
    echo "FAIL: per-prompt check ran 'gpy-agent start' $starts time(s) with the supervisor disabled"
    failed=1
fi

# Case 2: check_interval_seconds rate-limits the health probe. The second
# check is made to look 15 s after the first: past the old 10 s default,
# inside the configured 60 s.
run_case 1 '__gpy_supervisor_check
    (( __gpy_supervisor_last_check_time -= 15 ))
    __gpy_supervisor_check' \
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60
starts=$(count_calls start)
if [[ "$starts" == 1 ]]; then
    echo "PASS: check_interval_seconds=60 allows one check per window"
else
    echo "FAIL: two checks 15 s apart with a 60 s interval started the agent $starts time(s), want 1"
    failed=1
fi

# Case 3: max_restart_attempts caps restarts.
run_case 1 '__gpy_supervisor_check; __gpy_supervisor_check; __gpy_supervisor_check' \
    GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS=0 GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1
starts=$(count_calls start)
if [[ "$starts" == 1 ]]; then
    echo "PASS: max_restart_attempts=1 allows one restart"
else
    echo "FAIL: max_restart_attempts=1 allowed $starts restart(s), want 1"
    failed=1
fi

exit $failed
