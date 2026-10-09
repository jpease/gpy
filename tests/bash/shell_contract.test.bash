#!/usr/bin/env bash
# shellcheck disable=SC2016 # the sed program quotes literal `$` tokens of the theme
# tests/bash/shell_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #847: the cross-shell contract harness. Fish, Bash and Zsh must BEHAVE the
# same, not merely have the same functions (tests/*/parity.test.*) -- and the
# drift fixed in the parity epic (#841-#845, #843, #844) was behaviour: an
# agent-disabled prompt that rendered nothing, a supervisor flag that gated
# the agent start, a cold git miss that forked a foreground oneshot, a slow
# reply that was recomputed or resent, a clock that was never templated.
#
# Every row of tests/fixtures/shell_scenarios/scenarios.tsv (theme, segments,
# status, env, config, cwd, agent state; the column list is in that file) is
# run through the REAL integration of each shell
# (tests/lib/shell_scenarios/render.{fish,bash,zsh}: source the entry point,
# render the prompt through the real render path, print what the shell would
# draw), each in its own sandbox, and two things are compared across the shells:
#
#   1. the prompt: normalized to "the style and text of every visible run"
#      (tests/lib/normalize_prompt.py: Fish's `set_color`, the agent's
#      `ESC[..m` and Bash/Zsh's non-printing wrappers all fold to one picture);
#   2. the side effects (tests/lib/shell_scenarios/effects.py): how often the
#      agent was started or restarted, which `gpy-agent oneshot` forks ran,
#      which IPC requests reached the agent, registrations among them. They are
#      observed from outside the shell: a recording `gpy-agent` wrapper on PATH
#      and a recording IPC proxy (tests/lib/shell_scenarios/ipc_proxy.py) in
#      front of the real agent, or in place of it for a late agent.
#
# A row's `expect` column also pins each shell to an absolute value, so three
# shells that regress together still fail. Agent states: `up` (the real agent
# behind the proxy), `down` (no socket at all), `slow` (up and registered, but a
# data request is answered only after 1.8 s, longer than any transport's budget).
#
# The raw prompt source of Bash and Zsh (before the shell expands it) is also
# checked: every escape byte must sit inside the non-printing wrapper
# (`\[ \]` / `%{ %}`), or readline/ZLE miscount the prompt's width (#679).
#
# The table is the only place a future drift fix adds a row; see
# tests/fixtures/shell_scenarios/scenarios.tsv and tests/bash/README.md.
# SCN_FILTER=<regex> runs only the scenarios whose name matches, and SCN_VERBOSE=1
# prints each shell's side effects and normalized prompt (debugging aids).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command zsh
test_require_command python3

SCN_FIXTURES="$ROOT/tests/fixtures/shell_scenarios"
SCN_LIB="$ROOT/tests/lib/shell_scenarios"
NORMALIZE="$ROOT/tests/lib/normalize_prompt.py"
SLOW_REPLY_SECONDS=1.8

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

shell_e2e_init "$ROOT"
BASE_ROOT="$SHELL_E2E_ROOT"
REAL_AGENT="$SHELL_E2E_AGENT_BIN"
proxy_pid=""
trap 'stop_proxy; shell_e2e_cleanup' EXIT

# Hermetic: the developer's GPY_* settings must not reach the shells under test.
for gpy_var in $(compgen -e); do
    [[ "$gpy_var" == GPY_* && "$gpy_var" != GPY_NERD_FONT ]] && unset "$gpy_var"
done
unset gpy_var

stop_proxy() {
    [[ -n "$proxy_pid" ]] || return 0
    kill "$proxy_pid" 2>/dev/null
    wait "$proxy_pid" 2>/dev/null
    proxy_pid=""
}

# Start the recording proxy on $1/p.sock. $2.. are ipc_proxy.py's mode flags.
start_proxy() {
    local sandbox="$1" _
    shift
    python3 "$SCN_LIB/ipc_proxy.py" "$sandbox/p.sock" "$sandbox/ipc.log" "$@" &
    proxy_pid=$!
    for _ in $(seq 1 50); do
        [[ -S "$sandbox/p.sock" ]] && return 0
        sleep 0.1
    done
    return 1
}

