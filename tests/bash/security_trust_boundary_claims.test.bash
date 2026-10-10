#!/usr/bin/env bash
# tests/bash/security_trust_boundary_claims.test.bash
#
# Locks in the security-documentation decisions from #497.
#
# SECURITY.md described a program that does not exist. It called ordinary
# user-context execution "Sandboxed execution", claimed "No eval - No dynamic
# code execution in Fish shell" while all three shell integrations evaluate the
# agent's output, credited PID validation with preventing unauthorized socket
# access when `validate_pid` only proves a PID is live, and asserted blanket
# "Read-only operations" for a binary that writes caches, config, themes and a
# debug log. It also published an email address and promised acknowledgment
# "within 48 hours" for a personal project with no such capacity.
#
# The recorded decision on #497: vulnerability reports go through GitHub
# private vulnerability reporting, and no response-time commitment is given.
#
# Every assertion here is bound to the CLAIM, not to the sentence that
# happened to carry it. Deleting the exact historical wording and writing a
# fresh paraphrase of the same false claim must still turn this test red.
#
# Deliberately NOT asserted:
#   * `## Supported Versions` -- owned by
#     tests/bash/release_version_claims.test.bash, which derives the table from
#     CHANGELOG.md. This test only checks the heading still exists.
#   * The dependency-audit rationale in gpy-agent/src/lib.rs -- owned by #503.
#   * Release-integrity wording -- owned by #494 and already asserted by
#     tests/bash/install_checksum_verification.test.bash and
#     tests/bash/install_oneline_verification.test.bash.
#
# Asserts:
#   (a) no claim that the agent runs sandboxed/isolated/confined, and the
#       absence of a sandbox is stated out loud rather than merely deleted
#   (b) no "no eval" / "no dynamic code execution" claim in any wording
#   (c) the section that documents shell evaluation names every shell that
#       actually evaluates agent output -- the shell list is derived from the
#       integration sources, not hardcoded
#   (d) the socket's owner-only permission is documented as the access
#       control, and PID validation is not credited with authenticating peers
#   (e) filesystem claims are scoped: no blanket read-only claim, the
#       repository claim is qualified, and the paths GPY does write are named
#   (f) the trust relationship between shell, socket, agent output and
#       downloaded binaries is documented in one place
#   (g) no email address is offered as the reporting channel, GitHub private
#       vulnerability reporting is, and no response-time commitment is made

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

SECURITY="SECURITY.md"
BASH_INIT="bash/core/init.bash"
ZSH_INIT="zsh/core/init.zsh"
FISH_INIT="fish/core/init.fish"
IPC_HANDLE="gpy-agent/src/ipc/server/handle.rs"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

if [[ ! -f "$SECURITY" ]]; then
    fail "$SECURITY not found"
    echo "$failures assertion(s) failed"
    exit 1
fi

# Body of a `##`/`###` heading block, by heading text. `^###? ` rather than
# `^#{2,3} `: the runner's awk is mawk, which mis-handles the interval
# expression (it matches `## A` but not `### B`) and silently ended every
# section at the first `###` heading (#848).
section_body() {
    awk -v want="$1" '
        /^###? / {
            heading = $0
            sub(/^#+ /, "", heading)
            grab = (heading == want)
            next
        }
        grab { print }
    ' "$SECURITY"
}

# Every `##`/`###` heading in the document, one per line.
all_headings() {
    grep -E '^#{2,3} ' "$SECURITY" | sed -E 's/^#+ //'
}

# --- (a) no sandbox claim, and the absence of one is stated ------------------
echo "--- user-context execution is not described as sandboxed ---"
# Any sentence putting GPY inside a sandbox/jail/isolated context is a claim
# about a mechanism that does not exist: there is no seccomp filter, no
# namespace, no entitlement restriction. A line is allowed only when it is
# denying the thing.
sandbox_negation='\b(no|not|never|neither|nothing|without|does not|doesn.t|cannot|can.t|is not|are not|un)\b|\bno\b'
if grep -nEi '(sandbox|jail(ed|s)?\b|isolat[a-z]*|confin[a-z]*|seccomp|namespace[a-z]*|entitlement[a-z]*)' "$SECURITY" |
    grep -vEi "$sandbox_negation"; then
    fail "$SECURITY asserts sandboxing/isolation for the agent; it runs with the full authority of the invoking user (#497)"
