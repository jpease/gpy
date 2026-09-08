#!/usr/bin/env bash
# tests/bash/doc_links.test.bash
#
# Regression test for #505: tracked docs linked to files that had been renamed
# (ARCHITECTURE.md -> docs/dev/architecture.md), to source paths at the wrong
# depth, and to documents that never existed -- 44 dead links across 14 files
# by the time anyone counted.
#
# This exercises scripts/check-doc-links.sh against throwaway git repositories
# rather than re-implementing its scan, so the gate itself is under test.
# Asserting only that the real tree is clean would pass just as happily
# against a scanner that always exits 0.
#
# It asserts:
#   (a) a planted broken relative link is detected, and the offending file is
#       named in the output
#   (b) links that resolve -- including directories, root-relative paths and
#       reference definitions -- are accepted
#   (c) a broken-looking link inside a fenced code block or an inline code
#       span is NOT flagged: `[$style]` and `[bold green]` are configuration
#       and prompt samples, not links, and a naive scan reports 14 of them
#   (d) external URLs are never resolved, so the gate needs no network
#   (e) a case-only mismatch is caught even on a case-insensitive filesystem,
#       which is what hides ARCHITECTURE.md -> architecture.md on macOS
#   (f) heading anchors are checked, including a heading whose text is a code
#       span (`## `gpy doctor`` is reachable as #gpy-doctor)
#   (g) untracked files are ignored, since the gate is about published history
#   (h) the real repository is clean
#   (i) a plain-text SCREAMING_CASE.md reference (not a Markdown link) to a
#       file that does not exist anywhere in the tree is flagged, fenced and
#       inline occurrences are not, and the same reference inside
#       docs/archive/ is silent because archived material is allowed to
#       describe files that no longer exist (2026-09-03 audit follow-up)

# Every single-quoted `$style` / `$bg` / backtick below is fixture text fed to
# the scanner verbatim. Nothing in this file is meant to expand.
# shellcheck disable=SC2016

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCANNER="$ROOT/scripts/check-doc-links.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Build a scratch repo holding $1 as docs/sample.md plus a small fixture tree,
# run the scanner in it, and echo "<exit>|<output>". The scanner resolves the
# repo root as dirname($0)/.., so it is copied to scripts/ inside the repo.
run_scanner_on() {
    local content="$1"
    local repo="$WORKDIR/repo-$RANDOM$RANDOM"

    mkdir -p "$repo/scripts" "$repo/docs/dev" "$repo/src"
    cp "$SCANNER" "$repo/scripts/check-doc-links.sh"
    printf '%s\n' "$content" >"$repo/docs/sample.md"

    # Fixture targets a link may legitimately resolve to.
    printf '# Neighbour\n\n## Threading Model\n\n## `gpy doctor`\n' >"$repo/docs/neighbour.md"
    printf '# Deep\n' >"$repo/docs/dev/deep.md"
    printf '# Root\n' >"$repo/README.md"
    printf 'fn main() {}\n' >"$repo/src/main.rs"

    git -C "$repo" init -q
    git -C "$repo" add -A
    # Committing is not required -- git ls-files reads the index -- but
    # staging is, so an untracked-file case must skip the `git add` above.

    local out status
    out="$(cd "$repo" && bash scripts/check-doc-links.sh 2>&1)" && status=0 || status=$?
    printf '%s|%s' "$status" "$out"
}

expect_flagged() {
    local label="$1" content="$2" result
    result="$(run_scanner_on "$content")"
    [[ "${result%%|*}" -ne 0 ]] || fail "$label: scanner exited 0, expected a failure"
    [[ "${result#*|}" == *"docs/sample.md"* ]] ||
        fail "$label: output does not name the offending file"
}

expect_clean() {
    local label="$1" content="$2" result
    result="$(run_scanner_on "$content")"
    [[ "${result%%|*}" -eq 0 ]] ||
        fail "$label: scanner exited nonzero on acceptable content: ${result#*|}"
}

