#!/usr/bin/env bash
# tests/bash/contributing_workflow_docs.test.bash
#
# Locks in #501: CONTRIBUTING.md, the PR template, and the issue templates
# refreshed for a project that just went public.
#
# CONTRIBUTING.md used to be 1.3K and pointed at AGENTS.md ("For faster local
# iteration, see AGENTS.md") for the commands a contributor actually needs.
# AGENTS.md is not published (#504), so that link led nowhere for anyone
# outside the maintainer's own checkout. The PR template separately named
# commands that never existed in this repository (`fish test.fish all` --
# there has never been a `test.fish` at the repo root; the real runner is
# `./scripts/test_fish.sh`) or that are stale (`cargo test --workspace
# --all-features` where the project standard is `cargo nextest run`), and
# offered install channels this repository elsewhere refuses to advertise
# (a bare `brew install gpy`, `fisher install gpy`) plus a Windows platform
# checkbox that contradicts the #496 support matrix.
#
# The fix makes CONTRIBUTING.md self-contained -- it carries the quality-gate
# and development commands inline rather than linking to the private
# AGENTS.md -- and replaces the four fabricated/dead PR-template lines with
# commands and channels that are real. It also adds the bug-report issue
# template #501 says the realistic inflow needs (shell+version, OS,
# `gpy --version`, agent state) and a `config.yml` that routes security
# reports to GitHub private vulnerability reporting instead of a public issue.
#
# The house rule from #495/#504's sibling tests: derive the thing being
# checked from the repository rather than re-typing it as a second copy that
# can drift. Two things are derived here rather than hardcoded:
#   * the private-path set is read out of
#     tests/bash/agent_tooling_privacy_claims.test.bash's own PRIVATE_PATHS
#     line, not retyped, so the two tests cannot silently disagree about what
#     "private" means;
#   * every script path, `just` recipe, and `moon run gpy-agent:<task>` named
#     in CONTRIBUTING.md or the PR template is extracted with a pattern (not
#     matched against one hardcoded known-bad string) and checked against the
#     filesystem, the justfile, and gpy-agent/moon.yml -- the failure mode
#     this issue fixes is exactly a command that looks plausible and does not
#     exist.
#
# Deliberately NOT asserted:
#   * CODE_OF_CONDUCT.md's content -- out of scope for #501, explicitly
#     reserved for the maintainer per the issue.
#   * SECURITY.md's content -- rewritten by #497 and already covered by
#     tests/bash/security_trust_boundary_claims.test.bash; this file only
#     checks that nothing under .github/ contradicts it (no email, the
#     advisories URL is present).
#   * Non-operational install channels in README.md/install.sh/docs -- that
#     is tests/bash/release_version_claims.test.bash's job (#495). This file
#     scans only .github/, which that test does not cover.
#
# Asserts:
#   (a) CONTRIBUTING.md contains no reference to a private path (derived from
#       PRIVATE_PATHS in agent_tooling_privacy_claims.test.bash, plus .claude/
#       which that list omits because it is gitignored rather than tracked),
#       except .raven/git-hooks/
#   (b) every `./scripts/*.sh` / `./install*.{sh,fish}` path named in
#       CONTRIBUTING.md or the PR template exists and is executable
#   (c) every `just <recipe>` invocation named in CONTRIBUTING.md is a real
#       top-level recipe in justfile
#   (d) every `moon run gpy-agent:<task>` mentioned in CONTRIBUTING.md is a
#       real task in gpy-agent/moon.yml
#   (e) the detector in (b)/(c)/(d) actually fires against planted fake
#       commands, so it is not vacuously passing
#   (f) CONTRIBUTING.md states review is best-effort with no fixed timeline,
#       and asks for an issue before a large change
#   (g) the PR template no longer offers the dead/banned checkboxes
#       (`fish test.fish all`, `cargo test --workspace --all-features`,
#       Fisher, a bare `brew install gpy`, a Windows platform checkbox)
#   (h) exactly one bug-report issue template exists under
#       .github/ISSUE_TEMPLATE/, and it captures shell+version, OS,
#       `gpy --version`, and agent state
#   (i) .github/ISSUE_TEMPLATE/config.yml routes security reports to GitHub
#       private vulnerability reporting
#   (j) nothing under .github/ offers a non-operational install channel or
#       publishes an email address for security reports

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

