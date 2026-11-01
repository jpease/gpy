#!/usr/bin/env bash
# tests/repo/bench_compare_starship.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# benchmarks/compare-starship: the pinned inputs are well formed, the verdict
# rule in summarize.jq is the documented one for both hyperfine export schemas,
# and the machine-contention checks in run.sh accept a quiet block and reject a
# busy one. Nothing here runs a benchmark or touches the network.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command jq

DIR="$ROOT/benchmarks/compare-starship"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT


names=""
bad=0
while read -r name url sha ref _; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    names="$names $name"
    [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { fail "repos.lock: $name commit is not a full SHA ($sha)"; bad=1; }
    [[ "$url" == https://github.com/*.git ]] || { fail "repos.lock: $name url is not an https GitHub clone url ($url)"; bad=1; }
    [[ -n "$ref" ]] || { fail "repos.lock: $name has no ref"; bad=1; }
done <"$DIR/repos.lock"
dupes=$(printf '%s\n' "$names" | tr ' ' '\n' | sed '/^$/d' | sort | uniq -d)
[[ -z "$dupes" ]] || { fail "repos.lock: duplicate names: $dupes"; bad=1; }
[[ -n "$names" ]] || { fail "repos.lock has no entries"; bad=1; }
((bad)) || pass "repos.lock: every fixture pinned to a full commit SHA"

for key in GPY_PINNED STARSHIP_PINNED STARSHIP_SHA256_X86_64_UNKNOWN_LINUX_MUSL STARSHIP_SHA256_AARCH64_APPLE_DARWIN; do
    if [[ -n "$(sed -n "s/^$key=//p" "$DIR/versions.lock")" ]]; then
        pass "versions.lock sets $key"
    else
        fail "versions.lock is missing $key"
    fi
done

for key in STARSHIP_SHA256_X86_64_UNKNOWN_LINUX_MUSL STARSHIP_SHA256_AARCH64_APPLE_DARWIN; do
    if [[ "$(sed -n "s/^$key=//p" "$DIR/versions.lock")" =~ ^[0-9a-f]{64}$ ]]; then
        pass "versions.lock: $key is a SHA-256 digest"
    else
        fail "versions.lock: $key is not 64 lowercase hex characters"
    fi
done

# The triples .github/actions/install-shells builds map, by upcasing and
# turning '-' into '_', to the digest keys above.
for triple in aarch64-apple-darwin x86_64-unknown-linux-musl; do
    key="STARSHIP_SHA256_$(printf '%s' "$triple" | tr '[:lower:]-' '[:upper:]_')"
    if grep -q "^$key=" "$DIR/versions.lock"; then
        pass "versions.lock has the digest key $key for $triple"
    else
        fail "versions.lock has no $key for the $triple tarball install-shells downloads"
    fi
done


# doc SCHEMA GPY_MS STARSHIP_MS: one hyperfine export holding both tools.
doc() {
    jq -n -c --arg schema "$1" --argjson g "$2" --argjson s "$3" '
        def runs($ms): [range(0; 10) | ($ms + (. % 3 - 1) * 0.1) / 1000];
        def res($name; $ms):
          if $schema == "v1" then {name: $name, times: runs($ms)}
          else {name: $name, measurements: [runs($ms)[] | {time_wall_clock: {value: ., unit: "second"}}]}
          end;
        {results: [res("gpy"; $g), res("starship"; $s)]}'
}

# summarize "doc doc ..." FIELD: one field of summarize.jq's output.
summarize() {
    local docs=$1 field=$2
    # shellcheck disable=SC2086  # the docs are single-line JSON, separated by spaces
    printf '%s\n' $docs | jq -s --argjson min_rounds 5 -f "$DIR/summarize.jq" | jq -r "$field"
}

for schema in v1 v2; do
    d1=$(doc "$schema" 10 12)
    d2=$(doc "$schema" 12 10)
    six_faster="$d1 $d1 $d1 $d1 $d1 $d1"
    expect_eq "$schema: GPY lower in all 6 rounds is a GPY verdict" "$(summarize "$six_faster" .verdict)" gpy
    expect_eq "$schema: ratio is GPY median / Starship median" "$(summarize "$six_faster" '.ratio * 1000 | round')" 833
    expect_eq "$schema: Starship lower in all 6 rounds is a Starship verdict" "$(summarize "$d2 $d2 $d2 $d2 $d2 $d2" .verdict)" starship
    expect_eq "$schema: one reversed round makes the verdict inconclusive" "$(summarize "$d1 $d1 $d1 $d1 $d1 $d2" .verdict)" inconclusive
    expect_eq "$schema: reversed rounds are counted" "$(summarize "$d1 $d1 $d1 $d1 $d1 $d2" .rounds_gpy_faster)" 5
    expect_eq "$schema: fewer than 5 rounds never gives a verdict" "$(summarize "$d1 $d1 $d1 $d1" .verdict)" inconclusive
done

expect_eq "pooled sample count is rounds x runs" "$(summarize "$(doc v2 10 12) $(doc v2 10 12)" .gpy.n)" 20
expect_eq "a 20% round-to-round drift is flagged noisy" "$(summarize "$(doc v2 10 12) $(doc v2 12 12) $(doc v2 10 12) $(doc v2 10 12) $(doc v2 10 12)" .noisy)" true
expect_eq "stable rounds are not flagged noisy" "$(summarize "$(doc v2 10 12) $(doc v2 10 12) $(doc v2 10 12) $(doc v2 10 12) $(doc v2 10 12)" .noisy)" false


# One CPU spike at t=102. Sourcing run.sh defines functions only.
printf '100 5.00\n101 7.00\n102 90.00\n103 6.00\n' >"$TMP/samples"
got=$(
    # shellcheck source=benchmarks/compare-starship/run.sh
    . "$DIR/run.sh"
    SAMPLES="$TMP/samples"
    window_busy 101 103
)
expect_eq "window_busy covers exactly the requested seconds" "$got" "34.33 90.00 3"
got=$(
    # shellcheck source=benchmarks/compare-starship/run.sh
    . "$DIR/run.sh"
    SAMPLES="$TMP/samples"
    window_busy 100 101
)
expect_eq "window_busy excludes samples outside the window" "$got" "6.00 7.00 2"

reason() { # MEAN MAX SAMPLES SWAP0 SWAP1 THERMAL: mean limit 25, peak limit 45
    (
        # shellcheck source=benchmarks/compare-starship/run.sh
        . "$DIR/run.sh"
        contention_reason "$@" 25.0 45.0
    )
}
expect_eq "a quiet block has no contention reason" "$(reason 8 12 10 5 5 '')" ""
flagged() { # LABEL MEAN MAX SAMPLES SWAP0 SWAP1 THERMAL: must have a reason
    if [[ -n "$(reason "${@:2}")" ]]; then pass "$1"; else fail "$1: no contention reason given"; fi
}
flagged "a mean above the limit is contention" 40 50 10 5 5 ''
flagged "a single spike far above the limit is contention" 10 60 10 5 5 ''
expect_eq "a modest spike inside limit + 20 is tolerated" "$(reason 10 40 10 5 5 '')" ""
flagged "swap growth is contention" 10 12 10 5 200 ''
flagged "thermal throttling is contention" 10 12 10 5 5 80
expect_eq "an unthrottled CPU (limit 100) is not contention" "$(reason 10 12 10 5 5 100)" ""
expect_eq "CPU is not judged from fewer than 2 samples" "$(reason 99 99 1 5 5 '')" ""

# own_busy: the tools' own CPU, as "MEAN PEAK" percent of all CPUs, from either
own() { # FILE
    (
        # shellcheck source=benchmarks/compare-starship/run.sh
        . "$DIR/run.sh"
        ncpu() { echo 10; }
        WARMUP=0
        own_busy "$1"
    )
}
cat >"$TMP/own-v2.json" <<'JSON'
{"results":[
 {"name":"gpy","summary":{"time_cpu":{"count":100,"mean":0.6},"time_wall_clock":{"mean":0.1}}},
 {"name":"starship","summary":{"time_cpu":{"count":100,"mean":0.2},"time_wall_clock":{"mean":0.1}}}]}
JSON
expect_eq "own_busy reads hyperfine v2 (mean, busiest tool)" "$(own "$TMP/own-v2.json")" "40 60"
jq -n '{results: [
  {command: "gpy", mean: 0.1, user: 0.1, system: 0.5, times: [range(100) | 0.1]},
  {command: "starship", mean: 0.1, user: 0.1, system: 0.1, times: [range(100) | 0.1]}]}' >"$TMP/own-v1.json"
expect_eq "own_busy reads hyperfine v1 (mean, busiest tool)" "$(own "$TMP/own-v1.json")" "40 60"

test_finish "compare-starship inputs, verdict rule and contention checks"