# Same as run_scanner_on, but the fixture content lands at an arbitrary
# tracked path instead of the fixed docs/sample.md -- needed to plant a
# reference inside docs/archive/ and confirm the archive exclusion.
run_scanner_on_path() {
    local relpath="$1" content="$2"
    local repo="$WORKDIR/repo-$RANDOM$RANDOM"

    mkdir -p "$repo/scripts" "$repo/docs/dev" "$repo/docs/archive" "$repo/src"
    mkdir -p "$repo/$(dirname "$relpath")"
    cp "$SCANNER" "$repo/scripts/check-doc-links.sh"
    printf '%s\n' "$content" >"$repo/$relpath"

    printf '# Neighbour\n\n## Threading Model\n\n## `gpy doctor`\n' >"$repo/docs/neighbour.md"
    printf '# Deep\n' >"$repo/docs/dev/deep.md"
    printf '# Root\n' >"$repo/README.md"
    printf 'fn main() {}\n' >"$repo/src/main.rs"

    git -C "$repo" init -q
    git -C "$repo" add -A

    local out status
    out="$(cd "$repo" && bash scripts/check-doc-links.sh 2>&1)" && status=0 || status=$?
    printf '%s|%s' "$status" "$out"
}

expect_flagged_path() {
    local label="$1" relpath="$2" content="$3" result
    result="$(run_scanner_on_path "$relpath" "$content")"
    [[ "${result%%|*}" -ne 0 ]] || fail "$label: scanner exited 0, expected a failure"
    [[ "${result#*|}" == *"$relpath"* ]] ||
        fail "$label: output does not name the offending file"
}

expect_clean_path() {
    local label="$1" relpath="$2" content="$3" result
    result="$(run_scanner_on_path "$relpath" "$content")"
    [[ "${result%%|*}" -eq 0 ]] ||
        fail "$label: scanner exited nonzero on acceptable content: ${result#*|}"
}

# (a) broken relative links are detected
echo "--- broken relative links must be flagged ---"
expect_flagged "missing sibling" 'See [the guide](MISSING_GUIDE.md) for details.'
expect_flagged "missing nested" 'See [deep](dev/absent.md).'
expect_flagged "wrong depth to source" 'See [main](src/main.rs) -- one ../ too few.'
expect_flagged "missing directory" 'Code lives in [watcher](../gpy-agent/src/watcher/).'
expect_flagged "root-relative miss" 'See [x](/docs/nope.md).'
expect_flagged "reference definition" '[guide]: MISSING_GUIDE.md'
expect_flagged "image target" 'Diagram: ![arch](img/arch.png)'
expect_flagged "escapes the repo" 'See [outside](../../../etc/passwd).'

# (b) links that resolve are accepted
echo "--- resolving links must pass ---"
expect_clean "sibling file" 'See [neighbour](neighbour.md).'
expect_clean "nested file" 'See [deep](dev/deep.md).'
expect_clean "parent file" 'See [readme](../README.md).'
expect_clean "source file" 'See [main](../src/main.rs).'
expect_clean "directory target" 'Code lives in [src](../src/).'
expect_clean "root-relative" 'See [readme](/README.md).'
expect_clean "reference definition" '[nb]: neighbour.md'
expect_clean "link with title" 'See [nb](neighbour.md "The Neighbour").'
expect_clean "bare anchor" "$(printf '## Overview\n\nJump to [top](#overview).\n')"

# (c) code samples are not links.
# A scan that does not strip fenced and inline code reports every one of
# these, which is the false-positive class this checker exists to avoid.
echo "--- code samples must not be parsed as links ---"
expect_clean "fenced toml style" "$(printf '```toml\nformat = "[$style](bold green)"\n```\n')"
expect_clean "fenced starship style" "$(printf '```toml\nstyle = "[fg:$bg bg:default](nope.md)"\n```\n')"
expect_clean "fenced markdown sample" "$(printf 'Example:\n\n```markdown\n[a link](ABSENT.md)\n```\n')"
expect_clean "tilde fence" "$(printf '~~~toml\nformat = "[bold green](gone.md)"\n~~~\n')"
expect_clean "indented fence" "$(printf '1. Step:\n\n   ```toml\n   x = "[$style](gone.md)"\n   ```\n')"
expect_clean "inline code span" 'Set `format = "[$style](gone.md)"` in config.toml.'
expect_clean "html comment" '<!-- [old](REMOVED.md) -->'

# A fence must not swallow the rest of the file: a broken link after a closed
# fence is still a broken link.
expect_flagged "link after closed fence" "$(printf '```toml\nx = 1\n```\n\nSee [gone](GONE.md).\n')"

# (d) external URLs are never resolved -- the gate must not need a network
echo "--- external links must be left alone ---"
expect_clean "https" 'See [starship](https://starship.rs/config/).'
expect_clean "http" 'See [example](http://example.invalid/nope).'
expect_clean "mailto" 'Mail [us](mailto:nobody@example.invalid).'
expect_clean "url with fragment" 'See [zsh](https://zsh.sourceforge.io/Doc/Release/Functions.html#Hook-Functions).'

