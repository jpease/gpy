#!/usr/bin/env zsh
# tests/zsh/e2e_multi_session_live_repaint.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Two idle Zsh sessions in one repository repaint on outside changes.
#
# One real agent, two `zsh -i` clients on ptys: A in the repo root, B in a
# subdirectory. With no keystroke sent to either, each outside change below
# must reach BOTH sessions' latest render within 15 s:
#   1. `printf x >> tracked.txt`     -> the dirty marker ✱1 appears (timed)
#   2. `touch new_untracked`         -> the untracked marker ?1 appears (timed)
#   3. `git commit -qam m`           -> ✱1 is gone (correctness only, untimed)
#   4. `git checkout -q -b n1-latency` -> the new branch name appears (timed)
#   5. config.toml rewritten to the default theme with icons, no
#      `gpy-agent config reload` (the watcher path) -> a powerline glyph
#      U+E0B0..E0BF appears in the raw transcript (timed)
# Then the agent reports two registered clients and both show the same git
# segment.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
mkdir -p "$SHELL_E2E_REPO/sub"
: >"$SHELL_E2E_REPO/sub/.keep"
git -C "$SHELL_E2E_REPO" add sub/.keep
git -C "$SHELL_E2E_REPO" commit -qm sub
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1

shell_e2e_use_session A
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
shell_e2e_use_session B
cd "$SHELL_E2E_REPO/sub" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
cd "$SHELL_E2E_ROOT" || exit 1

dump_both() {
    for s in A B; do
        echo "=== session $s ==="
        shell_e2e_use_session "$s"
        shell_e2e_dump_transcript
    done
}

for s in A B; do
    shell_e2e_use_session "$s"
    shell_e2e_wait_for 'main' 10 >/dev/null || { fail "session $s: no branch prompt within 10 s"; dump_both; exit 1; }
done

# Repaints redraw on the same line, so split the stripped transcript of
# session $1 (from byte $2) on the prompt character and keep the most recent
# render that contains the fixed string $3.
latest_render() {
    shell_e2e_use_session "$1"
    shell_e2e_transcript "$2" | sed 's/❯/\n/g' | grep -F "$3" | tail -n 1
}

off_A=0
off_B=0
stamp=0

# Capture the stamp and both sessions' transcript offsets before a change.
begin_step() {
    stamp="$(python3 "$SHELL_E2E_PTY" now)"
    shell_e2e_use_session A
    off_A="$(shell_e2e_size)"
    shell_e2e_use_session B
    off_B="$(shell_e2e_size)"
}

# True when both sessions' latest render after the step contains $1.
both_show() {
    [ -n "$(latest_render A "$off_A" "$1")" ] && [ -n "$(latest_render B "$off_B" "$1")" ]
}

# True when both raw transcripts after the step match the regex $1.
both_match_raw() {
    python3 "$SHELL_E2E_PTY" first-match-time "$SHELL_E2E_ROOT/session-A" "$1" "$off_A" >/dev/null 2>&1 &&
        python3 "$SHELL_E2E_PTY" first-match-time "$SHELL_E2E_ROOT/session-B" "$1" "$off_B" >/dev/null 2>&1
}

# Record each session's latency for step $1: the time from the stamp to the
# chunk that completed the first match of the raw regex $2.
record_latency() {
    for s in A B; do
        if [ "$s" = A ]; then o="$off_A"; else o="$off_B"; fi
        t="$(python3 "$SHELL_E2E_PTY" first-match-time "$SHELL_E2E_ROOT/session-$s" "$2" "$o")" || {
            fail "session $s: no timestamp for the $1 repaint"
            continue
        }
        ms=$(( (t - stamp) / 1000000 ))
        printf 'latency %s %s: %s ms\n' "$1" "$s" "$ms"
    done
}

# --- 1. dirty --------------------------------------------------------------
begin_step
printf x >>"$SHELL_E2E_REPO/tracked.txt"
if shell_e2e_poll 15 both_show '✱1'; then
    pass "both idle sessions show the dirty marker"
    record_latency dirty '✱1'
else
    fail "the dirty marker did not reach both sessions within 15 s"
fi

# --- 2. untracked ----------------------------------------------------------
begin_step
touch "$SHELL_E2E_REPO/new_untracked"
if shell_e2e_poll 15 both_show '?1'; then
    pass "both idle sessions show the untracked marker"
    record_latency untracked '(?<!\[)\?1'
else
    fail "the untracked marker did not reach both sessions within 15 s"
fi

# --- 3. commit (correctness only) -------------------------------------------
begin_step
git -C "$SHELL_E2E_REPO" commit -qam m
clean_render() {
    r="$(latest_render "$1" "$2" main)"
    [ -n "$r" ] && ! printf '%s' "$r" | grep -qF '✱1'
}
both_clean() { clean_render A "$off_A" && clean_render B "$off_B"; }
if shell_e2e_poll 15 both_clean; then
    pass "after the commit neither session shows the dirty marker"
else
    fail "the dirty marker did not clear in both sessions within 15 s"
fi

# --- 4. branch -------------------------------------------------------------
begin_step
git -C "$SHELL_E2E_REPO" checkout -q -b n1-latency
if shell_e2e_poll 15 both_show n1-latency; then
    pass "both idle sessions show the new branch"
    record_latency branch 'n1-latency'
else
    fail "the new branch did not reach both sessions within 15 s"
fi
branch_off_A="$off_A"
branch_off_B="$off_B"

# --- 5. theme (watcher path, no reload command) ------------------------------
begin_step
printf '[ui]\nshow_icons = true\ntheme = "default"\nenabled_segments = ["directory", "git"]\n' >"$XDG_CONFIG_HOME/gpy/config.toml"
glyph_re='\xee\x82[\xb0-\xbf]'
if shell_e2e_poll 15 both_match_raw "$glyph_re"; then
    pass "both idle sessions repainted with powerline glyphs"
    record_latency theme "$glyph_re"
else
    fail "no powerline glyph reached both sessions within 15 s"
fi

# --- registration and agreement ---------------------------------------------
two_clients() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 2'; }
if shell_e2e_poll 5 two_clients; then
    pass "the agent reports two registered clients"
else
    fail "the agent does not report two registered clients"
fi

git_segment() {
    latest_render "$1" "$2" n1-latency | sed -n 's/.*\(n1-latency.*\)$/\1/p' | sed 's/[[:space:]]*$//'
}
seg_A="$(git_segment A "$branch_off_A")"
seg_B="$(git_segment B "$branch_off_B")"
if [ -n "$seg_A" ] && [ "$seg_A" = "$seg_B" ]; then
    pass "both sessions show the same git segment ($seg_A)"
else
    fail "git segments differ: A='$seg_A' B='$seg_B'"
fi

if [ "$failures" -gt 0 ]; then
    dump_both
fi
test_finish "two idle zsh sessions repaint on outside changes"
