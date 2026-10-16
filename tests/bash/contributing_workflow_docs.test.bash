#!/usr/bin/env bash
# tests/bash/contributing_workflow_docs.test.bash
#
# CONTRIBUTING.md and the PR template only name commands that exist.
#
# Asserts:
#   (b) every `./scripts/*.sh` / `./install*.{sh,fish}` path named in
#       CONTRIBUTING.md or the PR template exists and is executable
#   (c) every `just <recipe>` named in CONTRIBUTING.md is a real recipe
#   (e) the detectors in (b)/(c) fire against planted fake commands

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

CONTRIBUTING="CONTRIBUTING.md"
PR_TEMPLATE=".github/PULL_REQUEST_TEMPLATE.md"
JUSTFILE="justfile"

[[ -f "$CONTRIBUTING" ]] || {
    fail "$CONTRIBUTING not found"
    test_finish
}
[[ -f "$PR_TEMPLATE" ]] || fail "$PR_TEMPLATE not found"

# Shared by (b)/(c) and the planted-fake check in (e).
extract_script_paths() {
    grep -ohE '\./(scripts/[A-Za-z0-9_./-]+\.sh|install[A-Za-z0-9_.-]*\.(sh|fish))' "$@" 2>/dev/null | sort -u
}
extract_just_recipes() {
    grep -ohE '(^|[^A-Za-z0-9_-])just [a-z][a-z0-9_-]*' "$@" 2>/dev/null |
        sed -E 's/^.*just //' | sort -u
}

script_exists() {
    local rel="${1#./}"
    [[ -f "$rel" && -x "$rel" ]]
}
just_recipe_exists() {
    grep -qE "^${1}([[:space:]*]|:)" "$JUSTFILE" 2>/dev/null
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

# --- (e) the extractors actually fire on planted fake commands --------------
# Without this, (b)/(c) could pass vacuously against a detector that extracts nothing.
echo "--- extractors must flag planted fake commands ---"
PLANT="$(mktemp)"
trap 'rm -f "$PLANT"' EXIT
cat >"$PLANT" <<'EOF'
Run the tests:

```bash
./scripts/does-not-exist.sh
./install-nonexistent.fish
just totally-fake-recipe
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

# A real command must NOT be flagged, or the detector is simply "always fail".
cat >"$PLANT" <<'EOF'
```bash
./scripts/quality-check.sh
just check
```
EOF
while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    script_exists "$path" || fail "script_exists incorrectly flagged the real path '$path'"
done < <(extract_script_paths "$PLANT")
just_recipe_exists "check" || fail "just_recipe_exists incorrectly flagged the real recipe 'check'"

test_finish "CONTRIBUTING.md and the PR template name real commands"