fi
# A rephrased reintroduction that keeps a negation elsewhere on the line still
# has to be caught, so ban the positive construction outright.
if grep -nEi '(sandbox|jail|isolat[a-z]*|confin[a-z]*)[^.]{0,60}(execution|environment|context|process|agent)' "$SECURITY" |
    grep -vEi "$sandbox_negation"; then
    fail "$SECURITY still frames agent execution as sandboxed/isolated (#497)"
fi
if grep -nEi '(runs|executes|operates|lives)[^.]{0,60}(sandbox|jail|isolat[a-z]*|confin[a-z]*)' "$SECURITY" |
    grep -vEi "$sandbox_negation"; then
    fail "$SECURITY still says the agent runs sandboxed (#497)"
fi
# Deleting the false claim is not enough: a reader scanning for weaknesses has
# to be able to find that there is no sandbox, and to find it stated concretely
# enough to be checkable. A bare "no sandbox" bullet with the mechanisms
# stripped out is the shape this used to fail in.
grep -qEi '(no|not|never|without)[^.]{0,80}sandbox' "$SECURITY" ||
    fail "$SECURITY does not state anywhere that the agent is NOT sandboxed; removing the claim without stating the limitation hides it (#497)"
grep -qEi '(no|not|never|without)[^.]{0,80}(seccomp|namespace|entitlement|privilege separation|chroot)' "$SECURITY" ||
    fail "$SECURITY says nothing about WHICH confinement mechanism is absent (seccomp, namespaces, entitlements); an unqualified 'no sandbox' is not checkable (#497)"

# --- (b) no "no eval" claim in any wording ----------------------------------
echo "--- no 'no eval' / 'no dynamic code execution' claim ---"
if grep -nEi '(\bno\b|\bzero\b|\bwithout\b)[[:space:][:punct:]]*([a-z*`_-]+[[:space:]]+){0,3}(eval\b|dynamic[[:space:]]+code[[:space:]]+execution|dynamic[[:space:]]+execution)' "$SECURITY"; then
    fail "$SECURITY claims there is no eval / no dynamic code execution; all three shell integrations evaluate agent output (#497)"
fi
if grep -nEi '(does not|doesn.t|never|avoids?|refuses to|neither|free of|rather than)[^.]{0,60}\beval' "$SECURITY"; then
    fail "$SECURITY denies evaluating agent output; bash/zsh use eval and fish uses source (#497)"
fi
# The denial does not have to say "eval" to be the same false claim -- "never
# executes code the agent produced" asserts exactly what bash/core/init.bash
# line `eval "$(gpy-agent theme export ...)"` disproves. Match the claim.
if grep -nEi '(does not|doesn.t|never|avoids?|refuses to|neither)[^.]{0,60}(execute|run|interpret|source)s?[^.]{0,50}\b(code|output|shell)' "$SECURITY"; then
    fail "$SECURITY denies executing agent-produced code; all three integrations do (#497)"
fi

# --- (c) shell evaluation is scoped to every shell that does it -------------
echo "--- shell evaluation is documented for every shell that evaluates ---"
# Derived from the integration sources: whichever shells actually evaluate the
# agent's theme export are the shells the doc has to name. Adding a fourth
# shell integration that evals turns this red until SECURITY.md says so.
evaluating_shells=()
if [[ -f "$BASH_INIT" ]]; then
    grep -qE 'eval[[:space:]]*"?\$\(' "$BASH_INIT" && evaluating_shells+=("Bash")
else
    fail "$BASH_INIT not found"
fi
if [[ -f "$ZSH_INIT" ]]; then
    grep -qE 'eval[[:space:]]*"?\$\(' "$ZSH_INIT" && evaluating_shells+=("Zsh")
else
    fail "$ZSH_INIT not found"
fi
if [[ -f "$FISH_INIT" ]]; then
    grep -qE '\|[[:space:]]*source|^[[:space:]]*source[[:space:]]+"' "$FISH_INIT" && evaluating_shells+=("Fish")
else
    fail "$FISH_INIT not found"
