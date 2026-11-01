#!/usr/bin/env bash
# shellcheck disable=SC2016 # the sed program quotes literal `$` tokens of the theme
# tests/bash/shell_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Cross-shell behaviour contract: every row of tests/fixtures/shell_scenarios/scenarios.tsv
# runs through each shell's real integration (tests/lib/shell_scenarios/render.*) in
# its own sandbox, and the shells must agree on:
#   1. the normalized prompt (tests/lib/normalize_prompt.py)
#   2. side effects (effects.py): agent starts, oneshot forks and IPC requests, seen
#      via a recording `gpy-agent` on PATH and a recording IPC proxy (ipc_proxy.py)
# A row's `expect` column also pins absolute values, so shells regressing together
# still fail. Bash/Zsh raw prompt escapes must sit inside non-printing wrappers.
# Agent states: up, down, slow (data replies after 1.8 s, past every budget).
# SCN_FILTER=<regex> selects scenarios; SCN_VERBOSE=1 prints effects and prompts.

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

proxy_ready() {
    [[ -f "$1/proxy.ready" ]] && kill -0 "$proxy_pid" 2>/dev/null
}

# Start the recording proxy on $1/p.sock. $2.. are ipc_proxy.py's mode flags.
start_proxy() {
    local sandbox="$1"
    shift
    # -S: skip site-packages so .pth warnings cannot land in proxy.err.
    python3 -S "$SCN_LIB/ipc_proxy.py" "$sandbox/p.sock" "$sandbox/ipc.log" \
        --ready-file "$sandbox/proxy.ready" "$@" >"$sandbox/proxy.out" 2>"$sandbox/proxy.err" &
    proxy_pid=$!
    shell_e2e_poll 15 proxy_ready "$sandbox" && return 0
    cat "$sandbox/proxy.err" >&2
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
    # The golden rows draw hostname and username in every session; ssh_icon
    # and the root variant still follow SSH_CONNECTION and the root flag.
    printf '\n[plugin.hostname]\nshow_always = true\n\n[plugin.username]\nshow_always = true\n' \
        >>"$sandbox/config/gpy/config.toml"
    [[ "$row_config" == - ]] || printf '\n%b\n' "$row_config" >>"$sandbox/config/gpy/config.toml"
}

# Whether every shell's theme-export cache under sandbox $1 has the segment
# list the current config.toml exports.
exports_match_config() {
    local export_dir="$1/cache/gpy" sh want have
    for sh in fish bash zsh; do
        want="$("$REAL_AGENT" theme export --shell "$sh" 2>/dev/null | grep '__enabled_segments')"
        have="$(grep '__enabled_segments' "$export_dir/theme-export.$sh" 2>/dev/null)"
        [[ -n "$want" && "$want" == "$have" ]] || return 1
    done
}

# Make a running agent apply sandbox $1's config.toml before the next render.
# Its own reload is debounced, and a render that raced it drew the previous
# segment list from the shell export. Talks to the
# agent's own socket, so the observed IPC log never sees it.
sync_agent_config() {
    local sandbox="$1"
    [[ -S "$sandbox/r.sock" ]] || return 0
    "$REAL_AGENT" config reload >/dev/null 2>&1
    shell_e2e_poll 10 exports_match_config "$sandbox" && return 0
    fail "$row_name: the agent never exported the config's segments"
    return 1
}

# A scratch git repo with one commit, hermetic against global git config.
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

# Recording `gpy-agent` first on PATH: `start` is recorded, NOT performed (the
# harness owns the agent); everything else passes through to the real binary.
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

# Run one shell driver: $1 sandbox, $2 shell, $3 stdout file, $4 stderr file.
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

