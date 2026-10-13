#!/usr/bin/env bash
# tests/bash/contributing_workflow_docs.test.bash
#
# CONTRIBUTING.md and the PR template only name commands that exist, keep off
# the private agent-tooling paths, and route security reports away from public
# issues.
#
# Derived rather than retyped: the private-path set is parsed out of
# tests/bash/agent_tooling_privacy_claims.test.bash's PRIVATE_PATHS line, and
# every script path, `just` recipe and `moon run gpy-agent:<task>` is extracted
# with a pattern and checked against the filesystem, the justfile and
# gpy-agent/moon.yml.
#
# Asserts:
#   (a) CONTRIBUTING.md contains no reference to a private path (PRIVATE_PATHS
#       plus the gitignored .claude/), except .raven/git-hooks/
#   (b) every `./scripts/*.sh` / `./install*.{sh,fish}` path named in
#       CONTRIBUTING.md or the PR template exists and is executable
#   (c) every `just <recipe>` named in CONTRIBUTING.md is a real recipe
#   (d) every `moon run gpy-agent:<task>` named in CONTRIBUTING.md is a real
#       task in gpy-agent/moon.yml
#   (e) the detector in (b)/(c)/(d) fires against planted fake commands
#   (f) .github/ISSUE_TEMPLATE/config.yml routes security reports to GitHub
#       private vulnerability reporting, and nothing under .github/ publishes
#       an email address

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

CONTRIBUTING="CONTRIBUTING.md"
PR_TEMPLATE=".github/PULL_REQUEST_TEMPLATE.md"
ISSUE_TEMPLATE_DIR=".github/ISSUE_TEMPLATE"
SIBLING_PRIVACY_TEST="tests/bash/agent_tooling_privacy_claims.test.bash"
JUSTFILE="justfile"
MOON_YML="gpy-agent/moon.yml"

[[ -f "$CONTRIBUTING" ]] || {
    fail "$CONTRIBUTING not found"
    test_finish
}
[[ -f "$PR_TEMPLATE" ]] || fail "$PR_TEMPLATE not found"

# --- derive the private-path set from the privacy test ----------------------
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
# .claude/ is gitignored rather than tracked, so the privacy test's list has
# no reason to carry it; CONTRIBUTING.md must still not point at it.
PRIVATE_PATHS+=(.claude)
echo "  private paths: ${PRIVATE_PATHS[*]}"

# --- (a) CONTRIBUTING.md references no private path -------------------------
echo "--- CONTRIBUTING.md has no reference to a private path ---"
scan_private_refs() {
    local file="$1" root="$2" hits
    if [[ "$root" == ".raven" ]]; then
        # .raven/git-hooks/ is the one public part of .raven/: CONTRIBUTING.md
        # documents the git hooks. Every other .raven/... reference is private.
        hits="$(grep -noE '\.raven(/[A-Za-z0-9_.-]*)?' "$file" | grep -vE ':\.raven/git-hooks(/|\b)' || true)"
    else
        hits="$(grep -noF "$root" "$file" || true)"
    fi
    printf '%s' "$hits"
}
for private in "${PRIVATE_PATHS[@]}"; do
    hits="$(scan_private_refs "$CONTRIBUTING" "$private")"
    if [[ -n "$hits" ]]; then
        fail "$CONTRIBUTING references private path '$private', which is excluded from the public repository"
        printf '%s\n' "$hits" | sed 's/^/  /'
    fi
done

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
# Without this, (b)/(c)/(d) could pass vacuously against a detector that
# extracts nothing.
echo "--- extractors must flag planted fake commands ---"
PLANT="$(mktemp)"
trap 'rm -f "$PLANT"' EXIT
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

# --- (f) security reports go to private vulnerability reporting -------------
echo "--- security reports route to private vulnerability reporting ---"
CONFIG_YML="$ISSUE_TEMPLATE_DIR/config.yml"
if [[ ! -f "$CONFIG_YML" ]]; then
    fail "$CONFIG_YML not found; security reports need a routing entry away from public issues"
else
    grep -qF 'https://github.com/jpease/gpy/security/advisories/new' "$CONFIG_YML" ||
        fail "$CONFIG_YML does not link the GitHub private vulnerability reporting URL"
fi
while IFS= read -r -d '' target; do
    if grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' "$target"; then
        fail "$target publishes an email address; security reports route to GitHub private vulnerability reporting only"
    fi
done < <(find .github -type f -print0)

test_finish "CONTRIBUTING.md and .github/ name real commands and route security privately"
