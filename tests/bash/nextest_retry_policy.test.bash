#!/usr/bin/env bash
# tests/bash/nextest_retry_policy.test.bash
#
# gpy-agent#620: profile.default in gpy-agent/.config/nextest.toml used to
# retry the ENTIRE suite up to twice (`retries = 2`), which hid first-attempt
# regressions outside the small set of tests actually known to be flaky and
# made a green run weaker evidence for outside contributors. Locks in the
# fix so it can't silently regress back to a blanket retry:
#
#   (a) profile.default carries no unscoped retry budget: either it has no
#       `retries` key at all, or it is explicitly `retries = 0`.
#   (b) every retained per-test override that sets `retries` names the exact
#       test(s) it applies to (a `filter` containing `test(...)` and/or
#       `binary_id(...)`, not a blanket filter) and documents an issue number
#       (`#NNN`) in the comment block immediately above the override -- so a
#       future override can't reintroduce an undocumented blanket retry under
#       the cover of looking like a narrow one.
#   (c) `status-level`/`final-status-level` are configured on profile.default
#       so a retry prints during the run and is listed in the final summary
#       instead of a retried test looking identical to one that never needed
#       a retry (scripts/quality-check.sh's `report_nextest_retries` depends
#       on this to grep for the FLAKY line it emits).
#
# Point `NEXTEST_TOML_PATH` at another file to check it instead of the repo's
# real config -- used by this project's own TDD history for this test (see
# the issue) to show it RED against the pre-#620 config without needing a
# second copy of this script.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

CONFIG="${NEXTEST_TOML_PATH:-gpy-agent/.config/nextest.toml}"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

if [[ ! -f "$CONFIG" ]]; then
    fail "$CONFIG not found"
    echo "$failures assertion(s) failed"
    exit 1
fi

mapfile -t LINES <"$CONFIG"
line_count=${#LINES[@]}

# --- Locate the [profile.default] table -------------------------------
default_start=-1
for ((i = 0; i < line_count; i++)); do
    if [[ "${LINES[$i]}" =~ ^\[profile\.default\]$ ]]; then
        default_start=$i
        break
    fi
done

if ((default_start < 0)); then
    fail "$CONFIG has no [profile.default] table"
    echo "$failures assertion(s) failed"
    exit 1
fi

default_end=$((default_start + 1))
while ((default_end < line_count)) && [[ ! "${LINES[$default_end]}" =~ ^\[ ]]; do
    default_end=$((default_end + 1))
done

default_retries=""
has_status_level=0
has_final_status_level=0
for ((i = default_start; i < default_end; i++)); do
    l="${LINES[$i]}"
    if [[ "$l" =~ ^retries[[:space:]]*=[[:space:]]*(.+)$ ]]; then
        default_retries="${BASH_REMATCH[1]}"
    fi
    if [[ "$l" =~ ^status-level[[:space:]]*=[[:space:]]*\".+\" ]]; then
        has_status_level=1
    fi
    if [[ "$l" =~ ^final-status-level[[:space:]]*=[[:space:]]*\".+\" ]]; then
        has_final_status_level=1
    fi
done

# (a) no unscoped retry budget on profile.default.
echo "--- profile.default does not retry the whole suite ---"
if [[ -n "$default_retries" && "$default_retries" != "0" ]]; then
    fail "[profile.default] sets retries = $default_retries; a suite-wide retry masks first-attempt regressions (gpy-agent#620). Remove the key or set it to 0."
fi

# (c) retry visibility is configured.
echo "--- retry visibility is configured ---"
if ((!has_status_level)); then
    fail "[profile.default] does not set status-level -- a retried test's TRY 2+ attempt would not print (gpy-agent#620)"
fi
if ((!has_final_status_level)); then
    fail "[profile.default] does not set final-status-level -- a retried test would not be listed in the run's final summary (gpy-agent#620)"
fi

# --- Locate every [[profile.default.overrides]] block ------------------
echo "--- every retained retry override names its tests and an issue ---"
override_count=0
for ((i = 0; i < line_count; i++)); do
    [[ "${LINES[$i]}" =~ ^\[\[profile\.default\.overrides\]\]$ ]] || continue
    override_count=$((override_count + 1))
    start=$i

    end=$((start + 1))
    while ((end < line_count)) && [[ ! "${LINES[$end]}" =~ ^\[ ]]; do
        end=$((end + 1))
    done

    override_retries=""
    filter_line=""
    for ((j = start; j < end; j++)); do
        l="${LINES[$j]}"
        if [[ "$l" =~ ^retries[[:space:]]*=[[:space:]]*(.+)$ ]]; then
            override_retries="${BASH_REMATCH[1]}"
        fi
        if [[ "$l" =~ ^filter[[:space:]]*= ]]; then
            filter_line="$l"
        fi
    done

    # Overrides without a `retries` key aren't a retry-masking concern; skip.
    [[ -n "$override_retries" ]] || continue

    if [[ "$filter_line" != *"test("* && "$filter_line" != *"binary_id("* ]]; then
        fail "override at $CONFIG:$((start + 1)) sets retries = $override_retries but its filter does not name exact test(s) via test(...)/binary_id(...): ${filter_line:-<no filter line>}"
    fi

    # Walk upward over the contiguous run of comment lines immediately above
    # the override (stopping at the first blank or non-comment line) and
    # require an issue reference (#NNN) somewhere in it.
    k=$((start - 1))
    comment_block=""
    while ((k >= 0)) && [[ "${LINES[$k]}" =~ ^# ]]; do
        comment_block="${LINES[$k]}"$'\n'"$comment_block"
        k=$((k - 1))
    done

    if [[ ! "$comment_block" =~ \#[0-9]+ ]]; then
        fail "override at $CONFIG:$((start + 1)) sets retries = $override_retries but the comment block immediately above it names no issue (#NNN): a retained retry must link an open tracking issue (gpy-agent#620)"
    fi
done
echo "  ($override_count override block(s) checked)"

# --- (d) the Fish runner's retry list follows the same rule (#650) ----------
# scripts/test_fish.sh retries only files named in KNOWN_FLAKY_E2E_TESTS, and
# every entry must be `<file>.test.fish:#<issue>` naming a test that exists,
# so a retry there is as traceable as a nextest override. An empty list is
# the expected state.
echo "--- the Fish runner's retry list names its tests and an issue ---"
FISH_RUNNER="${FISH_RUNNER_PATH:-scripts/test_fish.sh}"
fish_list_line="$(grep -E '^KNOWN_FLAKY_E2E_TESTS=' "$FISH_RUNNER" || true)"
if [[ -z "$fish_list_line" ]]; then
    fail "$FISH_RUNNER has no KNOWN_FLAKY_E2E_TESTS= line; the retry policy must stay explicit"
else
    fish_list="${fish_list_line#KNOWN_FLAKY_E2E_TESTS=}"
    fish_list="${fish_list%\"}"
    fish_list="${fish_list#\"}"
    fish_entry_count=0
    for entry in $fish_list; do
        fish_entry_count=$((fish_entry_count + 1))
        if [[ ! "$entry" =~ ^[A-Za-z0-9_]+\.test\.fish:#[0-9]+$ ]]; then
            fail "$FISH_RUNNER retry entry '$entry' must be <file>.test.fish:#<issue>"
            continue
        fi
        if [[ ! -f "tests/fish/${entry%%:*}" ]]; then
            fail "$FISH_RUNNER retry entry '$entry' names a test that does not exist"
        fi
    done
    echo "  ($fish_entry_count Fish retry entr(y/ies) checked)"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