CONTRIBUTING="CONTRIBUTING.md"
PR_TEMPLATE=".github/PULL_REQUEST_TEMPLATE.md"
ISSUE_TEMPLATE_DIR=".github/ISSUE_TEMPLATE"
SIBLING_PRIVACY_TEST="tests/bash/agent_tooling_privacy_claims.test.bash"
JUSTFILE="justfile"
MOON_YML="gpy-agent/moon.yml"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

[[ -f "$CONTRIBUTING" ]] || {
    fail "$CONTRIBUTING not found"
    echo "$failures assertion(s) failed"
    exit 1
}
[[ -f "$PR_TEMPLATE" ]] || fail "$PR_TEMPLATE not found"

# --- derive the private-path set from the sibling #504 test -----------------
#
# Not retyped: parsed out of agent_tooling_privacy_claims.test.bash's own
# PRIVATE_PATHS=(...) line so the two tests share one definition of private.
echo "--- deriving the private-path set ---"
if [[ ! -f "$SIBLING_PRIVACY_TEST" ]]; then
    fail "$SIBLING_PRIVACY_TEST not found; cannot derive the private-path set"
    PRIVATE_PATHS=()
else
    private_raw="$(grep -m1 '^PRIVATE_PATHS=(' "$SIBLING_PRIVACY_TEST" | sed -E 's/^PRIVATE_PATHS=\(([^)]*)\).*/\1/')"
    if [[ -z "$private_raw" ]]; then
        fail "could not parse PRIVATE_PATHS out of $SIBLING_PRIVACY_TEST"
        PRIVATE_PATHS=()
    else
        # shellcheck disable=SC2206  # deliberate word split of a known-shaped list
        PRIVATE_PATHS=($private_raw)
    fi
fi
# .claude/ is not in the derived set (it is gitignored rather than tracked, so
# #504's detector has nothing to check it against) but #501's own acceptance
# criteria names it explicitly, so it is added here rather than silently
# relying on the sibling test to cover it.
PRIVATE_PATHS+=(.claude)
echo "  private paths: ${PRIVATE_PATHS[*]}"

# --- (a) CONTRIBUTING.md references no private path -------------------------
echo "--- CONTRIBUTING.md has no reference to a private path ---"
scan_private_refs() {
    local file="$1" root="$2" hits
    if [[ "$root" == ".raven" ]]; then
        # .raven/git-hooks/ is the one part of .raven/ that stays public
        # (AGENTS.md's ai-tooling-private-in-public-repos decision), and
        # CONTRIBUTING.md is expected to reference it for the git-hooks
        # section. Every other .raven/... reference is still private.
        hits="$(grep -noE '\.raven(/[A-Za-z0-9_.-]*)?' "$file" | grep -vE ':\.raven/git-hooks(/|\b)' || true)"
    else
        hits="$(grep -noF "$root" "$file" || true)"
    fi
    printf '%s' "$hits"
}
for private in "${PRIVATE_PATHS[@]}"; do
    hits="$(scan_private_refs "$CONTRIBUTING" "$private")"
    if [[ -n "$hits" ]]; then
        fail "$CONTRIBUTING references private path '$private', which is excluded from the public repository (#504); CONTRIBUTING.md must be self-contained (#501)"
        printf '%s\n' "$hits" | sed 's/^/  /'
    fi
done
# The exception itself must actually be exercised, or (a) is not proven to
# tell "references .raven/git-hooks/" apart from "references .raven/".
grep -qF '.raven/git-hooks' "$CONTRIBUTING" ||
    fail "$CONTRIBUTING does not mention .raven/git-hooks/; the git-hooks exception above is untested against real content"

# --- command/path extraction helpers -----------------------------------------
#
# Shared by (b)/(c)/(d) and by the mutation-detector proof in (e), so the
# same extraction logic is what both the real files and the planted fakes
# run through.
extract_script_paths() {
    grep -ohE '\./(scripts/[A-Za-z0-9_./-]+\.sh|install[A-Za-z0-9_.-]*\.(sh|fish))' "$@" 2>/dev/null | sort -u
}
extract_just_recipes() {
    grep -ohE '(^|[^A-Za-z0-9_-])just [a-z][a-z0-9_-]*' "$@" 2>/dev/null |
        sed -E 's/^.*just //' | sort -u
}
extract_moon_tasks() {
    grep -ohE 'moon run gpy-agent:[A-Za-z0-9_-]+' "$@" 2>/dev/null |
        sed -E 's/^.*gpy-agent://' | sort -u
}

