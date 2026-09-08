#!/usr/bin/env bash
# tests/bash/crate_publish_boundary.test.bash
#
# Locks in the distribution decision from #502: gpy-agent is never published to
# crates.io, and its library surface carries no semver guarantee.
#
# `cargo install gpy-agent` cannot deliver a working GPY. It places the two
# binaries and nothing else -- no fish/core/, fish/segments/ or fish/conf.d/,
# which is what the prompt actually renders from. Every place that offered it
# was offering a broken install, so the string is banned outright rather than
# deprecated.
#
# `brew install gpy` is banned in its bare form because it implies homebrew-core.
# The supported future spelling is a personal tap (`brew install jpease/tap/gpy`),
# which this assertion still allows.
#
# This covers the installer and documentation surface.
# tests/bash/release_packaging.test.bash already bans the same strings from
# generated release notes; the two do not overlap.
#
# Asserts:
#   (a) publish = false is set in gpy-agent/Cargo.toml
#   (b) no `cargo install gpy-agent` in the installers, update script, docs,
#       crate README, crate root docs, or the PR template
#   (c) no bare `brew install gpy` in the installers, docs, or crate README
#   (d) the crate README no longer advertises a crates.io/docs.rs publication
#       or a [dependencies] snippet
#   (e) both the crate README and lib.rs state the unsupported-library boundary

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
CRATE_README="gpy-agent/README.md"
CRATE_LIB="gpy-agent/src/lib.rs"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Files that must never offer `cargo install gpy-agent`. Docs are globbed so a
# new page is covered without editing this list.
cargo_scan=(install.sh install-oneline.sh scripts/update.fish "$MANIFEST"
    "$CRATE_README" "$CRATE_LIB" .github/PULL_REQUEST_TEMPLATE.md)
while IFS= read -r doc; do
    cargo_scan+=("$doc")
done < <(find docs -name '*.md' -type f | sort)

# Files that must never offer the core-implying `brew install gpy`.
brew_scan=(install.sh install-oneline.sh "$CRATE_README")
while IFS= read -r doc; do
    brew_scan+=("$doc")
done < <(find docs -name '*.md' -type f | sort)

# (a) the manifest opts out of publication.
echo "--- gpy-agent is marked unpublishable ---"
if [[ ! -f "$MANIFEST" ]]; then
    fail "$MANIFEST not found"
elif ! grep -qE '^[[:space:]]*publish[[:space:]]*=[[:space:]]*false' "$MANIFEST"; then
    fail "$MANIFEST does not set publish = false (#502)"
fi

# (b) the dead cargo channel is gone.
echo "--- no cargo install channel ---"
for target in "${cargo_scan[@]}"; do
    [[ -f "$target" ]] || continue
    if grep -nF 'cargo install gpy-agent' "$target"; then
        fail "$target still offers 'cargo install gpy-agent'; the crate is never published (#502)"
    fi
done

# (c) no bare Homebrew core form. A tap-qualified name is fine.
echo "--- no homebrew-core install form ---"
for target in "${brew_scan[@]}"; do
    [[ -f "$target" ]] || continue
    if grep -nF 'brew install gpy' "$target" | grep -vF 'brew install jpease/'; then
        fail "$target uses the bare 'brew install gpy' form, which implies homebrew-core"
    fi
done

# (d) the crate README no longer reads like a published library.
echo "--- crate README does not advertise a registry publication ---"
if [[ ! -f "$CRATE_README" ]]; then
    fail "$CRATE_README not found"
else
    for registry in 'crates.io/crates/gpy-agent' 'docs.rs/gpy-agent' 'img.shields.io/crates'; do
        if grep -nF "$registry" "$CRATE_README"; then
            fail "$CRATE_README still links $registry for an unpublished crate"
        fi
    done
    if grep -qE '^gpy-agent[[:space:]]*=[[:space:]]*"' "$CRATE_README"; then
        fail "$CRATE_README still shows a [dependencies] snippet for an unpublished crate"
    fi
fi

# (e) the boundary is stated where a would-be library consumer would look.
echo "--- unsupported-library boundary is documented ---"
for target in "$CRATE_README" "$CRATE_LIB"; do
    [[ -f "$target" ]] || {
        fail "$target not found"
        continue
    }
    grep -qiF 'not published to crates.io' "$target" ||
        fail "$target does not state that the crate is not published to crates.io (#502)"
    grep -qi 'semver' "$target" ||
        fail "$target does not state that the library surface carries no semver guarantee (#502)"
done

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
