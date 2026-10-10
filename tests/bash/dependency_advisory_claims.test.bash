#!/usr/bin/env bash
# tests/bash/dependency_advisory_claims.test.bash
#
# Originally locked in the dependency-advisory waiver from #503. #525 retired
# that waiver and repurposed this test to keep the (now advisory-free) three
# registers -- gpy-agent/deny.toml, gpy-agent/src/lib.rs and
# docs/dev/security-audit.md -- honest about that, and to keep the
# duplicate-crate-version table (assertions g/h) accurate going forward.
#
# History: `cargo audit` used to report five advisories against gpy-agent's
# lockfile. The repository documented four of them, in three places that
# disagreed with each other and with the lockfile:
#
#   * gpy-agent/deny.toml ignored three IDs behind one blanket comment, so
#     RUSTSEC-2021-0145 and RUSTSEC-2026-0097 passed silently by default rather
#     than by decision -- warning-class advisories never fail `cargo audit`.
#   * gpy-agent/src/lib.rs carried an "Audited 2025-11-14" block whose stated
#     resolution for every entry was that hyperpolyglot would update. 0.1.7 is
#     the highest version ever published and was released 2020-07-26, so that
#     is a plan that waits on a project with no releases in six years. The same
#     block cited crate versions the lockfile no longer contains and listed
#     crates that are no longer duplicated at all.
#   * docs/dev/security-audit.md said the same thing a third way, with a
#     "Next Review Date" eight months in the past.
#
# The recorded decision on #503: waive for the 0.1.0 launch, do not swap the
# detector yet. #523 (epic #519) swapped it anyway, to `gengo-language`'s
# tables, and removing `hyperpolyglot` removed the advisories with it --
# `cargo audit` reports zero as of 2026-09-01. #525 is the separate work #503
# said it was leaving for later: it emptied `deny.toml`'s `[advisories]
# ignore` list, deleted the reachability writeup, and left `docs/dev/
# security-audit.md`'s History section as the record of what was waived and
# why, since a waiver's retirement is itself worth being able to check.
#
# Every assertion is bound to the CLAIM, not to the sentence that carried it.
# Deleting the historical wording and writing a fresh paraphrase of the same
# false claim must still turn this test red -- including a paraphrase that
# wraps across lines, which is why the prose bans run over `claim_windows`
# rather than over raw lines.
#
# The advisory ID list is DERIVED from `cargo audit`, never hardcoded: a new
# advisory published against this lockfile turns this test red until it is
# ignored deliberately and documented (or, if it is a false positive, ignored
# with a reason the way #503 did it).
#
# Deliberately NOT asserted:
#   * SECURITY.md's `### Dependencies` -- owned by #497 and asserted by
#     tests/bash/security_trust_boundary_claims.test.bash. This test only
#     checks it does not contradict the register.
#   * The choice of language detector itself, or anything under
#     gpy-agent/src/language/ -- covered by gpy-agent/tests/language_tests.rs.
#
# Asserts:
#   (a) every advisory `cargo audit` reports is ignored in deny.toml AND
#       documented in lib.rs AND documented in docs/dev/security-audit.md, no
#       deny.toml ignore is stale, and (with zero advisories, as of #525) an
#       empty ignore list is not itself treated as a bug
#   (b) each deny.toml ignore carries its own machine-readable reason, and no
#       two entries share one, and cargo-deny is configured to evaluate the
#       advisory classes those entries belong to (vacuous while the ignore
#       list is empty, but still enforced the moment it is not)
#   (c) no file claims GPY is waiting on, monitoring, or blocked by
#       hyperpolyglot/upstream for a fix, in any wording -- and, since both
#       lib.rs and the audit doc still name hyperpolyglot 0.1.7 as history,
#       each keeps stating the date that makes "no fix is coming" checkable
#   (d) no file claims the exposure is compile-time-only / build-only, and
#       each file states the edges are normal runtime edges and says why the
#       code is nevertheless unreachable -- dormant while `cargo audit` is
#       clean (#525), enforced again the moment an advisory is reported
#   (e) the recorded audit date is real, agrees across files, is not in the
#       future, and is paired with a review date that is still ahead of today
#       and within a bounded window (now a duplicate-version re-derivation
#       date, not an advisory-waiver expiry -- see lib.rs and the audit doc)
#   (f) no unverifiable superlative is used to justify keeping hyperpolyglot
#       (dormant now that hyperpolyglot is gone, kept for the next dependency
#       someone tries to justify this way)
#   (g) every crate and version in the duplicate-version table exists in
#       gpy-agent/Cargo.lock with more than one version
#   (h) every crate in deny.toml's `[bans] skip` list is genuinely duplicated
#   (i) the three registers name one canonical home so they cannot silently
#       drift apart again

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