# Write config.toml for the segment list $1 (comma separated).
write_config() {
    local sandbox="$1" segments="$2" seg_list="" seg
    local IFS=,
    for seg in $segments; do seg_list+="\"$seg\", "; done
    unset IFS
    printf '[ui]\ntheme = "%s"\nenabled_segments = [%s]\n' "$row_theme" "${seg_list%, }" \
        >"$sandbox/config/gpy/config.toml"
    # The supervisor is off unless the row configures it itself.
    [[ "$row_config" == *"[agent.supervisor]"* ]] ||
        printf '\n[agent.supervisor]\nenabled = false\n' >>"$sandbox/config/gpy/config.toml"
    [[ "$row_config" == - ]] || printf '\n%b\n' "$row_config" >>"$sandbox/config/gpy/config.toml"
}

# A scratch git repository with one commit, hermetic against the developer's
# global git config (the same recipe as shell_e2e_init).
make_repo() {
    local repo="$1/repo"
    mkdir -p "$repo"
    git -C "$repo" init -q -b main
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name Test
    git -C "$repo" config core.fsmonitor false
    git -C "$repo" config commit.gpgsign false
    git -C "$repo" config core.hooksPath /dev/null
    echo hello >"$repo/tracked.txt"
    git -C "$repo" add tracked.txt
    git -C "$repo" commit -qm init
}

# The recording `gpy-agent` the shells find first on PATH. Starting the agent is
# recorded and NOT performed (the harness owns the agent's lifetime, and a start
# the shell asked for must not change the scenario's agent state); everything
# else is recorded and passed to the real binary.
make_wrapper() {
    mkdir -p "$1/bin"
    cat >"$1/bin/gpy-agent" <<'WRAP'
#!/bin/sh
printf '%s\n' "$*" >>"$SCN_CALLS"
case "$1" in start | restart) exit 0 ;; esac
exec "$SCN_REAL_AGENT" "$@"
WRAP
    chmod +x "$1/bin/gpy-agent"
}

# Run one shell driver with the sandbox environment; $1 sandbox, $2 shell,
# $3 stdout file, $4 stderr file.
run_driver() {
    local sandbox="$1" sh="$2" out="$3" err="$4" cwd
    local -a cmd
    case "$sh" in
        fish) cmd=(fish --no-config "$SCN_LIB/render.fish") ;;
        bash) cmd=("$BASH" "$SCN_LIB/render.bash") ;;
        zsh) cmd=(zsh "$SCN_LIB/render.zsh") ;;
    esac
    case "$row_cwd" in
        -) cwd="$ROOT" ;;
        repo) cwd="$sandbox/repo" ;;
        *) cwd="$row_cwd" ;;
    esac
    local -a scn_env=(
        "HOME=$sandbox/home" "XDG_CONFIG_HOME=$sandbox/config" "XDG_CACHE_HOME=$sandbox/cache"
        "XDG_RUNTIME_DIR=$sandbox/runtime" "GPY_AGENT_SOCKET_PATH=$sandbox/p.sock"
        "GPY_DEBUG_LOG=$sandbox/agent.log" "PATH=$sandbox/bin:$PATH"
        "SCN_CALLS=$sandbox/calls.log" "SCN_REAL_AGENT=$REAL_AGENT"
        "TERM=xterm-256color" "GPY_ROOT=$ROOT" "SCN_CWD=$cwd" "SCN_STATUS=$row_status"
        "SCN_ROOT=$([[ ",$row_flags," == *,root,* ]] && echo 1 || echo 0)"
    )
    local -a extra=()
    [[ "$row_env" != "-" ]] && read -r -a extra <<<"$row_env" && scn_env+=("${extra[@]}")
    env -u SSH_CONNECTION -u SSH_CLIENT -u SSH_TTY -u SUDO_USER \
        "${scn_env[@]}" "SCN_RAW_OUT=${SCN_RAW_OUT:-}" "${cmd[@]}" >"$out" 2>"$err"
}