fi
if [[ ${#evaluating_shells[@]} -lt 3 ]]; then
    fail "expected Bash, Zsh and Fish to all evaluate agent output; derived [${evaluating_shells[*]}] from the integration sources"
fi

# Naming a shell somewhere in the section is not enough: the opening sentence
# lists all three as supported shells, so a doc could drop Bash from the
# evaluation claim and still mention it. Require each evaluating shell to
# appear in the same SENTENCE as an eval/source token. The file is reflowed to
# one line first, because these paragraphs are hard-wrapped.
security_flat="$(tr '\n' ' ' <"$SECURITY" | tr -s ' ')"
for shell_name in "${evaluating_shells[@]}"; do
    if ! printf '%s' "$security_flat" |
        grep -qEi "${shell_name}[^.]{0,200}(\beval\b|\bsource\b)|(\beval\b|\bsource\b)[^.]{0,200}$shell_name"; then
        fail "$SECURITY never names $shell_name in the same sentence as an eval/source claim; $shell_name evaluates agent output (#497)"
    fi
done

eval_sections=0
while IFS= read -r heading; do
    [[ -n "$heading" ]] || continue
    body="$(section_body "$heading")"
    grep -qiE '\beval|evaluat' <<<"$body" || continue
    eval_sections=$((eval_sections + 1))
    for shell in "${evaluating_shells[@]}"; do
        if ! grep -qiF "$shell" <<<"$body"; then
            fail "$SECURITY's '$heading' section describes shell evaluation but does not name $shell, which evaluates agent output (#497)"
        fi
    done
done < <(all_headings)
if [[ "$eval_sections" -eq 0 ]]; then
    fail "$SECURITY has no section describing shell evaluation of agent output at all (#497)"
fi

# --- (d) the real socket access control is documented -----------------------
echo "--- socket access control is the owner-only mode, not PID validation ---"
if [[ -f "$IPC_HANDLE" ]]; then
    grep -qF 'set_mode(0o600)' "$IPC_HANDLE" ||
        fail "$IPC_HANDLE no longer chmods the socket to 0600; SECURITY.md's access-control claim needs rechecking"
    grep -qE 'mode\(\)[[:space:]]*&[[:space:]]*0o022' "$IPC_HANDLE" ||
        fail "$IPC_HANDLE no longer rejects a group/world-writable runtime dir; SECURITY.md's claim needs rechecking"
else
    fail "$IPC_HANDLE not found"
fi
grep -qEi '0600|0o600|owner[- ]only|only the owning user|owner of the socket' "$SECURITY" ||
    fail "$SECURITY does not document the socket's owner-only permission, which is the actual access control (#497)"
grep -qEi '(group|world|other)[- ]?writ' "$SECURITY" ||
    fail "$SECURITY does not document the refusal to bind in a group/world-writable runtime directory (#497)"
# PID validation proves a PID is live. It authenticates nothing.
if grep -nEi 'PID[^.]{0,80}(prevent|prevents|block(s|ing)?|stop(s)?)[^.]{0,60}(unauthori[sz]ed|unauthenticated|access)' "$SECURITY"; then
    fail "$SECURITY credits PID validation with preventing unauthorized access; validate_pid only checks the PID is well-formed and live (#497)"
fi
if grep -nEi 'PID[^.]{0,60}(authenticat[a-z]*|access control|identif(y|ies) the (peer|caller|client))' "$SECURITY" |
    grep -vEi '\b(no|not|never|does not|doesn.t|cannot|can.t|rather than|instead of|is not)\b'; then
    fail "$SECURITY presents PID validation as peer authentication; any local process may claim any live PID (#497)"
fi

# --- (e) filesystem claims are scoped ---------------------------------------
echo "--- filesystem write claims are scoped to what GPY actually does ---"
if grep -nEi '(read[- ]only|read only)[^.]{0,40}(operation|access only|by design|throughout|everywhere)' "$SECURITY"; then
    fail "$SECURITY claims blanket read-only operation; GPY writes caches, config, themes and a debug log (#497)"
fi
# A "never writes" claim is only true when it is scoped to repositories.
if grep -nEi '(never|does not|doesn.t|\bno\b)[^.]{0,30}\bwrit' "$SECURITY" |
    grep -vEi 'repositor|working tree|working copy|checkout|your code|watched (tree|repo)'; then
    fail "$SECURITY makes an unqualified 'does not write' claim; only the repository working tree is never written (#497)"
fi
# And the paths it does write have to be named, or the scoping is invisible.
write_targets=0
grep -qEi 'cache' "$SECURITY" && write_targets=$((write_targets + 1))
grep -qEi 'config(uration)?' "$SECURITY" && write_targets=$((write_targets + 1))
grep -qEi 'theme' "$SECURITY" && write_targets=$((write_targets + 1))
if [[ "$write_targets" -lt 3 ]]; then
    fail "$SECURITY does not name the cache/config/theme paths GPY writes; scoping the repository claim requires saying where writes DO go (#497)"
fi

# --- (f) the trust boundary is documented in one place ----------------------
echo "--- trust boundary covers shell, socket, agent output and downloads ---"
trust_heading="$(grep -m1 -oE '^#{2,3} .*[Tt]rust.*' "$SECURITY" | sed -E 's/^#+ //')"
if [[ -z "$trust_heading" ]]; then
    fail "$SECURITY has no trust-boundary section (#497 criterion 3)"
else
    trust_body="$(section_body "$trust_heading")"
    for term in 'shell' 'socket' 'binar'; do
        grep -qi "$term" <<<"$trust_body" ||
            fail "$SECURITY's '$trust_heading' section does not cover '$term'; the shell client, the local socket, the agent's output and the downloaded binaries all belong in one trust statement (#497)"
    done
    # The consequence a reader deserves: a spoofed gpy-agent on PATH runs
    # arbitrary code in every new shell.
    grep -qiE 'PATH|spoof|replac|tamper|compromis' <<<"$trust_body" ||
        fail "$SECURITY's '$trust_heading' section does not say what a compromised or spoofed gpy-agent on PATH would get (#497)"
    # And the mitigations that actually exist, not more than exist.
    grep -qiE 'checksum|verif' <<<"$trust_body" ||
        fail "$SECURITY's '$trust_heading' section names no install-time verification as a mitigation (#497)"
fi

# --- (g) reporting channel and response commitments -------------------------
echo "--- reporting goes through GitHub private reporting, with no SLA ---"
if grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' "$SECURITY"; then
    fail "$SECURITY publishes an email address; reports go through GitHub private vulnerability reporting (#497)"
fi
grep -qiE 'private vulnerability report|privately report a vulnerability|report a vulnerability privately' "$SECURITY" ||
    fail "$SECURITY does not name GitHub private vulnerability reporting as the channel (#497)"
grep -qF '/security/advisories' "$SECURITY" ||
    fail "$SECURITY does not link the repository's security advisories page, so a reporter has no route to follow (#497)"
# The route has to be in the section a reporter reads, not only in a footer.
report_heading="$(grep -m1 -oE '^#{2,3} .*How to Report.*' "$SECURITY" | sed -E 's/^#+ //')"
[[ -n "$report_heading" ]] ||
    report_heading="$(grep -m1 -oE '^#{2,3} .*Reporting a Vulnerability.*' "$SECURITY" | sed -E 's/^#+ //')"
if [[ -z "$report_heading" ]]; then
    fail "$SECURITY has no 'How to Report' / 'Reporting a Vulnerability' section (#497)"
else
    section_body "$report_heading" | grep -qF '/security/advisories/new' ||
        fail "$SECURITY's '$report_heading' section does not carry the private-reporting URL a reporter has to open (#497)"
fi

# No response-time commitment, in any wording.
if grep -nEi '(within|in|under)[[:space:]]+(one|two|three|a|[0-9]+)[[:space:]-]*(hour|day|business day|week|month)s?' "$SECURITY" |
    grep -vEi '\b(no|not|never|cannot|can.t|does not|doesn.t)\b'; then
    fail "$SECURITY commits to a response window; the decision on #497 is that no response-time commitment is given"
fi
if grep -nEi '(we will|i will|you can expect|expect a|guarantee[a-z]*|commit(ment|s|ted)?|aim to|strive to)[^.]{0,60}(acknowledg|respond|reply|response|triage)' "$SECURITY" |
    grep -vEi '\b(no|not|never|cannot|can.t|does not|doesn.t|without)\b'; then
    fail "$SECURITY promises acknowledgment or a response; #497 records that no response commitment is given"
fi
# The absence of a commitment has to be stated, not merely implied by deletion.
grep -qEi '(no|not|cannot|can.t|without)[^.]{0,90}(guarantee[a-z]*|commit[a-z]*|promise[a-z]*|assur[a-z]*)' "$SECURITY" ||
    fail "$SECURITY does not state plainly that no response window is guaranteed (#497)"
grep -qEi 'personal project|solo|single maintainer|one maintainer|spare[- ]time|hobby' "$SECURITY" ||
    fail "$SECURITY does not say this is a personal project, which is the reason no window is promised (#497)"

# --- guard: the section another test owns is still present ------------------
grep -qE '^## Supported Versions' "$SECURITY" ||
    fail "$SECURITY lost its '## Supported Versions' heading, which tests/bash/release_version_claims.test.bash derives from CHANGELOG.md"

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