LIB="gpy-agent/src/lib.rs"
DENY="gpy-agent/deny.toml"
DOC="docs/dev/security-audit.md"
LOCK="gpy-agent/Cargo.lock"
SECURITY="SECURITY.md"
CANONICAL="docs/dev/security-audit.md"

# Files that make claims about the advisory waiver. Every ban below is applied
# to all of them; a claim moved from one to another does not escape.
CLAIM_FILES=("$LIB" "$DENY" "$DOC")

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

for f in "${CLAIM_FILES[@]}" "$LOCK"; do
    if [[ ! -f "$f" ]]; then
        fail "$f not found"
        echo "$failures assertion(s) failed"
        exit 1
    fi
done

# --- helpers ----------------------------------------------------------------

# Body of deny.toml's `[advisories] ignore = [...]` array, one entry per line.
deny_ignore_entries() {
    awk '
        /^\[[a-z]+\]/ { section = $0 }
        section == "[advisories]" && /^ignore[[:space:]]*=[[:space:]]*\[/ { grab = 1; next }
        grab && /^\]/ { grab = 0 }
        grab && /[^[:space:]]/ { print }
    ' "$DENY"
}

# Crate names in deny.toml's `[bans] skip = [...]` array.
deny_skip_crates() {
    awk '
        /^\[[a-z]+\]/ { section = $0 }
        section == "[bans]" && /^skip[[:space:]]*=[[:space:]]*\[/ { grab = 1; next }
        grab && /^\]/ { grab = 0 }
        grab { print }
    ' "$DENY" | sed -nE 's/.*name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p'
}

