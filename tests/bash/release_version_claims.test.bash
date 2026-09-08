#!/usr/bin/env bash
# tests/bash/release_version_claims.test.bash
#
# Locks in the release-versioning and distribution decisions from #495.
#
# GPY has never been released: `git tag` is empty. The repository nevertheless
# carried two version histories -- a phantom `## [0.1.1] - 2025-10-26` sitting
# above `## [0.1.0] - TBD`, with a large `## [Unreleased]` body above both --
# while `gpy-agent/Cargo.toml` and `fish/fisher.json` both said 0.1.0. The
# first public release is a single v0.1.0, so exactly one version may be
# declared anywhere, and every declaration has to agree.
#
# The distribution half is the same discipline applied to install channels: an
# instruction for a channel that does not work is a bug report waiting to be
# filed. Homebrew has a formula with a placeholder checksum and no tap, no
# Debian or Arch package is built, the crate sets `publish = false` (#502), and
# the Fisher plugin ships no agent binary. None of those may be offered to a
# user by an installer or a doc page.
#
# Deliberately NOT scanned for channel strings:
#   * CHANGELOG.md -- it records that these channels were REMOVED (#492); the
#     history has to keep naming them.
#   * scripts/release-notes.sh -- its channel list is settled by #492 and is
#     already asserted by tests/bash/release_packaging.test.bash, which bans
#     the `.deb`/PKGBUILD/Homebrew/cargo blocks from generated notes. The two
#     tests do not overlap.
#   * tests/ -- test files name the banned strings in order to ban them.
#
# Asserts:
#   (a) the CHANGELOG declares exactly one released version and no phantom
#       [0.1.1] heading survives
#   (b) Cargo.toml, fisher.json, the CHANGELOG, and Formula/gpy.rb's tag URL
#       all agree on that one version
#   (c) every released CHANGELOG heading has a link reference and no link
#       reference points at a version with no section
#   (d) the release date is an ISO date or the literal YYYY-MM-DD placeholder
#   (e) scripts/release-notes.sh renders non-empty notes for that version
#   (f) no installer or doc advertises a non-operational channel
#   (g) Formula/gpy.rb still declares itself unfinished, and says so in a way
#       that names the issue that finishes it
#   (h) SECURITY.md's supported-versions table names the released minor line
#       and nothing else
#   (i) docs/dev/releasing.md exists and the version-bearing files it
#       documents are exactly the ones release.yml's validate job compares
#   (j) every install channel README/INSTALL advertise has an executing test,
#       and the from-source channel names install-dev.fish, not install.sh

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

CHANGELOG="CHANGELOG.md"
MANIFEST="gpy-agent/Cargo.toml"
FISHER="fish/fisher.json"
FORMULA="Formula/gpy.rb"
SECURITY="SECURITY.md"
RELEASE_DOC="docs/dev/releasing.md"
RELEASE_WORKFLOW=".github/workflows/release.yml"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# --- (a) exactly one released version in the CHANGELOG ----------------------
echo "--- CHANGELOG declares exactly one released version ---"
if [[ ! -f "$CHANGELOG" ]]; then
    fail "$CHANGELOG not found"
    echo "$failures assertion(s) failed"
    exit 1
fi

if grep -nE '^## \[0\.1\.1\]' "$CHANGELOG"; then
    fail "$CHANGELOG still has a [0.1.1] section; 0.1.1 was never released and folds into 0.1.0 (#495)"
fi

release_headings="$(grep -E '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' "$CHANGELOG" || true)"
heading_count="$(printf '%s' "$release_headings" | grep -c . || true)"
if [[ "$heading_count" -ne 1 ]]; then
    fail "$CHANGELOG declares $heading_count released versions; the first release is a single 0.1.0 (#495)"
    printf '%s\n' "$release_headings"
fi

# An empty [Unreleased] heading stays for future work (Keep a Changelog), but
# it must not be carrying the release's own content.
grep -qE '^## \[Unreleased\]' "$CHANGELOG" ||
    fail "$CHANGELOG has no [Unreleased] heading for future work"
