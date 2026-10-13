#!/usr/bin/env bash
# tests/bash/gengo_gix_free.test.bash
#
# GPY does not use gix: git status comes from a
# `git status --porcelain=v2 --branch` subprocess
# (docs/dev/adr/adr-0003-gix-for-git-operations.md is superseded). Any
# dependency that resolves gix pulls the retired backend back in.
#
# The `gengo` crate is not a dependency: its `Gengo::analyze` reads every
# file in the tree -- 546 ms on a 20k-file tree against a 200 ms hard max --
# and its default features ("git", "max-performance-safe") both gate on
# `dep:gix`. GPY owns its own `ignore::WalkBuilder` walk in
# gpy-agent/src/language/detector.rs and needs only `gengo-language`'s matcher
# tables. Assertion (d) below is the only thing in the tree that would notice
# a future dependency resolving gix.
#
# Asserts, against the real files (not a description of them):
#   (a) gengo-language is a direct dependency of gpy-agent
#   (b) gengo is NOT a dependency
#   (c) hyperpolyglot is NOT a dependency (gengo-language replaces it)
#   (d) gpy-agent/Cargo.lock -- the real, resolved dependency tree, not a
#       description of it -- resolves no package named gix or gix-*

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
LOCK="gpy-agent/Cargo.lock"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

for f in "$MANIFEST" "$LOCK"; do
    if [[ ! -f "$f" ]]; then
        fail "$f not found"
        test_finish
    fi
done

echo "--- gengo-language is a direct dependency ---"
if ! grep -qE '^gengo-language[[:space:]]*=' "$MANIFEST"; then
    fail "$MANIFEST declares no 'gengo-language' dependency; it supplies the matcher tables src/language/detector.rs runs on"
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
    fail "$LOCK resolves one or more gix/gix-* packages; a dependency change has reintroduced the git backend GPY retired in 0efbaa24"
fi

test_finish