# Run one scenario through one shell in its own sandbox and leave its prompt
# (<sh>.norm, <sh>.raw) and side effects (<sh>.fx) in $dir.
# $1 out dir, $2 shell, $3 sandbox label. The label stays short: the sockets
# live inside the sandbox and a unix socket path is limited to ~104 bytes.
run_shell() {
    local dir="$1" sh="$2" sandbox="$BASE_ROOT/s$3${2:0:1}" _
    rm -rf "$sandbox"
    mkdir -p "$sandbox/config/gpy/themes" "$sandbox/cache" "$sandbox/runtime" "$sandbox/home"
    cp "$SCN_FIXTURES"/themes/*.toml "$sandbox/config/gpy/themes/"
    if [[ "$row_theme" == variant-git ]]; then
        # The stock theme does not vary the git cap by prev_bg: read it from
        # the previous segment so a context-free cache entry draws wrong.
        sed 's/\[\$sep_open\](fg:\$bg bg:default))\[ \$symbol \$branch\]/[$sep_open](fg:prev_bg bg:default))[ $symbol $branch]/' \
            "$ROOT/config/themes/default.toml" >"$sandbox/config/gpy/themes/variant-git.toml"
        if cmp -s "$ROOT/config/themes/default.toml" "$sandbox/config/gpy/themes/variant-git.toml"; then
            fail "$row_name: the default theme no longer has the git opening separator this row rewrites"
            return 1
        fi
    fi
    [[ "$row_cwd" == repo ]] && make_repo "$sandbox"
    make_wrapper "$sandbox"
    : >"$sandbox/calls.log"
    : >"$sandbox/ipc.log"

    export HOME="$sandbox/home" XDG_CONFIG_HOME="$sandbox/config" XDG_CACHE_HOME="$sandbox/cache"
    export XDG_RUNTIME_DIR="$sandbox/runtime" GPY_AGENT_SOCKET_PATH="$sandbox/r.sock"
    export GPY_DEBUG_LOG="$sandbox/agent.log"
    SHELL_E2E_ROOT="$sandbox"
    write_config "$sandbox" "$row_segments"

    case "$row_agent" in
        up | slow)
            shell_e2e_start_agent || return 1
            local -a proxy_args=(--upstream "$sandbox/r.sock")
            [[ "$row_agent" == slow ]] && proxy_args+=(--slow "$SLOW_REPLY_SECONDS")
            start_proxy "$sandbox" "${proxy_args[@]}" || { fail "$row_name: proxy did not start"; return 1; }
            ;;
        down) ;;
        *) fail "$row_name: unknown agent state '$row_agent'"; return 1 ;;
    esac

    if [[ "$row_prime" != - ]]; then
        # Warm the agent and the instant cache with a different segment mix
        # first (a variant-fallback row needs the context-free entry on disk),
        # then forget what that did: only the render under test is observed.
        local saved_segments="$row_segments"
        write_config "$sandbox" "$row_prime"
        SCN_RAW_OUT="" run_driver "$sandbox" "$sh" "$sandbox/prime.out" "$sandbox/prime.err"
        # The agent writes the cache entries after it replies: wait until the
        # directory has been quiet for 0.3 s.
        local seen="" now
        for _ in $(seq 1 50); do
            now="$(find "$sandbox/cache/gpy/instant-prompts" -maxdepth 1 -name '*git*' 2>/dev/null | sort | tr '\n' ' ')"
            [[ -n "$now" && "$now" == "$seen" ]] && break
            seen="$now"
            sleep 0.3
        done
        if [[ ",$row_flags," == *,drop_variants,* ]]; then
            # Keep only the context-free `.none` entries: the agent also wrote
            # the variant for the previous segment's colour, and the row is
            # about the render that finds that one missing.
            find "$sandbox/cache/gpy/instant-prompts" -maxdepth 1 -name '*git*' ! -name '*.none.*' -delete 2>/dev/null
        fi
        # Whole-second cache ages: let the primed entries outlive a 1 s TTL.
        [[ ",$row_flags," == *,aged_cache,* ]] && sleep 2.2
        if [[ ",$row_flags," == *,down_after_prime,* ]]; then
            stop_proxy
            shell_e2e_stop_agent
        fi
        : >"$sandbox/calls.log"
        : >"$sandbox/ipc.log"
        write_config "$sandbox" "$saved_segments"
    fi

    SCN_RAW_OUT="$dir/$sh.raw" run_driver "$sandbox" "$sh" "$dir/$sh.out" "$dir/$sh.err"
    # A start the supervisor backgrounds can land after the shell has exited.
    [[ "$row_agent" == up ]] || sleep 0.4
    stop_proxy
    shell_e2e_stop_agent
    SHELL_E2E_ROOT="$BASE_ROOT"

    # The sandbox path differs per shell; the prompt must not.
    sed -i.bak "s|$sandbox|@SB@|g" "$dir/$sh.out" "$dir/$sh.err" && rm -f "$dir"/*.bak
    local -a norm_args=()
    [[ ",$row_flags," == *,mask_seconds,* ]] && norm_args=(--mask-seconds)
    python3 "$NORMALIZE" "${norm_args[@]}" <"$dir/$sh.out" >"$dir/$sh.norm"
    cp "$sandbox/calls.log" "$dir/$sh.calls"
    cp "$sandbox/ipc.log" "$dir/$sh.ipc"
    python3 "$SCN_LIB/effects.py" "$sandbox/calls.log" "$sandbox/ipc.log" >"$dir/$sh.fx"
    return 0
}

# Every escape byte of the raw prompt source must sit inside the non-printing
# wrapper. $1 label, $2 raw file, $3 open marker regex, $4 close marker regex
check_nonprinting() {
    local label="$1" raw="$2" open="$3" close="$4" printed
    [[ -s "$raw" ]] || { fail "$label: no raw prompt source captured"; return; }
    printed="$(python3 -c '
import re, sys
raw = open(sys.argv[1], encoding="utf-8", errors="replace").read()
sys.stdout.write(re.sub(sys.argv[2] + r".*?" + sys.argv[3], "", raw, flags=re.S))
' "$raw" "$open" "$close")"
    if [[ "$printed" == *$'\e'* ]]; then
        fail "$label: an escape byte sits outside the non-printing wrapper: $(printf '%q' "$printed")"
    else
        pass "$label: every escape byte is inside the non-printing wrapper"
    fi
}

# Check one `expect` token for one shell. $1 dir, $2 shell, $3 token.
#   key=value        a side-effect key from effects.py (an absent ipc.<op> is 0)
#   prompt~=TEXT     the normalized prompt contains TEXT
#   prompt!~=TEXT    the normalized prompt does not contain TEXT
check_expect() {
    local dir="$1" sh="$2" tok="$3" key value actual
    case "$tok" in
        prompt~=*)
            [[ "$(cat "$dir/$sh.norm")" == *"${tok#prompt~=}"* ]] ||
                fail "$row_name: $sh prompt lacks '${tok#prompt~=}'"
            return
            ;;
        'prompt!~='*)
            [[ "$(cat "$dir/$sh.norm")" != *"${tok#prompt!~=}"* ]] ||
                fail "$row_name: $sh prompt contains '${tok#prompt!~=}'"
            return
            ;;
    esac
    key="${tok%%=*}"
    value="${tok#*=}"
    actual="$(sed -n "s/^$key=//p" "$dir/$sh.fx")"
    [[ -z "$actual" && "$key" == ipc.* ]] && actual=0
    [[ "$actual" == "$value" ]] ||
        fail "$row_name: $sh $key is '${actual:-<absent>}', expected '$value'"
}

wait_for_stable_minute() {
    # A clock shows the minute each shell samples at its own moment; do not
    # start a scenario in the last seconds of a minute.
    local sec
    sec="$(date +%S)"
    sec=$((10#$sec))
    ((sec >= 55)) && sleep $((61 - sec))
}

scenario_count=0
while IFS=$'\t' read -r row_name row_theme row_segments row_status row_env row_flags \
    row_agent row_cwd row_config row_prime row_expect; do
    [[ -z "$row_name" || "$row_name" == \#* ]] && continue
    [[ -n "${SCN_FILTER:-}" && ! "$row_name" =~ $SCN_FILTER ]] && continue
    # Optional trailing columns default to: a live agent, the checkout as cwd,
    # no extra config, no priming render, no absolute expectations.
    row_agent="${row_agent:-up}"
    row_cwd="${row_cwd:--}"
    row_config="${row_config:--}"
    row_prime="${row_prime:--}"
    row_expect="${row_expect:--}"
    scenario_count=$((scenario_count + 1))
    has_clock=0
    [[ ",$row_segments," == *,clock,* ]] && has_clock=1

    attempt=1
    while :; do
        ((has_clock)) && wait_for_stable_minute
        out_dir="$BASE_ROOT/o$scenario_count.$attempt"
        mkdir -p "$out_dir"
        ran=1
        for sh in fish bash zsh; do
            run_shell "$out_dir" "$sh" "$scenario_count" || { ran=0; break; }
        done
        ((ran)) || break
        mismatch=""
        for sh in bash zsh; do
            if ! diff -u "$out_dir/fish.norm" "$out_dir/$sh.norm" >"$out_dir/fish-vs-$sh.diff" ||
                ! diff -u "$out_dir/fish.fx" "$out_dir/$sh.fx" >"$out_dir/fish-vs-$sh.fxdiff"; then
                mismatch="$mismatch $sh"
            fi
        done
        # A clock sampled across a minute rollover is a harness race, not a
        # rendering difference: retry. A real difference repeats and fails.
        if [[ -z "$mismatch" || $attempt -ge 3 || $has_clock -eq 0 ]]; then
            break
        fi
        attempt=$((attempt + 1))
    done
    ((ran)) || { fail "$row_name: could not run the scenario"; continue; }

    if [[ ! -s "$out_dir/fish.norm" && "$row_expect" != *'prompt!~='* ]]; then
        fail "$row_name: fish rendered nothing, so nothing was compared ($(tr -d '\n' <"$out_dir/fish.err"))"
        continue
    fi
    if [[ -z "$mismatch" ]]; then
        pass "$row_name: Fish, Bash and Zsh draw the same prompt and cause the same side effects"
    else
        for sh in $mismatch; do
            if [[ -s "$out_dir/fish-vs-$sh.diff" ]]; then
                fail "$row_name: $sh prompt differs from fish (- fish, + $sh)"
                sed 's/^/    /' "$out_dir/fish-vs-$sh.diff"
            fi
            if [[ -s "$out_dir/fish-vs-$sh.fxdiff" ]]; then
                fail "$row_name: $sh side effects differ from fish (- fish, + $sh)"
                sed 's/^/    /' "$out_dir/fish-vs-$sh.fxdiff"
            fi
            [[ -s "$out_dir/$sh.err" ]] && sed 's/^/    stderr: /' "$out_dir/$sh.err"
        done
    fi
    if [[ -n "${SCN_VERBOSE:-}" ]]; then
        for sh in fish bash zsh; do
            echo "  [$sh] $(tr '\n' ' ' <"$out_dir/$sh.fx")"
            sed "s/^/  [$sh] gpy-agent call: /" "$out_dir/$sh.calls"
            sed "s/^/  [$sh] ipc request: /" "$out_dir/$sh.ipc"
            sed "s/^/  [$sh] prompt: /" "$out_dir/$sh.norm"
        done
    fi
    if [[ "$row_expect" != - ]]; then
        for sh in fish bash zsh; do
            for tok in $row_expect; do
                check_expect "$out_dir" "$sh" "$tok"
            done
        done
    fi
    # Fish prints `set_color: Unknown color ...` to stderr when a color keyword
    # reaches it unresolved; a clean prompt render says nothing.
    if [[ -s "$out_dir/fish.err" ]]; then
        fail "$row_name: fish wrote to stderr: $(tr '\n' ' ' <"$out_dir/fish.err")"
    fi
    check_nonprinting "$row_name [bash PS1]" "$out_dir/bash.raw" '\\\[' '\\\]'
    check_nonprinting "$row_name [zsh PROMPT]" "$out_dir/zsh.raw" '%\{' '%\}'
done <"$SCN_FIXTURES/scenarios.tsv"

if ((scenario_count == 0)); then
    fail "no scenarios found in $SCN_FIXTURES/scenarios.tsv"
fi

if ((failures > 0)); then
    echo "=== FAILED: $failures check(s) ==="
    exit 1
fi
echo "=== cross-shell contract test passed ($scenario_count scenarios) ==="
