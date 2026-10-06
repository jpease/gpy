#!/usr/bin/env bash
# tests/bash/startup_respects_config.test.bash
#
# Regression test for #837: bash's startup `gpy-agent start` must honour the
# GPY_AGENT_ENABLED / GPY_AGENT_SUPERVISOR_ENABLED flags the theme export
# sets from config.toml, so gpy.bash may start the agent only after the
# export has loaded (the zsh twin is #762).
#
# A stub gpy-agent stands in for the agent: it logs argv, always fails
# `status`, and prints a theme export whose flags come from STUB_AGENT and
# STUB_SUP. The environment carries no GPY_AGENT_* flags, as in a real
# shell whose settings live only in config.toml.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-startup-config.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/home" "$tmp/cfg" "$tmp/cache" "$tmp/run"

cat >"$tmp/bin/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >> "$tmp/calls.log"
case "\$1 \$2" in
  "theme export")
    echo "export GPY_AGENT_ENABLED=\"\$STUB_AGENT\""
    echo "export GPY_AGENT_SUPERVISOR_ENABLED=\"\$STUB_SUP\""
    ;;
  status*) exit 1 ;;
esac
exit 0
EOF
chmod +x "$tmp/bin/gpy-agent"

failed=0

# Sources bash/gpy.bash in a clean bash with the stub first on PATH and
# prints how many times it ran `gpy-agent start`.
# $1: STUB_AGENT, $2: STUB_SUP.
starts_for() {
    : >"$tmp/calls.log"
    # shellcheck disable=SC2016  # $ROOT expands in the child, passed as data
    env -i PATH="$tmp/bin:/usr/bin:/bin" HOME="$tmp/home" TERM=dumb \
        XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
        XDG_RUNTIME_DIR="$tmp/run" TMPDIR="$tmp" \
        GPY_AGENT_SOCKET_PATH="$tmp/run/missing.sock" \
        STUB_AGENT="$1" STUB_SUP="$2" ROOT="$ROOT" \
        bash --noprofile --norc -c 'source "$ROOT/bash/gpy.bash"' >/dev/null 2>&1
    grep -c '^start' "$tmp/calls.log"
}

check() {
    local name=$1 want=$2 got=$3
    if [[ "$got" == "$want" ]]; then
        echo "PASS: $name"
    else
        echo "FAIL: $name: want $want start call(s), got $got"
        failed=1
    fi
}

check "[agent] enabled = false skips the startup start" 0 "$(starts_for 0 1)"
check "[agent.supervisor] enabled = false skips the startup start" 0 "$(starts_for 1 0)"
# Control: both enabled still starts the missing agent, so the stub is wired.
check "both enabled starts the agent once" 1 "$(starts_for 1 1)"

exit $failed