# (e) case-only mismatches must be caught.
# macOS mounts a case-insensitive filesystem, so `[[ -e ]]` resolves
# NEIGHBOUR.md against neighbour.md and the gate would pass locally while
# every Linux checkout 404s. This case pins that down.
echo "--- case-only mismatches must be flagged ---"
expect_flagged "uppercase sibling" 'See [neighbour](NEIGHBOUR.md).'
expect_flagged "uppercase readme" 'See [readme](../Readme.md).'

# (f) heading anchors
echo "--- heading anchors must be checked ---"
expect_clean "existing anchor" 'See [threads](neighbour.md#threading-model).'
expect_clean "anchor on code-span heading" 'Run [gpy doctor](neighbour.md#gpy-doctor).'
expect_flagged "absent anchor" 'See [nope](neighbour.md#no-such-heading).'
# Same-file jumps are the ones tables of contents are built from, so they get
# the same treatment as cross-file ones.
expect_flagged "absent same-file anchor" 'Jump to [nowhere](#no-such-heading).'
expect_clean "punctuated heading" "$(printf '## Advanced: Agent-Assisted Segment\n\nSee [it](#advanced-agent-assisted-segment).\n')"
expect_clean "anchor into a non-Markdown file" 'See [toml](../src/main.rs#L10).'

# (i) plain-text references to a removed planning file, outside Markdown
# link syntax entirely, must also be caught -- and only outside docs/archive/.
echo "--- plain-text SCREAMING_CASE.md references must be flagged ---"
expect_flagged "plain-text TODO.md reference" 'See TODO.md for details.'
expect_flagged "plain-text AGENT.md reference" 'Follow the guidance in AGENT.md.'
expect_flagged "plain-text DESIGN_DECISIONS.md reference" \
    'Before implementing anything, read DESIGN_DECISIONS.md.'

echo "--- fenced or inline plain-text references must not be flagged ---"
expect_clean "fenced plain-text reference" "$(printf '```\nSee TODO.md for details.\n```\n')"
expect_clean "inline-code plain-text reference" 'See `TODO.md` for details.'

echo "--- a real filename is never flagged, even via the explicit safety list ---"
expect_clean "known real root file" 'See CONTRIBUTING.md for workflow rules.'
expect_clean "lowercase filename untouched" 'See architecture.md for details.'

echo "--- archived docs may describe files that no longer exist ---"
expect_clean_path "archived plain-text reference" "docs/archive/sample.md" \
    'See TODO.md for details.'

# (g) untracked files are out of scope: the gate guards published history.
echo "--- untracked files are ignored ---"
UNTRACKED_REPO="$WORKDIR/untracked"
mkdir -p "$UNTRACKED_REPO/scripts" "$UNTRACKED_REPO/docs"
cp "$SCANNER" "$UNTRACKED_REPO/scripts/check-doc-links.sh"
git -C "$UNTRACKED_REPO" init -q
echo 'See [gone](ABSENT.md).' >"$UNTRACKED_REPO/docs/scratch.md"
if ! (cd "$UNTRACKED_REPO" && bash scripts/check-doc-links.sh >/dev/null 2>&1); then
    fail "scanner flagged an untracked file; it must scan tracked files only"
fi

# (h) the real repository is clean
echo "--- the repository itself must be clean ---"
if ! bash "$SCANNER" >/dev/null 2>&1; then
    echo "--- scanner output ---"
    bash "$SCANNER" 2>&1 | sed 's/^/  /'
    fail "the repository has broken documentation links"
fi

# The checker must be wired into the quality gate, not merely present. #505
# requires it to run in CI, which it does through scripts/quality-check.sh --
# registered at every mode the gate offers, the way check-privacy-patterns.sh
# is. A checker nobody runs catches nothing.
#
# Four, not five: this counted a fifth occurrence in the SHELLCHECK_TARGETS
# array until #510 replaced that hand-maintained list with a glob over
# scripts/*.sh. The script is still shellchecked, just not by name -- so
# counting names is no longer the way to prove it.
echo "--- the checker must be registered in the quality gate ---"
registrations="$(grep -c 'check-doc-links\.sh' "$ROOT/scripts/quality-check.sh" || true)"
if [[ "$registrations" -lt 4 ]]; then
    fail "check-doc-links.sh appears $registrations time(s) in quality-check.sh;" \
        "expected 4 (full, --fast, --rust-only, --shell-only)"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