script_exists() {
    local rel="${1#./}"
    [[ -f "$rel" && -x "$rel" ]]
}
just_recipe_exists() {
    grep -qE "^${1}([[:space:]*]|:)" "$JUSTFILE" 2>/dev/null
}
moon_task_exists() {
    grep -qE "^  ${1}:" "$MOON_YML" 2>/dev/null
}

# --- (b) every script path named actually exists and is executable ----------
echo "--- script paths named in CONTRIBUTING.md / PR template exist ---"
while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    script_exists "$path" ||
        fail "'$path' is named in $CONTRIBUTING or $PR_TEMPLATE but does not exist (or is not executable)"
done < <(extract_script_paths "$CONTRIBUTING" "$PR_TEMPLATE")

# --- (c) every `just <recipe>` invocation is a real recipe -------------------
echo "--- just recipes named in CONTRIBUTING.md are real ---"
while IFS= read -r recipe; do
    [[ -n "$recipe" ]] || continue
    just_recipe_exists "$recipe" ||
        fail "'just $recipe' is named in $CONTRIBUTING but '$recipe' is not a recipe in $JUSTFILE"
done < <(extract_just_recipes "$CONTRIBUTING")

# --- (d) every `moon run gpy-agent:<task>` is a real task -------------------
echo "--- moon tasks named in CONTRIBUTING.md are real ---"
while IFS= read -r task; do
    [[ -n "$task" ]] || continue
    moon_task_exists "$task" ||
        fail "'moon run gpy-agent:$task' is named in $CONTRIBUTING but '$task' is not a task in $MOON_YML"
done < <(extract_moon_tasks "$CONTRIBUTING")

# --- (e) the extractors actually fire on planted fake commands --------------
#
# Without this, (b)/(c)/(d) are three assertions that have never been
# observed to fail -- exactly the shape of bug #501 fixes (a template naming
# `fish test.fish all`, which parses as a plausible instruction and was never
# checked against anything).
echo "--- extractors must flag planted fake commands ---"
PLANT="$(mktemp)"
cleanup_plant() { rm -f "$PLANT"; }
trap cleanup_plant EXIT
cat >"$PLANT" <<'EOF'
Run the tests:

```bash
./scripts/does-not-exist.sh
./install-nonexistent.fish
just totally-fake-recipe
moon run gpy-agent:not-a-real-task
```
EOF

fake_script="$(extract_script_paths "$PLANT")"
[[ -n "$fake_script" ]] || fail "extract_script_paths missed a planted fake script path"
if [[ -n "$fake_script" ]]; then
    while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        script_exists "$path" &&
            fail "script_exists incorrectly reported the planted fake path '$path' as real"
    done <<<"$fake_script"
fi

fake_recipe="$(extract_just_recipes "$PLANT")"
[[ "$fake_recipe" == "totally-fake-recipe" ]] || fail "extract_just_recipes missed the planted fake recipe (got '$fake_recipe')"
just_recipe_exists "totally-fake-recipe" && fail "just_recipe_exists incorrectly reported the planted fake recipe as real"

fake_task="$(extract_moon_tasks "$PLANT")"
[[ "$fake_task" == "not-a-real-task" ]] || fail "extract_moon_tasks missed the planted fake moon task (got '$fake_task')"
moon_task_exists "not-a-real-task" && fail "moon_task_exists incorrectly reported the planted fake task as real"

# A real command must NOT be flagged, or the detector is simply "always fail".
cat >"$PLANT" <<'EOF'
```bash
./scripts/quality-check.sh
just check
moon run gpy-agent:build
```
EOF
while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    script_exists "$path" || fail "script_exists incorrectly flagged the real path '$path'"
done < <(extract_script_paths "$PLANT")
just_recipe_exists "check" || fail "just_recipe_exists incorrectly flagged the real recipe 'check'"
moon_task_exists "build" || fail "moon_task_exists incorrectly flagged the real task 'build'"