# All versions of a crate present in the lockfile, one per line.
lock_versions() {
    awk -v want="$1" '
        /^name = / { cur = $3; gsub(/"/, "", cur) }
        /^version = / { if (cur == want) { v = $3; gsub(/"/, "", v); print v } }
    ' "$LOCK"
}

# Rows of the duplicate-version table in lib.rs, as `crate<TAB>versions`.
# The table is a `//` comment block introduced by a `Duplicate crate versions`
# heading; rows are pipe-delimited so they stay machine-checkable.
lib_duplicate_rows() {
    awk '
        /^\/\/ #+ .*[Dd]uplicate crate versions/ { grab = 1; next }
        grab && /^\/\/ #+ / { grab = 0 }
        grab && /^\/\/ \|/ {
            line = $0
            sub(/^\/\/ \|[[:space:]]*/, "", line)
            n = split(line, cell, /[[:space:]]*\|[[:space:]]*/)
            if (n < 2) next
            if (cell[1] ~ /^-+$/) next
            if (cell[1] == "crate") next
            print cell[1] "\t" cell[2]
        }
    ' "$LIB"
}

# ISO date -> epoch seconds at midnight, BSD (macOS) form first, GNU second.
#
# The explicit 00:00:00 is load-bearing (#526). Given only '%Y-%m-%d', BSD
# `date -j -f` fills the unspecified time fields from the current clock, so the
# same date parsed twice a second apart yields two different epochs. Callers
# here compare a date parsed at the top of a section against one parsed forty
# greps later; without a pinned time that drift reported today's date as being
# in the future whenever the run crossed a second boundary. GNU `date -d`
# already defaults to midnight, so only the BSD branch needed the anchor.
to_epoch() {
    date -j -f '%Y-%m-%d %H:%M:%S' "$1 00:00:00" '+%s' 2>/dev/null && return 0
    date -d "$1" '+%s' 2>/dev/null && return 0
    return 1
}

# First ISO date on a line matching the given label, in the given file.
dated_line() {
    grep -Eio "$1[^0-9]{0,24}[0-9]{4}-[0-9]{2}-[0-9]{2}" "$2" |
        grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}' | head -n1
}

# Overlapping three-line windows of a file, comment markers stripped and the
# lines joined, prefixed with the line number the window starts at.
#
# Prose bans have to run over these, not over raw lines. A claim that wraps --
# "We are holding off on any change here until the upstream project ships a /
# corrected release of its own." -- puts its actor on one line and its verb on
# the next, and a line-based grep sees neither half as a claim. Three lines is
# about one wrapped sentence: wide enough to catch the wrap, narrow enough that
# unrelated sentences do not collide.
claim_windows() {
    sed -E 's@^[[:space:]]*(//|#)+[[:space:]]?@@' "$1" |
        awk '
            { l[NR] = $0 }
            END {
                for (i = 1; i <= NR; i++) {
                    w = l[i]
                    # A wrapped sentence never crosses a blank line, so a
                    # paragraph break ends the window. Without this, the last
                    # line of one paragraph collides with the first of the next.
                    for (j = i + 1; j <= i + 2 && j <= NR; j++) {
                        if (l[j] ~ /^[[:space:]]*$/) break
                        w = w " " l[j]
                    }
                    print i ": " w
                }
            }'
}

# --- (a) advisory coverage, derived from cargo audit ------------------------
echo "--- every reported advisory is ignored deliberately and documented ---"

audit_output=""
audit_ran=0
if command -v cargo >/dev/null 2>&1 && cargo audit --version >/dev/null 2>&1; then
    audit_output="$(cd "$ROOT/gpy-agent" && cargo audit 2>&1 || true)"
    if ! printf '%s' "$audit_output" | grep -q 'Scanning Cargo.lock'; then
        # Advisory-database fetch needs the network. Retry against whatever
        # copy is already on disk before giving up.
        audit_output="$(cd "$ROOT/gpy-agent" && cargo audit --no-fetch 2>&1 || true)"
    fi
    if printf '%s' "$audit_output" | grep -q 'Scanning Cargo.lock'; then
        audit_ran=1
    fi
fi

reported_ids=""
if [[ "$audit_ran" -eq 1 ]]; then
    reported_ids="$(printf '%s\n' "$audit_output" |
        grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}' | sort -u)"
    if [[ -z "$reported_ids" ]]; then
        echo "note: cargo audit reports no advisories; coverage checks are vacuous"
    fi
else
    echo "cargo audit output (first lines), to tell a missing tool from a failed fetch:"
    printf '%s\n' "${audit_output:-<cargo audit was not run: not installed>}" | head -n 15 | sed 's/^/    /'
    echo "SKIP: cargo audit could not scan the lockfile (not installed, or the"
    echo "      advisory database is unavailable offline); the advisory-ID list"
    echo "      could not be derived. Every other assertion below still runs."
    # Under CI the audit tooling is installed and the network is available,
    # so a scan that did not happen is a broken gate, not an environment
    # to tolerate (#650). Locally the remaining assertions still run.
    if [[ -n "${CI:-}" ]]; then
        fail "cargo audit did not scan the lockfile under CI; the advisory coverage check is unverified (#650)"
    fi
fi

ignored_ids="$(deny_ignore_entries | grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}' | sort -u)"
# An empty ignore list is the correct, current state (#525): `cargo audit`
# reports zero advisories now that hyperpolyglot -- the source of all five
# the ignore list used to carry -- is gone (#523). Only fail here if there is
# something that actually needs a waiver and none is recorded; an empty list
# with nothing to waive is not the bug #503 found.
if [[ "$audit_ran" -eq 1 && -n "$reported_ids" && -z "$ignored_ids" ]]; then
    fail "cargo audit reports advisories but $DENY has no [advisories] ignore entries recording a waiver for them (#503)"
fi

if [[ "$audit_ran" -eq 1 ]]; then
    while IFS= read -r id; do
        [[ -n "$id" ]] || continue
        printf '%s\n' "$ignored_ids" | grep -qx "$id" ||
            fail "cargo audit reports $id but $DENY does not ignore it; it passes silently by default, not by decision (#503)"
        grep -q "$id" "$LIB" ||
            fail "cargo audit reports $id but $LIB does not document it (#503)"
        grep -q "$id" "$DOC" ||
            fail "cargo audit reports $id but $DOC does not document it (#503)"
    done <<<"$reported_ids"

    while IFS= read -r id; do
        [[ -n "$id" ]] || continue
        printf '%s\n' "$reported_ids" | grep -qx "$id" ||
            fail "$DENY ignores $id but cargo audit no longer reports it; a stale ignore hides the next advisory against that crate (#503)"
    done <<<"$ignored_ids"
fi

# --- (b) per-advisory rationale, not one blanket comment --------------------
echo "--- each ignored advisory carries its own reason ---"
entry_count=0
reason_count=0
reasons=""
while IFS= read -r entry; do
    [[ -n "$entry" ]] || continue
    printf '%s' "$entry" | grep -q 'RUSTSEC-' || continue
    entry_count=$((entry_count + 1))
    reason="$(printf '%s' "$entry" | sed -nE 's/.*reason[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p')"
    if [[ -z "$reason" ]]; then
        fail "$DENY ignore entry has no machine-readable reason: $entry (#503)"
        continue
    fi
    if [[ "${#reason}" -lt 40 ]]; then
        fail "$DENY reason for $(printf '%s' "$entry" | grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}') is $((${#reason})) chars; too short to record a dependency path and a finding (#503)"
    fi
    reason_count=$((reason_count + 1))
    reasons+="$reason"$'\n'
done <<<"$(deny_ignore_entries)"

if [[ "$entry_count" -gt 0 ]]; then
    distinct="$(printf '%s' "$reasons" | grep -c '[^[:space:]]' || true)"
    unique="$(printf '%s' "$reasons" | sort -u | grep -c '[^[:space:]]' || true)"
    if [[ "$distinct" -ne "$unique" ]]; then
        fail "$DENY reuses the same reason for more than one advisory; a blanket justification is what #503 replaced"
    fi
fi

# --- (b2) cargo-deny actually evaluates the classes we are ignoring ---------
echo "--- deny.toml evaluates unsound and unmaintained advisories ---"
# cargo-deny 0.20 skips `unsound` advisories unless asked. Before #503 that is
# why RUSTSEC-2021-0145 and RUSTSEC-2026-0097 passed `cargo deny check` while
# `cargo audit` reported them: not a decision, a default. If this setting goes
# away the ignore entries below it become dead config and the next unsound
# advisory passes silently again.
for key in unsound unmaintained; do
    value="$(awk -v k="$key" '
        /^\[[a-z]+\]/ { section = $0 }
        section == "[advisories]" && $1 == k { print $3 }
    ' "$DENY" | tr -d '"')"
    if [[ -z "$value" ]]; then
        fail "$DENY does not set [advisories] $key; cargo-deny then decides for itself whether to evaluate that class, which is how two advisories went unreviewed (#503)"
    elif [[ "$value" == "none" ]]; then
        fail "$DENY sets [advisories] $key = \"none\"; the class is not evaluated at all and its ignore entries are dead config (#503)"
    fi
done

# --- (c) no upstream-fix dependency, in any wording -------------------------
echo "--- no claim that a fix is coming from hyperpolyglot/upstream ---"
# The banned claim is "GPY's plan for this advisory is that someone upstream
# fixes it". Three parts have to co-occur on one line: a deferral, an actor,
# and the event being deferred to. Any phrasing that puts all three together
# is the claim, whatever verbs it picks.
# Word boundaries matter: a bare `if` alternative also matches inside
# "specific", and a bare `no` matches inside "none of".
wait_word='\b(wait[a-z]*|await[a-z]*|monitor[a-z]*|pending|block[a-z]*|track[a-z]*|watch[a-z]*|hold[a-z]*|defer[a-z]*|until|once|when|if|after|would|will|expect[a-z]*|anticipat[a-z]*|hop(e|es|ing)|plan[a-z]*|intend[a-z]*|should)\b'
upstream_actor='\b(hyperpolyglot|upstream|maintainers?|monkslc|superloach)\b'
upstream_event='\b(updat[a-z]*|migrat[a-z]*|releas[a-z]*|fix[a-z]*|resolv[a-z]*|patch[a-z]*|maintain[a-z]*|unmaintained|reviv[a-z]*|new version|bump[a-z]*|clap ?v?[0-9])\b'
# A line only escapes when the negation sits immediately before the deferral,
# i.e. it is denying the wait. A negation appearing later in the line (". . .
# no other option") does not launder the claim.
negated_wait="\b(no|not|never|nothing|none|without|cannot|can.t|won.t|nor|neither)\b[^.]{0,30}$wait_word"

for f in "${CLAIM_FILES[@]}"; do
    hits="$(claim_windows "$f" |
        grep -Ei "$upstream_actor" |
        grep -Ei "$upstream_event" |
        grep -Ei "$wait_word" |
        grep -vEi "$negated_wait" || true)"
    if [[ -n "$hits" ]]; then
        fail "$f defers these advisories to an upstream fix; hyperpolyglot 0.1.7 is the highest version ever published and dates from 2020 (#503)"
        printf '%s\n' "$hits"
    fi
done

# An open action item pointed at upstream is the same claim in checkbox form.
for f in "${CLAIM_FILES[@]}"; do
    if grep -nEi '^[^A-Za-z]*\[ \].*(hyperpolyglot|upstream)' "$f"; then
        fail "$f carries an open action item waiting on hyperpolyglot/upstream (#503)"
    fi
done

# Deleting the false plan is not enough. A reader has to be able to see that no
# fix is coming and to check that for themselves, so the abandonment has to be
# stated with the version and the year that make it verifiable.
for f in "$LIB" "$DOC"; do
    grep -q '0\.1\.7' "$f" ||
        fail "$f does not name hyperpolyglot 0.1.7, the version the waiver is written against (#503)"
    grep -qE '2020' "$f" ||
        fail "$f does not record when hyperpolyglot last published; without it, 'abandoned' is an assertion the reader cannot check (#503)"
    grep -qEi '(abandon[a-z]*|no release|never published|highest version|last publish[a-z]*|last release|final release|no further release|dormant|unmaintained upstream)' "$f" ||
        fail "$f does not state that hyperpolyglot is abandoned; the waiver's whole premise is that no fix will arrive (#503)"
done

# --- (d) no compile-time-only exposure claim --------------------------------
# Gated by #525 rather than deleted. Every claim below is about how an
# advisory's exposure is characterised, so it has no subject while `cargo
# audit` is clean -- and its positive half would force reachability language
# ("normal", "cargo tree", "unreachable") back into prose that is now about
# duplicate crate versions. But the falsehood it bans is the one #503 was
# opened to remove, and the next advisory someone waives is exactly when it
# earns its place again. Gating it on a reported advisory keeps the teeth and
# drops the false positives, matching how (a) and (b) were handled.
if [[ "$audit_ran" -eq 1 && -n "$reported_ids" ]]; then
    echo "--- exposure is not described as compile-time-only ---"
    # `cargo tree -i <crate> -e build` prints nothing for all five: every chain is
    # a normal (runtime) dependency edge. Calling it build-time-only is false, and
    # it is the specific falsehood #503 was opened to remove.
    build_only='((compile|build|link|codegen|dev)[- ]?time[- ]?only|build[- ]?only|compile[- ]?only)'
    build_only_prose='(only|solely|purely|merely|just|exclusively)[^.]{0,30}(at |during |a )?(compile|build|link|codegen)[- ]?time'
    build_only_scope='(isolated|limited|confined|scoped|restricted|contained|sandbox[a-z]*)[^.]{0,30}(to[^.]{0,10})?(build|compile|codegen|link)[- ]?(time|/test|and test)'
    advisory_crates='(ansi_term|atty|yaml.rust|serde_yaml|phf_codegen|phf_generator|\brand\b|\bclap\b)'
    build_only_edge='(build|compile|dev|codegen)[- /]?(time[- ])?(dependenc(y|ies)|edge)'

    for f in "${CLAIM_FILES[@]}"; do
        for pattern in "$build_only" "$build_only_prose" "$build_only_scope"; do
            if claim_windows "$f" | grep -Ei "$pattern"; then
                fail "$f claims the advisory exposure is compile-time-only; every chain is a normal runtime edge (cargo tree -i <crate> -e build prints nothing) (#503)"
            fi
        done
        hits="$(claim_windows "$f" | grep -Ei "$advisory_crates" | grep -Ei "$build_only_edge" || true)"
        if [[ -n "$hits" ]]; then
            fail "$f describes an advisory crate as arriving on a build/dev dependency edge; all five are normal edges (#503)"
            printf '%s\n' "$hits"
        fi
    done

    # Again, removing the false claim is not enough: the true finding has to be
    # stated, and stated in a form the reader can re-derive.
    for f in "$LIB" "$DOC"; do
        grep -qEi '\bnormal\b' "$f" ||
            fail "$f does not say the advisory chains are normal (runtime) dependency edges; that correction is the point of #503"
        grep -qEi 'cargo tree' "$f" ||
            fail "$f does not name the command that re-derives the dependency paths, so the reachability claim cannot be checked (#503)"
        grep -qEi '(unreachable|never call[a-z]*|not call[a-z]*|no call|dead code|never invoked|not invoked|never exercised)' "$f" ||
            fail "$f does not state why the linked-but-advisory code is never reached; 'low risk' without a mechanism is not a finding (#503)"
    done
fi

# --- (e) the audit is dated, and the review date has not passed -------------

# to_epoch has to name an instant that depends only on its argument. Every
# check below compares one date parsed here against another parsed dozens of
# greps later, so a parse that inherits the current time-of-day makes those
# comparisons drift with how long the run takes -- which is how #526 reported
# today's date as being in the future on a cold run. Parsing the same date
# either side of a second boundary is the assertion that catches it: identical
# under the pinned-midnight form, always different under the bare '%Y-%m-%d'
# one this replaced.
echo "--- to_epoch does not depend on the time of day ---"
epoch_before="$(to_epoch '2026-09-01')"
sleep 1.1
epoch_after="$(to_epoch '2026-09-01')"
if [[ -z "$epoch_before" || -z "$epoch_after" ]]; then
    fail "to_epoch could not parse an ISO date on this platform"
elif [[ "$epoch_before" != "$epoch_after" ]]; then
    fail "to_epoch('2026-09-01') returned $epoch_before then $epoch_after; it is reading the current clock for the time fields, so every date comparison in this file drifts with the runtime (#526)"
fi

echo "--- audit and review dates are real and current ---"
today="$(date '+%Y-%m-%d')"
today_epoch="$(to_epoch "$today")"

lib_audit_date="$(dated_line 'audit(ed)?' "$LIB")"
doc_audit_date="$(dated_line '(last updated|audit(ed)?)' "$DOC")"
doc_review_date="$(dated_line '(next review|review (date|on|by))' "$DOC")"
lib_review_date="$(dated_line '(next review|review (date|on|by))' "$LIB")"

[[ -n "$lib_audit_date" ]] ||
    fail "$LIB records no audit date; an undated waiver cannot be said to have expired (#503)"
[[ -n "$doc_audit_date" ]] ||
    fail "$DOC records no audit date (#503)"
[[ -n "$doc_review_date" ]] ||
    fail "$DOC records no next-review date; cargo-deny's ignore list has no expiry field, so the date has to live here (#503)"
[[ -n "$lib_review_date" ]] ||
    fail "$LIB records no next-review date (#503)"

if [[ -n "$lib_audit_date" && -n "$doc_audit_date" && "$lib_audit_date" != "$doc_audit_date" ]]; then
    fail "$LIB says the audit ran $lib_audit_date, $DOC says $doc_audit_date; the two registers already disagreed once (#503)"
fi
if [[ -n "$lib_review_date" && -n "$doc_review_date" && "$lib_review_date" != "$doc_review_date" ]]; then
    fail "$LIB and $DOC disagree on the next review date ($lib_review_date vs $doc_review_date) (#503)"
fi

if [[ -n "$doc_audit_date" && -n "$today_epoch" ]]; then
    audit_epoch="$(to_epoch "$doc_audit_date")"
    if [[ -z "$audit_epoch" ]]; then
        fail "$DOC audit date '$doc_audit_date' is not a real date"
    elif [[ "$audit_epoch" -gt "$today_epoch" ]]; then
        fail "$DOC claims it was audited on $doc_audit_date, which is in the future"
    fi
fi

if [[ -n "$doc_review_date" && -n "$today_epoch" ]]; then
    review_epoch="$(to_epoch "$doc_review_date")"
    if [[ -z "$review_epoch" ]]; then
        fail "$DOC review date '$doc_review_date' is not a real date"
    else
        if [[ "$review_epoch" -le "$today_epoch" ]]; then
            fail "$DOC's next review date $doc_review_date has passed; re-run cargo audit, confirm the findings still hold, and move the date (#503)"
        fi
        if [[ -n "${audit_epoch:-}" ]]; then
            window=$(((review_epoch - audit_epoch) / 86400))
            if [[ "$window" -gt 400 ]]; then
                fail "$DOC sets the review $window days after the audit; a waiver with an effectively open-ended window is the state #503 replaced"
            fi
            if [[ "$window" -le 0 ]]; then
                fail "$DOC's review date is not after its audit date"
            fi
        fi
    fi
fi

# --- (f) no unverifiable superlative props up the waiver --------------------
echo "--- hyperpolyglot is not justified by an unverifiable superlative ---"
for f in "${CLAIM_FILES[@]}"; do
    if claim_windows "$f" | grep -Ei '(best|finest|leading|superior|fastest|most accurate)[- ]?(available|in[- ]class|option|choice|detector|library|there is)|only (viable|real|available|maintained|serious) (option|choice|detector|library)|no (real |viable |serious )?alternative'; then
        fail "$f justifies hyperpolyglot with a superlative no one measured; gengo 0.15.0 is a maintained alternative and the decision to keep hyperpolyglot rests on other grounds (#503)"
    fi
done

# --- (g) the duplicate-version table matches the lockfile -------------------
echo "--- duplicate-version claims match $LOCK ---"
rows="$(lib_duplicate_rows)"
if [[ -z "$rows" ]]; then
    fail "$LIB has no machine-checkable duplicate crate version table; the #![allow(clippy::multiple_crate_versions)] above it is then unjustified (#503)"
else
    while IFS=$'\t' read -r crate versions; do
        [[ -n "$crate" ]] || continue
        present="$(lock_versions "$crate")"
        count="$(printf '%s' "$present" | grep -c '[^[:space:]]' || true)"
        if [[ "$count" -eq 0 ]]; then
            fail "$LIB lists '$crate' as a duplicated crate but $LOCK has no such package (#503)"
            continue
        fi
        if [[ "$count" -lt 2 ]]; then
            fail "$LIB lists '$crate' as duplicated but $LOCK has exactly one version ($present); a crate that no longer duplicates should be dropped, not updated (#503)"
        fi
        for v in $(printf '%s' "$versions" | tr ',' ' '); do
            v="${v#v}"
            [[ "$v" =~ ^[0-9] ]] || continue
            printf '%s\n' "$present" | grep -qx "$v" ||
                fail "$LIB says $crate $v is in the tree; $LOCK has: $(printf '%s' "$present" | tr '\n' ' ') (#503)"
        done
    done <<<"$rows"
fi

# --- (h) every skipped duplicate is genuinely duplicated --------------------
echo "--- deny.toml [bans] skip entries are still duplicated ---"
while IFS= read -r crate; do
    [[ -n "$crate" ]] || continue
    count="$(lock_versions "$crate" | grep -c '[^[:space:]]' || true)"
    if [[ "$count" -eq 0 ]]; then
        fail "$DENY skips duplicate-checking for '$crate', which is not in $LOCK at all (#503)"
    elif [[ "$count" -lt 2 ]]; then
        fail "$DENY skips duplicate-checking for '$crate', which now has a single version; the skip hides a future duplicate (#503)"
    fi
done <<<"$(deny_skip_crates)"

# --- (i) one canonical register, named by the others ------------------------
echo "--- the three registers name one canonical home ---"
grep -q "$CANONICAL" "$LIB" ||
    fail "$LIB does not point at $CANONICAL as the canonical advisory register; three unlinked copies is how they drifted apart (#503)"
grep -q "$CANONICAL" "$DENY" ||
    fail "$DENY does not point at $CANONICAL as the canonical advisory register (#503)"

# --- guard: SECURITY.md is not contradicted ---------------------------------
# #497 deliberately left SECURITY.md without an advisory count or taxonomy.
# Reintroducing one there creates a fourth register that will drift.
if [[ -f "$SECURITY" ]]; then
    if grep -nEi '(there are|we have|currently)[^.]{0,30}[0-9]+[^.]{0,20}(advisor|vulnerab)' "$SECURITY"; then
        fail "$SECURITY states an advisory count; #497 removed it deliberately and $CANONICAL is the register (#503)"
    fi
    if grep -nE 'RUSTSEC-[0-9]{4}-[0-9]{4}' "$SECURITY"; then
        fail "$SECURITY names specific advisory IDs; that list belongs in $CANONICAL only (#503)"
    fi
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
