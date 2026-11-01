#!/usr/bin/env bash
# tests/repo/doc_links.test.bash
#
# Tracked docs must not link to renamed, misplaced or never-existing files.
# Exercises scripts/check-doc-links.sh against scratch repos, so the gate itself is tested.

# Single-quoted `$style`/backticks below are scanner fixture text.
# shellcheck disable=SC2016

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCANNER="$ROOT/scripts/check-doc-links.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# Scratch repo with $1 as docs/sample.md; echoes "<exit>|<output>".
run_scanner_on() {
    local content="$1"
    local repo="$WORKDIR/repo-$RANDOM$RANDOM"

    mkdir -p "$repo/scripts" "$repo/docs/dev" "$repo/src"
    cp "$SCANNER" "$repo/scripts/check-doc-links.sh"
    printf '%s\n' "$content" >"$repo/docs/sample.md"

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

echo "--- broken relative links must be flagged ---"
expect_flagged "missing sibling" 'See [the guide](MISSING_GUIDE.md) for details.'
expect_flagged "missing nested" 'See [deep](dev/absent.md).'
expect_flagged "wrong depth to source" 'See [main](src/main.rs) -- one ../ too few.'
expect_flagged "missing directory" 'Code lives in [watcher](../gpy-agent/src/watcher/).'
expect_flagged "root-relative miss" 'See [x](/docs/nope.md).'
expect_flagged "reference definition" '[guide]: MISSING_GUIDE.md'
expect_flagged "image target" 'Diagram: ![arch](img/arch.png)'
expect_flagged "escapes the repo" 'See [outside](../../../etc/passwd).'

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

# Code samples are not links.
echo "--- code samples must not be parsed as links ---"
expect_clean "fenced toml style" "$(printf '```toml\nformat = "[$style](bold green)"\n```\n')"
expect_clean "fenced starship style" "$(printf '```toml\nstyle = "[fg:$bg bg:default](nope.md)"\n```\n')"
expect_clean "fenced markdown sample" "$(printf 'Example:\n\n```markdown\n[a link](ABSENT.md)\n```\n')"
expect_clean "tilde fence" "$(printf '~~~toml\nformat = "[bold green](gone.md)"\n~~~\n')"
expect_clean "indented fence" "$(printf '1. Step:\n\n   ```toml\n   x = "[$style](gone.md)"\n   ```\n')"
expect_clean "inline code span" 'Set `format = "[$style](gone.md)"` in config.toml.'
expect_clean "html comment" '<!-- [old](REMOVED.md) -->'

# A broken link after a closed fence is still broken.
# fence is still a broken link.
expect_flagged "link after closed fence" "$(printf '```toml\nx = 1\n```\n\nSee [gone](GONE.md).\n')"

echo "--- external links must be left alone ---"
expect_clean "https" 'See [starship](https://starship.rs/config/).'
expect_clean "http" 'See [example](http://example.invalid/nope).'
expect_clean "mailto" 'Mail [us](mailto:nobody@example.invalid).'
expect_clean "url with fragment" 'See [zsh](https://zsh.sourceforge.io/Doc/Release/Functions.html#Hook-Functions).'

# Case-only mismatch: macOS's case-insensitive FS would hide it; Linux 404s.
echo "--- case-only mismatches must be flagged ---"
expect_flagged "uppercase sibling" 'See [neighbour](NEIGHBOUR.md).'
expect_flagged "uppercase readme" 'See [readme](../Readme.md).'

echo "--- heading anchors must be checked ---"
expect_clean "existing anchor" 'See [threads](neighbour.md#threading-model).'
expect_clean "anchor on code-span heading" 'Run [gpy doctor](neighbour.md#gpy-doctor).'
expect_flagged "absent anchor" 'See [nope](neighbour.md#no-such-heading).'
expect_flagged "absent same-file anchor" 'Jump to [nowhere](#no-such-heading).'
expect_clean "punctuated heading" "$(printf '## Advanced: Agent-Assisted Segment\n\nSee [it](#advanced-agent-assisted-segment).\n')"
expect_clean "anchor into a non-Markdown file" 'See [toml](../src/main.rs#L10).'

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

echo "--- untracked files are ignored ---"
UNTRACKED_REPO="$WORKDIR/untracked"
mkdir -p "$UNTRACKED_REPO/scripts" "$UNTRACKED_REPO/docs"
cp "$SCANNER" "$UNTRACKED_REPO/scripts/check-doc-links.sh"
git -C "$UNTRACKED_REPO" init -q
echo 'See [gone](ABSENT.md).' >"$UNTRACKED_REPO/docs/scratch.md"
if ! (cd "$UNTRACKED_REPO" && bash scripts/check-doc-links.sh >/dev/null 2>&1); then
    fail "scanner flagged an untracked file; it must scan tracked files only"
fi

echo "--- the repository itself must be clean ---"
if ! bash "$SCANNER" >/dev/null 2>&1; then
    echo "--- scanner output ---"
    bash "$SCANNER" 2>&1 | sed 's/^/  /'
    fail "the repository has broken documentation links"
fi

echo "--- the checker must be registered in the quality gate ---"
grep -q 'check-doc-links\.sh' "$ROOT/scripts/quality-check.sh" ||
    fail "scripts/quality-check.sh does not run check-doc-links.sh"

test_finish
