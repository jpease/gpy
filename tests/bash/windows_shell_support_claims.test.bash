#!/usr/bin/env bash
# tests/bash/windows_shell_support_claims.test.bash
#
# Locks in the decision from #496: WSL is recommended for Windows, native
# Windows is CLI-only (the prompt integration needs Unix domain sockets,
# which native Windows does not provide), and no document may claim a
# support level that a green CI run does not back. The `windows-latest` leg
# is the reusable `.github/workflows/windows-gate.yml`, called by pr-gate.yml
# (every pull request) and cross-platform-test.yml (post-merge and weekly).
#
# Also locks in the real IPC transport chain documented for #496: Zsh can
# run with no external tool at all (the `zsh/net/socket` builtin), Fish and
# Bash cannot; neither Bash nor Zsh has a `/dev/tcp`-to-Unix-socket path.
#
# Asserts:
#   (a) no tracked doc claims native-Windows full/native prompt support
#   (b) the canonical support matrix exists in docs/INSTALL.md and names
#       Bash, Fish, and Zsh
#   (c) socat / nc -U / awk requirements are documented per shell
#   (d) degraded oneshot behavior is explained
#   (e) no doc claims Bash or Zsh IPC uses /dev/tcp
#   (f) Cargo metadata, crate docs, and CLI help agree it's not Fish-only
#   (g) docs/INSTALL.md's blunt "not supported" line for native Windows
#       (PowerShell) is intact -- the decision is "no softening"
#   (h) the documented CI coverage claim matches the pr-gate.yml/
#       cross-platform-test.yml workflows
#   (i) fish/fisher.json's fish_version and os fields match the canonical
#       Fish-version and native-Windows policy, derived from docs/INSTALL.md
#       rather than hardcoded (2026-09-03 audit follow-up)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

INSTALL_DOC="docs/INSTALL.md"
AGENT_MANIFEST="gpy-agent/Cargo.toml"
AGENT_LIB="gpy-agent/src/lib.rs"
AGENT_MAIN="gpy-agent/src/main.rs"
AGENT_CLI="gpy-agent/src/bin/gpy.rs"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# (a) no tracked doc overclaims native-Windows prompt support. These are the
# specific phrasings this issue found and fixed; a blanket ban on the word
# "Windows" would also flag legitimate mentions of competitors (Starship,
# Oh-My-Posh) that DO support native Windows.
echo "--- no native-Windows overclaim ---"
while IFS= read -r doc; do
    if grep -nF 'Native support for Linux, macOS, and Windows' "$doc"; then
        fail "$doc claims native support for Windows; native Windows is CLI-only (#496)"
    fi
    # Catch any NEW phrasing of the same overclaim, not just the one string
    # this issue happened to find. Competitor write-ups legitimately say
    # Starship and Oh-My-Posh support native Windows, so the broad scan skips
    # that file and relies on the GPY-row check below instead.
    if [[ "$doc" != "docs/COMPETITIVE_COMPARISON.md" ]]; then
        if grep -nEi '(native[ -]?windows|powershell)[^.|]{0,60}(is |are )?(fully |full |native )?support' "$doc" |
            grep -vEi 'not supported|no native|CLI-only|does not|is not|intended scope|not a confirmed|unconfirmed|not confirmed' ; then
            fail "$doc claims native Windows/PowerShell support; native Windows is CLI-only (#496)"
        fi
    fi
    # Only GPY's own matrix cell matters here -- competitor rows in
    # docs/COMPETITIVE_COMPARISON.md legitimately say "Full" for tools that
    # DO support native Windows (Starship, Oh-My-Posh). GPY's cell is the
    # one immediately after the "**Windows**" row label.
    if grep -nE '\*\*Windows\*\*[[:space:]]*\|[[:space:]]*✅ Full' "$doc"; then
        fail "$doc marks Windows as fully supported for GPY; only WSL is (#496)"
    fi
done < <(git ls-files '*.md' 'gpy-agent/*.md')

# (b) the canonical support matrix lives in docs/INSTALL.md and names all
# three shells.
echo "--- canonical support matrix exists and names all three shells ---"
if [[ ! -f "$INSTALL_DOC" ]]; then
    fail "$INSTALL_DOC not found"
else
    grep -qF '## Supported Platforms and Shells' "$INSTALL_DOC" ||
        fail "$INSTALL_DOC has no canonical 'Supported Platforms and Shells' section (#496)"
    for shell in Bash Fish Zsh; do
        grep -qF "| $shell |" "$INSTALL_DOC" ||
            fail "$INSTALL_DOC's support matrix does not name $shell"
    done
fi