# --- (f) review expectations are stated plainly ------------------------------
echo "--- CONTRIBUTING.md states best-effort review and asks for an issue first ---"
grep -qiE 'best-effort' "$CONTRIBUTING" ||
    fail "$CONTRIBUTING does not say review is best-effort (#501)"
grep -qiE 'no fixed timeline|no timeline' "$CONTRIBUTING" ||
    fail "$CONTRIBUTING does not say review carries no fixed timeline (#501)"
grep -qiE 'open an issue first|open an issue.*before' "$CONTRIBUTING" ||
    fail "$CONTRIBUTING does not ask contributors to open an issue before a large change (#501)"

# --- (g) the PR template drops the dead/banned lines -------------------------
echo "--- PR template no longer offers dead or banned commands/channels ---"
if [[ -f "$PR_TEMPLATE" ]]; then
    grep -qF 'fish test.fish all' "$PR_TEMPLATE" &&
        fail "$PR_TEMPLATE still names 'fish test.fish all', which has never existed in this repository (#501)"
    grep -qF 'cargo test --workspace --all-features' "$PR_TEMPLATE" &&
        fail "$PR_TEMPLATE still names 'cargo test --workspace --all-features'; the project standard is 'cargo nextest run' (#501)"
    grep -qiF 'fisher install' "$PR_TEMPLATE" &&
        fail "$PR_TEMPLATE still offers the Fisher plugin channel, which ships no agent binary (#495/#501)"
    grep -qE 'brew install [^ /]*gpy' "$PR_TEMPLATE" &&
        fail "$PR_TEMPLATE still offers a bare 'brew install gpy'; no tap exists and the formula's sha256 is a placeholder (#515/#501)"
    grep -qiE 'Windows \(x86_64\)' "$PR_TEMPLATE" &&
        fail "$PR_TEMPLATE still carries a Windows platform checkbox, contradicting the #496 support matrix"
fi

# --- (h) exactly one bug-report template, with the required fields ----------
echo "--- one bug-report issue template with the required fields ---"
if [[ ! -d "$ISSUE_TEMPLATE_DIR" ]]; then
    fail "$ISSUE_TEMPLATE_DIR does not exist; #501 requires one bug-report issue template"
else
    templates=()
    while IFS= read -r -d '' f; do
        base="$(basename "$f")"
        [[ "$base" == "config.yml" ]] && continue
        templates+=("$f")
    done < <(find "$ISSUE_TEMPLATE_DIR" -maxdepth 1 -type f \( -name '*.yml' -o -name '*.yaml' -o -name '*.md' \) -print0)

    if [[ "${#templates[@]}" -ne 1 ]]; then
        fail "expected exactly one issue template under $ISSUE_TEMPLATE_DIR, found ${#templates[@]} (${templates[*]}); #501 settled on one bug-report template, not a multi-template suite"
    else
        tmpl="${templates[0]}"
        echo "  template: $tmpl"
        grep -qiE 'bug' "$tmpl" || fail "$tmpl does not look like a bug-report template"
        grep -qiE 'shell' "$tmpl" || fail "$tmpl does not ask for the shell"
        grep -qiE 'version' "$tmpl" || fail "$tmpl does not ask for a version"
        grep -qiE '\bos\b|operating system' "$tmpl" || fail "$tmpl does not ask for the operating system"
        grep -qF 'gpy --version' "$tmpl" || fail "$tmpl does not ask for 'gpy --version' output"
        grep -qiE 'agent state|gpy status' "$tmpl" || fail "$tmpl does not ask for agent state (gpy status)"

        # A YAML issue form was chosen over Markdown precisely because GitHub
        # enforces `required: true` before submission. Asserting only that the
        # words appear somewhere would pass a form where every field is
        # optional -- which is exactly the Markdown behaviour the format was
        # chosen to avoid. Each diagnostic field must actually be required.
        if [[ "$tmpl" == *.yml || "$tmpl" == *.yaml ]]; then
            for want in 'gpy --version' 'Shell and version' 'Operating system' 'Agent state'; do
                if ! awk -v want="$want" '
                    index($0, "label: " want) { hit = NR }
                    hit && NR > hit && NR <= hit + 8 && /required:[[:space:]]*true/ { found = 1 }
                    END { exit(found ? 0 : 1) }
                ' "$tmpl"; then
                    fail "$tmpl field '$want' is not marked 'required: true'; a form whose fields are optional collects no more than a Markdown template"
                fi
            done
        fi
    fi
