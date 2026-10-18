#!/usr/bin/env bash
# shellcheck disable=SC2016 # driver scripts are single-quoted for the shell under test
# tests/bash/e2e_lang_long_cache_key.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# An encoded cache key over 200 characters is stored by the agent under
# 50-character chunk directories. In Bash, Zsh and Fish the language segment's
# omit gate (the instant-cache presence probe) must find those entries like the
# reader does: from a cold cache, the shell's own refresh fills the cache and a
# later render draws the segment instead of omitting it forever. A project
# keyed under 200 characters is the control. Once the agent is down, the cached
# entry is still drawn (stale, through the context-free fallback).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
for shell in zsh fish; do
    test_require_command "$shell"
done

cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
printf '[ui]\nenabled_segments = ["language"]\n\n[agent.supervisor]\nenabled = false\n\n[language]\nshow_versions = false\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"

short_project="$SHELL_E2E_ROOT/short"
long_project="$SHELL_E2E_ROOT/$(printf 'a%.0s' {1..75})/$(printf 'b%.0s' {1..75})/$(printf 'c%.0s' {1..50})/proj"
for project in "$short_project" "$long_project"; do
    mkdir -p "$project/src"
    printf '[package]\nname = "gpy-e2e-test"\nversion = "0.1.0"\n' >"$project/Cargo.toml"
    printf 'fn main() {}\n' >"$project/src/main.rs"
done

# The language cache is keyed by the canonical cwd; the agent stores keys over
# 200 characters chunked, so the long project must actually cross the boundary.
key_length() {
    local key
    key="$(cd "$1" && pwd -P)"
    key="${key//_/__}"
    key="${key//\//_s}"
    printf '%s' "${#key}"
}
[[ "$(key_length "$short_project")" -le 200 ]] || fail "short project key is over 200 characters"
[[ "$(key_length "$long_project")" -gt 200 ]] || fail "long project key is not over 200 characters"

# Each driver runs in $PROJ: up to $TRIES times it asks the omit gate and, once
# the segment is not omitted, prints `RENDER:<segment>`; otherwise `OMITTED`.
# In `cold` mode an omitted render waits 100 ms for the refresh it dispatched.
# The render carries a previous background, so it reads through the variant
# fallback.
drive_bash='
cd "$PROJ" || exit 1
source "$GPY_ROOT/bash/gpy.bash" >/dev/null 2>&1
export GPY_AGENT_ENABLED=1
for ((i = 0; i < TRIES; i++)); do
    if ! __gpy_segment_language_omit; then
        printf "RENDER:%s" "$(__gpy_segment_language true 231 "")"
        exit 0
    fi
    [[ "$MODE" == cold ]] || break
    sleep 0.1
done
printf OMITTED
'
drive_zsh='
cd "$PROJ" || exit 1
source "$GPY_ROOT/zsh/gpy.zsh" >/dev/null 2>&1
export GPY_AGENT_ENABLED=1
for ((i = 0; i < TRIES; i++)); do
    if ! __gpy_segment_language_omit; then
        printf "RENDER:%s" "$(__gpy_segment_language true 231 "")"
        exit 0
    fi
    [[ "$MODE" == cold ]] || break
    sleep 0.1
done
printf OMITTED
'
drive_fish='
cd $PROJ; or exit 1
source $GPY_ROOT/fish/core/init.fish >/dev/null 2>&1
source $GPY_ROOT/fish/segments/language.fish
set -gx GPY_AGENT_ENABLED 1
for i in (seq $TRIES)
    if not segment_language_omit
        set -g __gpy_last_segment_bg 231
        printf "RENDER:%s" (segment_language_render true "")
        exit 0
    end
    test "$MODE" = cold; or break
    sleep 0.1
end
printf OMITTED
'

# drive SHELL MODE PROJECT [PATH]: the shell binary is resolved before PATH
# is narrowed.
drive() {
    local shell="$1" mode="$2" project="$3" path="${4:-$PATH}" script shell_bin
    case "$shell" in
        bash) script="$drive_bash" ;;
        zsh) script="$drive_zsh" ;;
        fish) script="$drive_fish" ;;
    esac
    shell_bin="$(command -v "$shell")"
    PATH="$path" GPY_ROOT="$ROOT" PROJ="$project" MODE="$mode" \
        TRIES=$((80 * $(shell_e2e_timeout_scale))) \
        "$shell_bin" -c "$script" 2>/dev/null
}

check_render() {
    local label="$1" out="$2"
    if [[ "$out" == RENDER:?* ]]; then
        pass "$label"
    else
        fail "$label: got [$out]"
    fi
}

shell_e2e_start_agent || exit 1

for shell in bash zsh fish; do
    rm -rf "$cache_dir"
    for project in "$short_project" "$long_project"; do
        label="$shell, $([[ "$project" == "$short_project" ]] && echo short || echo long) key"
        check_render "$label: a cold cache is filled by the shell's refresh and drawn" \
            "$(drive "$shell" cold "$project")"
    done
done

if find "$cache_dir" -mindepth 2 -name '*.lang*' | grep -q .; then
    pass "the agent stored the long key's entries under chunk directories"
else
    fail "no chunked language cache entry under $cache_dir"
fi

shell_e2e_stop_agent
for shell in bash zsh fish; do
    for project in "$short_project" "$long_project"; do
        label="$shell, $([[ "$project" == "$short_project" ]] && echo short || echo long) key"
        check_render "$label: with the agent down the stale entry is still drawn" \
            "$(GPY_LANGUAGE_CACHE_TTL_SECONDS=0 drive "$shell" down "$project" /usr/bin:/bin:/usr/sbin:/sbin)"
    done
done

test_finish "the language segment renders agent-written caches on both sides of the 200-character key boundary"
