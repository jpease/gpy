#!/usr/bin/env bash
# tests/bash/gengo_gix_free.test.bash
#
# Regression guard for #520 (epic #519, step 1 of 6), rewritten for #523
# (step 5), which swapped the detector over and changed this file's premise.
#
# GPY retired gix as its git backend in 0efbaa24, replacing it with a
# `git status --porcelain=v2 --branch` subprocess (see
# docs/dev/adr/adr-0003-gix-for-git-operations.md, now superseded). #520 added
# `gengo` alongside `gengo-language` with default-features = false, because
# gengo's defaults ("git", "max-performance-safe") both gate on `dep:gix` and
# would have pulled the retired backend back in.
#
# #523 dropped the `gengo` crate entirely. Its `Gengo::analyze` reads every
# file in the tree -- 546 ms on a 20k-file tree against a 200 ms hard max --
# so GPY owns its own `ignore::WalkBuilder` walk in
# gpy-agent/src/language/detector.rs and needs only `gengo-language`'s matcher
# tables. That removes the whole feature-flag hazard #520 was written to
# police, but not the underlying one: any future dependency that resolves gix
# still undoes 0efbaa24, and assertion (d) below is the only thing in the tree
# that would notice.
#
# Asserts, against the real files (not a description of them):
#   (a) gengo-language is a direct dependency of gpy-agent
#   (b) gengo is NOT a dependency (dropped in #523)
#   (c) hyperpolyglot is NOT a dependency (replaced in #523)
#   (d) gpy-agent/Cargo.lock -- the real, resolved dependency tree, not a
#       description of it -- resolves no package named gix or gix-*

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
LOCK="gpy-agent/Cargo.lock"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

for f in "$MANIFEST" "$LOCK"; do
    if [[ ! -f "$f" ]]; then
        fail "$f not found"
        echo "$failures assertion(s) failed"
        exit 1
    fi
done

echo "--- gengo-language is a direct dependency ---"
if ! grep -qE '^gengo-language[[:space:]]*=' "$MANIFEST"; then
    fail "$MANIFEST declares no 'gengo-language' dependency; it supplies the matcher tables src/language/detector.rs runs on (#523)"
fi

echo "--- the gengo wrapper crate is not a dependency ---"
if grep -qE '^gengo[[:space:]]*=' "$MANIFEST"; then
    fail "$MANIFEST declares a 'gengo' dependency; #523 dropped it because Gengo::analyze reads every file in the tree (546 ms at 20k files vs a 200 ms hard max) and its default features gate on dep:gix"
fi
if grep -qE '^name = "gengo"$' "$LOCK"; then
    fail "$LOCK resolves the 'gengo' package; #523 dropped it (see above)"
fi

echo "--- hyperpolyglot is not a dependency ---"
if grep -qE '^hyperpolyglot[[:space:]]*=' "$MANIFEST"; then
    fail "$MANIFEST declares a 'hyperpolyglot' dependency; #523 replaced it with gengo-language"
fi
if grep -qE '^name = "hyperpolyglot"$' "$LOCK"; then
    fail "$LOCK resolves the 'hyperpolyglot' package; #523 replaced it with gengo-language"
fi

echo "--- the resolved dependency tree carries no gix crate ---"
if grep -qE '^name = "gix(-[a-zA-Z0-9_-]+)?"' "$LOCK"; then
    fail "$LOCK resolves one or more gix/gix-* packages; a dependency change has reintroduced the git backend GPY retired in 0efbaa24 (#520)"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