# Run one scenario through one shell; leaves <sh>.norm, <sh>.raw, <sh>.fx in $1.
# $2 shell, $3 sandbox label (short: unix socket paths are capped near 104 bytes).
run_shell() {
    local dir="$1" sh="$2" sandbox="$BASE_ROOT/s$3${2:0:1}" _
    rm -rf "$sandbox"
    mkdir -p "$sandbox/config/gpy/themes" "$sandbox/cache" "$sandbox/runtime" "$sandbox/home"
    # The agent refuses a group-writable socket directory (a umask of 002 makes one).
    chmod -R go-w "$sandbox"
    cp "$SCN_FIXTURES"/themes/*.toml "$sandbox/config/gpy/themes/"
    # The bundled username/hostname plugins, where `gpy setup` puts them
    # (the default bundled root). Plugin files must not be group/world
    # writable, which a umask of 002 would leave them.
    cp -R "$ROOT/plugins" "$sandbox/config/gpy/bundled-plugins"
    chmod -R go-w "$sandbox/config/gpy/bundled-plugins"
    unset GPY_BUNDLED_PLUGIN_DIR
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
            if [[ "$row_prime" == - && "$row_cwd" == repo && ",$row_segments," == *,git,* ]]; then
                # The contract is a cold SHELL cache: prepare git status outside the observed
                # IPC log, then remove every instant entry before listening.
                proxy_args+=(--prepare-git "$sandbox/repo"
                    --instant-cache "$sandbox/cache/gpy/instant-prompts"
                    --prepare-timeout "$((10 * $(shell_e2e_timeout_scale)))")
            fi
            start_proxy "$sandbox" "${proxy_args[@]}" || { fail "$row_name: proxy did not start"; return 1; }
            sync_agent_config "$sandbox" || return 1
            ;;
        down) ;;
        *) fail "$row_name: unknown agent state '$row_agent'"; return 1 ;;
    esac

    if [[ "$row_prime" != - ]]; then
        # Prime with a different segment mix (variant-fallback rows need the context-free
        # entry on disk), then forget it: only the render under test is observed.
        local saved_segments="$row_segments"
        write_config "$sandbox" "$row_prime"
        sync_agent_config "$sandbox" || return 1
        SCN_RAW_OUT="" run_driver "$sandbox" "$sh" "$sandbox/prime.out" "$sandbox/prime.err"
        # Let the priming render's cache entries settle first.
        local seen="" now
        for _ in $(seq 1 50); do
            now="$(find "$sandbox/cache/gpy/instant-prompts" -maxdepth 1 -name '*git*' 2>/dev/null | sort | tr '\n' ' ')"
            [[ -n "$now" && "$now" == "$seen" ]] && break
            seen="$now"
            sleep 0.3
        done
        # Back to the row's own segments before the cache is doctored: a reload
        # may rewrite instant entries, which must not undo what follows.
        write_config "$sandbox" "$saved_segments"
        sync_agent_config "$sandbox" || return 1
        if [[ ",$row_flags," == *,drop_variants,* ]]; then
            # Keep only the context-free `.none` entries: the row exercises a missing
            # variant for the previous segment's colour.
            find "$sandbox/cache/gpy/instant-prompts" -maxdepth 1 -name '*git*' ! -name '*.none.*' -delete 2>/dev/null
        fi
        # Whole-second cache ages: make the primed entries older than a 1 s TTL.
        if [[ ",$row_flags," == *,aged_cache,* ]]; then
            local -a primed_entries=()
            while IFS= read -r entry; do primed_entries+=("$entry"); done \
                < <(find "$sandbox/cache/gpy/instant-prompts" -maxdepth 1 -type f 2>/dev/null)
            shell_e2e_backdate 5 "${primed_entries[@]}"
        fi
        if [[ ",$row_flags," == *,down_after_prime,* ]]; then
            stop_proxy
            shell_e2e_stop_agent
        fi
        : >"$sandbox/calls.log"
        : >"$sandbox/ipc.log"
    fi

    SCN_RAW_OUT="$dir/$sh.raw" run_driver "$sandbox" "$sh" "$dir/$sh.out" "$dir/$sh.err"
    # A backgrounded start announces itself nowhere, so wait a fixed window;
    # shell_e2e_cleanup reaps anything bound under the sandbox.
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
    # Avoid starting a clock scenario in a minute's last seconds.
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

test_finish "cross-shell contract test passed ($scenario_count scenarios)"