unreleased_body="$(awk '
    /^## \[Unreleased\]/ { grab = 1; next }
    grab && /^## / { exit }
    grab { print }
' "$CHANGELOG" | grep -c '[^[:space:]]' || true)"
if [[ "$unreleased_body" -ne 0 ]]; then
    fail "$CHANGELOG's [Unreleased] section has $unreleased_body non-blank lines; nothing has shipped, so that content belongs to the first release (#495)"
fi

CHANGELOG_VERSION="$(printf '%s\n' "$release_headings" | head -1 |
    sed -E 's/^## \[([0-9]+\.[0-9]+\.[0-9]+)\].*/\1/')"
[[ -n "$CHANGELOG_VERSION" ]] || fail "could not read a released version from $CHANGELOG"

# --- (b) every version declaration agrees -----------------------------------
echo "--- version declarations agree ---"
CARGO_VERSION="$(grep -m1 '^version = ' "$MANIFEST" | sed 's/version = "\(.*\)"/\1/')"
FISHER_VERSION="$(grep -m1 '"version"' "$FISHER" | sed 's/.*"version": *"\([^"]*\)".*/\1/')"
FORMULA_VERSION="$(grep -m1 -oE 'refs/tags/v[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz' "$FORMULA" |
    sed -E 's#refs/tags/v(.*)\.tar\.gz#\1#')"

for pair in "$MANIFEST:$CARGO_VERSION" "$FISHER:$FISHER_VERSION" "$FORMULA:$FORMULA_VERSION"; do
    where="${pair%%:*}"
    what="${pair#*:}"
    if [[ -z "$what" ]]; then
        fail "could not read a version from $where"
    elif [[ "$what" != "$CHANGELOG_VERSION" ]]; then
        fail "$where declares $what but $CHANGELOG declares $CHANGELOG_VERSION (#495)"
    fi
done

# --- (c) CHANGELOG link references match the sections that exist -------------
echo "--- CHANGELOG link references match its sections ---"
link_versions="$(grep -oE '^\[[0-9]+\.[0-9]+\.[0-9]+\]:' "$CHANGELOG" | tr -d '[]:' | sort -u)"
if [[ "$link_versions" != "$CHANGELOG_VERSION" ]]; then
    fail "$CHANGELOG's link references are [${link_versions:-none}] but the released version is $CHANGELOG_VERSION"
fi
grep -qF "releases/tag/v$CHANGELOG_VERSION" "$CHANGELOG" ||
    fail "$CHANGELOG's link reference does not point at the v$CHANGELOG_VERSION release tag"

# --- (d) the release date is an ISO date or the documented placeholder -------
echo "--- release heading carries a date or the documented placeholder ---"
release_heading="$(printf '%s\n' "$release_headings" | head -1)"
if [[ ! "$release_heading" =~ ^\#\#\ \[[0-9]+\.[0-9]+\.[0-9]+\]\ -\ (YYYY-MM-DD|[0-9]{4}-[0-9]{2}-[0-9]{2})$ ]]; then
    fail "release heading '$release_heading' is neither an ISO date nor the YYYY-MM-DD placeholder the release procedure replaces (#495)"
fi

# --- (e) release notes actually render from the collapsed section ------------
echo "--- release notes render from the collapsed section ---"
notes="$(scripts/release-notes.sh --version "v$CHANGELOG_VERSION" 2>&1)" || {
    fail "scripts/release-notes.sh failed for v$CHANGELOG_VERSION: $notes"
    notes=""
}
if [[ -n "$notes" ]]; then
    grep -qF "# GPY v$CHANGELOG_VERSION" <<<"$notes" ||
        fail "release notes for v$CHANGELOG_VERSION have no version heading"
    # The Dead Client Cleanup work came from the folded-away 0.1.1 section. If
    # the collapse dropped it, the notes are the place it becomes visible.
    grep -qF 'Dead Client Cleanup' <<<"$notes" ||
        fail "release notes lost the 0.1.1 'Dead Client Cleanup' entries in the collapse (#495)"
fi

# --- (f) only operational channels are advertised ---------------------------
echo "--- no non-operational install channel is advertised ---"
channel_scan=(install.sh install-oneline.sh README.md)
while IFS= read -r doc; do
    channel_scan+=("$doc")
done < <(find docs -name '*.md' -type f | sort)

# Each entry is "<label>|<extended regex>". The Homebrew and apt patterns are
# anchored on a gpy-shaped package name so legitimate `brew install bash` /
# `apt install jq` prerequisite lines are not flagged.
channel_patterns=(
    "Homebrew|brew (install|tap) [^ ]*(gpy|jpease)"
    "crates.io|cargo install gpy"
    "Debian|apt(-get)? install [^ ]*gpy|dpkg -i"
    "Arch|pacman -S [^ ]*gpy|yay -S [^ ]*gpy|makepkg|PKGBUILD"
    "Fisher|fisher install"
)
for target in "${channel_scan[@]}"; do
    [[ -f "$target" ]] || continue
    for entry in "${channel_patterns[@]}"; do
        label="${entry%%|*}"
        pattern="${entry#*|}"
        if grep -nE "$pattern" "$target"; then
            fail "$target advertises the $label channel, which is not operational (#495)"
        fi
    done
done

# --- (g) the Homebrew formula still admits it is unfinished ------------------
echo "--- Homebrew formula declares itself unfinished ---"
if [[ ! -f "$FORMULA" ]]; then
    fail "$FORMULA not found"
else
    formula_sha="$(grep -m1 -oE '^[[:space:]]*sha256 "[^"]*"' "$FORMULA" | sed 's/.*"\(.*\)"/\1/')"
    if [[ "$formula_sha" == "REPLACE_WITH_ACTUAL_SHA256" ]]; then
        # A placeholder checksum is fine only while the file says out loud what
        # has to happen and which issue owns it.
        grep -qF '#515' "$FORMULA" ||
            fail "$FORMULA has a placeholder sha256 but does not name #515, the issue that computes it"
        grep -qF "$RELEASE_DOC" "$FORMULA" ||
            fail "$FORMULA has a placeholder sha256 but does not point at $RELEASE_DOC"
    elif [[ ! "$formula_sha" =~ ^[0-9a-f]{64}$ ]]; then
        fail "$FORMULA declares sha256 '$formula_sha', which is neither the placeholder nor a SHA-256 digest"
    fi
fi

# --- (h) SECURITY.md supports exactly the released minor line ----------------
echo "--- SECURITY.md supported versions match the release ---"
if [[ ! -f "$SECURITY" ]]; then
    fail "$SECURITY not found"
else
    expected_line="${CHANGELOG_VERSION%.*}.x"
    supported_section="$(awk '
        /^## Supported Versions/ { grab = 1; next }
        grab && /^## / { exit }
        grab { print }
    ' "$SECURITY")"
    supported_rows="$(printf '%s\n' "$supported_section" |
        grep -oE '^\|[[:space:]]*[0-9]+\.[0-9]+\.[0-9x]+[[:space:]]*\|' |
        sed -E 's/[|[:space:]]//g' | sort -u)"
    if [[ "$supported_rows" != "$expected_line" ]]; then
        fail "$SECURITY's supported-versions table lists [${supported_rows:-none}] but the only release line is $expected_line (#495)"
    fi
fi

# --- (i) the release procedure cannot drift from the workflow ----------------
echo "--- release procedure lists exactly the workflow's version-bearing files ---"
if [[ ! -f "$RELEASE_DOC" ]]; then
    fail "$RELEASE_DOC not found; the first-release procedure must be written down (#495)"
elif [[ ! -f "$RELEASE_WORKFLOW" ]]; then
    fail "$RELEASE_WORKFLOW not found"
else
    # Both sides go through the same extraction so they can be compared: take
    # every .toml/.json/.md path, then drop any bare filename that is the tail
    # of a fuller path already in the set (the workflow's closing summary line
    # says "Cargo.toml, fisher.json" where its checks said "gpy-agent/..." and
    # "fish/...").
    version_files() {
        local raw kept token other
        raw="$(grep -oE '[A-Za-z0-9_/.-]+\.(toml|json|md)' | sort -u)"
        kept=""
        while IFS= read -r token; do
            [[ -n "$token" ]] || continue
            while IFS= read -r other; do
                [[ -n "$other" && "$other" != "$token" ]] || continue
                if [[ "$other" == */"$token" ]]; then
                    token=""
                    break
                fi
            done <<<"$raw"
            [[ -n "$token" ]] && kept="$kept$token"$'\n'
        done <<<"$raw"
        printf '%s' "$kept" | sort -u
    }

    # Derived, not hardcoded: read the validate job's version step and take
    # every repository path it names. Adding a fourth version-bearing file to
    # the workflow turns this red until the procedure documents it too.
    workflow_files="$(awk '
        /^      - name: Resolve and validate version$/ { grab = 1; next }
        grab && /^      - name: / { exit }
        grab { print }
    ' "$RELEASE_WORKFLOW" | version_files)"

    bt='`'
    doc_files="$(awk '
        /<!-- version-bearing-files:start -->/ { grab = 1; next }
        /<!-- version-bearing-files:end -->/ { exit }
        grab { print }
    ' "$RELEASE_DOC" | grep -oE "${bt}[A-Za-z0-9_/.-]+${bt}" | tr -d "$bt" | version_files)"

    if [[ -z "$workflow_files" ]]; then
        fail "could not derive any version-bearing file from $RELEASE_WORKFLOW's validate step"
    elif [[ -z "$doc_files" ]]; then
        fail "$RELEASE_DOC has no <!-- version-bearing-files --> block listing what the release gate compares"
    elif [[ "$workflow_files" != "$doc_files" ]]; then
        fail "$RELEASE_DOC's version-bearing file list has drifted from $RELEASE_WORKFLOW's validate step"
        diff <(printf '%s\n' "$workflow_files") <(printf '%s\n' "$doc_files") |
            sed 's/^/       /'
    fi

    # The procedure has to tell the releaser to replace the date placeholder,
    # or (d) above becomes a permanent excuse rather than a checkpoint.
    grep -qF 'YYYY-MM-DD' "$RELEASE_DOC" ||
        fail "$RELEASE_DOC does not tell the releaser to replace the YYYY-MM-DD date placeholder"
    grep -qF "v$CHANGELOG_VERSION" "$RELEASE_DOC" ||
        fail "$RELEASE_DOC does not name the release being cut (v$CHANGELOG_VERSION)"
fi

# --- (j) every advertised install channel has an executing test ------------
# The audit behind #635 found README's from-source instructions could not
# work and nothing ran them. Each channel a user can be sent to must be backed
# by a test that executes the channel's installer, not one that greps for
# strings; a doc that names a new channel without a row here fails.
echo "--- every advertised install channel has an executing test ---"
channel_tests=(
    "one-line installer|curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh|tests/bash/install_oneline_verification.test.bash"
    "release archive (install.sh)|Manual Installation|tests/bash/install_checksum_verification.test.bash"
    "from source (install-dev.fish)|fish install-dev.fish|tests/bash/install_from_source_docs.test.bash"
)
for entry in "${channel_tests[@]}"; do
    IFS='|' read -r label marker test_file <<<"$entry"
    if ! grep -qF -- "$marker" README.md docs/INSTALL.md; then
        fail "neither README.md nor docs/INSTALL.md advertises the $label channel any more (marker: $marker); drop its row or restore the docs"
    fi
    if [[ ! -f "$test_file" ]]; then
        fail "the $label channel has no executing test ($test_file missing)"
    fi
done
# The from-source channel is documented in both files and must agree on the
# installer it names (#635).
grep -qF 'fish install-dev.fish' README.md ||
    fail "README.md's from-source section no longer uses install-dev.fish"
grep -qF 'fish install-dev.fish' docs/INSTALL.md ||
    fail "docs/INSTALL.md's Install Locally section no longer uses install-dev.fish"
if grep -nE '^bash install\.sh$' README.md; then
    fail "README.md tells a source checkout to run install.sh, which needs the release bin/ payload (#635)"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