# (c) socat / nc -U / awk requirements are documented per shell/platform.
echo "--- socat/nc/awk dependencies documented ---"
grep -qF 'socat' "$INSTALL_DOC" || fail "$INSTALL_DOC does not document socat"
grep -qF 'nc -U' "$INSTALL_DOC" || fail "$INSTALL_DOC does not document nc -U"
grep -qF 'awk' "$INSTALL_DOC" || fail "$INSTALL_DOC does not document the awk dependency"
grep -qF 'zsh/net/socket' "$INSTALL_DOC" ||
    fail "$INSTALL_DOC does not document Zsh's zsh/net/socket builtin (needs no external tool)"

# (d) degraded oneshot behavior is explained.
echo "--- degraded oneshot behavior is explained ---"
if grep -qi 'oneshot' "$INSTALL_DOC"; then
    grep -qiE 'fall back|fallback|degraded' "$INSTALL_DOC" ||
        fail "$INSTALL_DOC mentions oneshot but does not explain it as a fallback/degraded path"
else
    fail "$INSTALL_DOC does not explain oneshot degradation when IPC tools are absent (#496)"
fi

# (e) no doc claims Bash or Zsh IPC uses /dev/tcp -- neither shell has that
# path in the real implementation (bash/core/ipc.bash, zsh/core/ipc.zsh).
echo "--- no /dev/tcp claim for Bash or Zsh IPC ---"
while IFS= read -r doc; do
    # Match /dev/tcp however it is marked up -- backticks, quotes, or bare.
    # The only legitimate mention is an explicit denial that the path exists.
    if grep -nF '/dev/tcp' "$doc" | grep -vEi 'no /dev/tcp|has no|does not|never'; then
        fail "$doc still claims a /dev/tcp IPC path; Bash has none and Zsh uses zsh/net/socket (#496)"
    fi
done < <(git ls-files '*.md')

# (f) Cargo metadata, crate docs, and CLI help all agree GPY is not Fish-only.
echo "--- Cargo metadata and CLI help agree on shell support ---"
if [[ -f "$AGENT_MANIFEST" ]]; then
    grep -qF 'used by the GPY Fish shell prompt' "$AGENT_MANIFEST" &&
        fail "$AGENT_MANIFEST description still claims Fish-only (#496)"
    if ! grep -q 'description.*Bash' "$AGENT_MANIFEST" || ! grep -q 'description.*Zsh' "$AGENT_MANIFEST"; then
        fail "$AGENT_MANIFEST description does not mention Bash and Zsh"
    fi
fi
for target in "$AGENT_LIB" "$AGENT_MAIN" "$AGENT_CLI"; do
    [[ -f "$target" ]] || { fail "$target not found"; continue; }
    if grep -nE 'Fish [Ss]hell [Pp]rompt|Fish shell prompt system' "$target"; then
        fail "$target still frames GPY as Fish-only (#496)"
    fi
done

# (h) the CI claim matches the workflows. #496 originally shipped "only
# ubuntu-latest and macos-latest run on pull_request" and "the windows leg has
# never gone green (#482)". Both were stale: pr-gate.yml gained a PR-blocking
# windows-latest leg when #482 closed. Derive the fact instead of asserting a
# sentence, so the doc cannot drift from the workflow again.
echo "--- the documented CI coverage matches the workflows ---"
PR_GATE=".github/workflows/pr-gate.yml"
if [[ ! -f "$PR_GATE" ]]; then
    fail "$PR_GATE not found"