fi

# --- (k) bug_report.yml emits the repository-standard title and labels ------
#
# #501 reopened on 2026-09-04 (2026-09-03 audit): the form still emitted the
# placeholder title prefix "[Bug]: " and the legacy "bug" label instead of
# this repository's actual conventions (`gh label list`): issue titles use a
# Bug:/Task:/Docs: prefix, and labels are type:<x> plus priority:P[0-3], not
# the bare legacy "bug" label. Checked directly against bug_report.yml by
# path rather than through the generic "exactly one template" discovery in
# (h), so this keeps working even if a second template is added later and
# (h)'s discovery logic has to change.
echo "--- bug_report.yml emits the standard title and required type/priority labels ---"
BUG_REPORT_TMPL="$ISSUE_TEMPLATE_DIR/bug_report.yml"
if [[ ! -f "$BUG_REPORT_TMPL" ]]; then
    fail "$BUG_REPORT_TMPL not found"
else
    title_line="$(grep -m1 -E '^title:' "$BUG_REPORT_TMPL")"
    if [[ "$title_line" != 'title: "Bug: "' ]]; then
        fail "$BUG_REPORT_TMPL title is '$title_line', expected title: \"Bug: \" (repository issue titles use a Bug:/Task:/Docs: prefix, not [Bug]:)"
    fi

    labels_line="$(grep -m1 -E '^labels:' "$BUG_REPORT_TMPL")"
    if [[ -z "$labels_line" ]]; then
        fail "$BUG_REPORT_TMPL has no top-level 'labels:' line"
    else
        echo "$labels_line" | grep -qF '"type:bug"' ||
            fail "$BUG_REPORT_TMPL labels do not include \"type:bug\" (repository standard per 'gh label list'; got: $labels_line)"

        priority_count="$(grep -oE '"priority:P[0-3]"' <<<"$labels_line" | wc -l | tr -d ' ')"
        if [[ "$priority_count" -ne 1 ]]; then
            fail "$BUG_REPORT_TMPL labels contain $priority_count priority:P[0-3] entries, expected exactly one (got: $labels_line)"
        fi

        echo "$labels_line" | grep -qE '"bug"' &&
            fail "$BUG_REPORT_TMPL labels still include the legacy bare \"bug\" label; the repository standard is type:/priority: labels (got: $labels_line)"
    fi
fi

# --- (i) security routes to GitHub private vulnerability reporting -----------
echo "--- issue-template config routes security reports off-platform ---"
CONFIG_YML="$ISSUE_TEMPLATE_DIR/config.yml"
if [[ ! -f "$CONFIG_YML" ]]; then
    fail "$CONFIG_YML not found; security reports need a routing entry away from public issues (#497/#501)"
else
    grep -qF 'https://github.com/jpease/gpy/security/advisories/new' "$CONFIG_YML" ||
        fail "$CONFIG_YML does not link the GitHub private vulnerability reporting URL"
    grep -qiE 'security' "$CONFIG_YML" || fail "$CONFIG_YML has no security-labeled entry"
fi

# --- (j) nothing under .github/ offers a dead channel or an email -----------
echo "--- .github/ offers no non-operational channel and no email ---"
github_files=()
while IFS= read -r -d '' f; do
    github_files+=("$f")
done < <(find .github -type f -print0)

channel_patterns=(
    "Homebrew|brew (install|tap) [^ ]*(gpy|jpease)"
    "crates.io|cargo install gpy"
    "Debian|apt(-get)? install [^ ]*gpy|dpkg -i"
    "Arch|pacman -S [^ ]*gpy|yay -S [^ ]*gpy|makepkg|PKGBUILD"
    "Fisher|fisher install"
)
for target in "${github_files[@]}"; do
    for entry in "${channel_patterns[@]}"; do
        label="${entry%%|*}"
        pattern="${entry#*|}"
        if grep -nE "$pattern" "$target"; then
            fail "$target advertises the $label channel, which is not operational (#495/#501)"
        fi
    done
    if grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' "$target"; then
        fail "$target publishes an email address; security reports route to GitHub private vulnerability reporting only (#497/#501)"
    fi
done

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