else
    # The Windows leg is the reusable windows-gate.yml; it is a PR gate when
    # pr-gate.yml runs on pull_request and calls it, and that workflow runs on
    # windows-latest.
    WINDOWS_GATE=".github/workflows/windows-gate.yml"
    pr_gate_has_windows=0
    grep -qE '^\s*runs-on:\s*windows-latest' "$WINDOWS_GATE" &&
        grep -qE '^    uses: \./\.github/workflows/windows-gate\.yml$' "$PR_GATE" &&
        grep -qE '^\s*pull_request:' "$PR_GATE" && pr_gate_has_windows=1

    if [[ $pr_gate_has_windows -eq 1 ]]; then
        # Windows IS a PR gate: the docs must not say otherwise.
        if grep -nEi 'only .{0,40}(ubuntu|macos).{0,40}(run|gate).{0,30}pull_request' "$INSTALL_DOC"; then
            fail "$INSTALL_DOC says only ubuntu/macos gate PRs, but $PR_GATE runs windows-latest on pull_request"
        fi
        if grep -nEi 'windows.{0,80}(never (gone|been) green|no green CI run)' "$INSTALL_DOC"; then
            fail "$INSTALL_DOC says the Windows leg has never been green, but $PR_GATE gates PRs on it"
        fi
        if tr '\n' ' ' <"$INSTALL_DOC" | grep -qEi 'windows[^.]{0,120}(manual dispatch|dispatching|disabled)|(trigger|triggers)[^.]{0,80}(is|are) (temporarily )?disabled'; then
            fail "$INSTALL_DOC says the Windows leg is manual or its trigger is disabled, but $PR_GATE gates PRs on it (#556)"
        fi
        grep -qF 'pr-gate.yml' "$INSTALL_DOC" ||
            fail "$INSTALL_DOC does not mention pr-gate.yml, which is what actually backs the Windows claim"
    else
        # No PR gate. Where the remaining Windows coverage comes from depends
        # on the smoke workflow's own triggers, which were commented out on
        # 2026-09-02 to stop metered Actions minutes bleeding (#556). "Windows
        # runs post-merge" was true before that and false after it, so the
        # required wording follows the workflow rather than being fixed here.
        SMOKE=".github/workflows/cross-platform-test.yml"
        smoke_automatic=0
        if [[ -f "$SMOKE" ]] && grep -qE '^[[:space:]]*(push|schedule):' "$SMOKE"; then
            smoke_automatic=1
        fi

        # Read the doc as one line: the sentence that has to carry this wraps,
        # and a line-at-a-time grep splits "Windows coverage" from "dispatching
        # ... manually". The `[^.]` bounds below still keep a match inside one
        # sentence.
        install_flat="$(tr '\n' ' ' <"$INSTALL_DOC")"

        if [[ $smoke_automatic -eq 1 ]]; then
            # Match "post-merge" specifically. A looser pattern here matched
            # "does not provide. `pr-gate.yml`" by accident and let the branch
            # pass on a doc that claimed the opposite.
            grep -qEi 'windows[^.]{0,80}post-merge|post-merge[^.]{0,80}windows' "$INSTALL_DOC" ||
                fail "$PR_GATE has no windows-latest pull_request leg; $INSTALL_DOC must say the Windows leg is post-merge only"
        else
            printf '%s\n' "$install_flat" |
                grep -qEi 'windows[^.]{0,120}(manual|dispatch)|(manual|dispatch)[^.]{0,120}windows' ||
                fail "neither $PR_GATE nor $SMOKE runs windows-latest automatically; $INSTALL_DOC must say Windows coverage is manual-dispatch only"
            if printf '%s\n' "$install_flat" |
                grep -qEi 'windows[^.]{0,80}post-merge|post-merge[^.]{0,80}windows'; then
                fail "$INSTALL_DOC still claims post-merge Windows coverage, but $SMOKE has no push or schedule trigger"
            fi
        fi
    fi
fi

# (g) docs/INSTALL.md's blunt Windows line is intact -- no softening.
echo "--- INSTALL.md Windows line is not softened ---"
grep -qF 'Native Windows (PowerShell) is not supported' "$INSTALL_DOC" ||
    fail "$INSTALL_DOC no longer states plainly that native Windows (PowerShell) is not supported (#496)"

# (i) fish/fisher.json's fish_version and os fields must match the canonical
# matrix rather than being asserted separately, so the two cannot drift
# independently (2026-09-03 audit follow-up to #496). The minimum Fish
# version is derived from docs/INSTALL.md's Shells table row rather than
# hardcoded here.
echo "--- fish/fisher.json matches the canonical Fish-version and Windows policy ---"
FISHER_JSON="fish/fisher.json"
if [[ ! -f "$FISHER_JSON" ]]; then
    fail "$FISHER_JSON not found"
else
    fish_doc_version="$(grep -E '^\| *Fish *\|' "$INSTALL_DOC" | grep -oE '[0-9]+\.[0-9]+' | head -1)"
    if [[ -z "$fish_doc_version" ]]; then
        fail "could not find Fish's minimum version in $INSTALL_DOC's Shells table (expected a row like '| Fish | 3.6+ |')"
    else
        expected_fish_version=">=${fish_doc_version}.0"
        fisher_fish_version="$(sed -n 's/.*"fish_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$FISHER_JSON")"
        if [[ "$fisher_fish_version" != "$expected_fish_version" ]]; then
            fail "$FISHER_JSON declares fish_version \"$fisher_fish_version\", but $INSTALL_DOC's Shells table requires Fish $fish_doc_version+ (expected \"$expected_fish_version\")"
        fi
    fi

    fisher_os_line="$(grep -E '"os"[[:space:]]*:' "$FISHER_JSON")"
    if [[ -z "$fisher_os_line" ]]; then
        fail "$FISHER_JSON has no \"os\" field"
    elif grep -qi 'windows' <<<"$fisher_os_line"; then
        fail "$FISHER_JSON lists windows as a supported OS, but native Windows is CLI-only -- no shell integration ships for it (#496)"
    fi
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
