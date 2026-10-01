#!/usr/bin/env bash
# Fail if any tracked Markdown file links to a repository path that does not
# exist, or to a heading anchor the target file does not define (gpy#505).
#
# Also fails on a plain-text reference to a removed planning file (e.g. "see
# TODO.md") outside docs/archive/: Markdown links are not the only way a doc
# points at a file that no longer exists, and a link checker alone cannot
# catch prose (2026-09-03 audit follow-up on gpy#505).
#
# Scans tracked files only, via git ls-files, so build artifacts and local
# scratch files can never trip the gate. Only repository-relative links are
# resolved: external URLs are never fetched, because a network check turns the
# gate red for reasons no commit can fix.

set -euo pipefail
cd "$(dirname "$0")/.."

failures=0
broken=0
prose_broken=0

# Every path the repository publishes -- each tracked file plus each of its
# ancestor directories -- one per line.
#
# Membership is tested against this manifest rather than with `[[ -e ]]`
# because macOS mounts a case-insensitive filesystem by default: `[[ -e ]]`
# happily resolves a link to ARCHITECTURE.md against docs/dev/architecture.md,
# so a rename that broke every Linux checkout would pass here silently.
MANIFEST="$(mktemp)"
trap 'rm -f "$MANIFEST"' EXIT

git ls-files |
    awk '
        { print }
        {
            path = $0
            while (match(path, "/[^/]*$")) {
                path = substr(path, 1, RSTART - 1)
                print path
            }
        }
    ' | sort -u >"$MANIFEST"

path_exists() {
    grep -qxF -- "$1" "$MANIFEST"
}

# Basenames of every tracked file, one per line -- used by the plain-text
# reference pass below, which has no directory context to resolve against
# (unlike a Markdown link, a bare "TODO.md" in prose carries no path).
BASENAME_MANIFEST="$(mktemp)"
trap 'rm -f "$MANIFEST" "$BASENAME_MANIFEST"' EXIT

git ls-files | awk -F/ '{ print $NF }' | sort -u >"$BASENAME_MANIFEST"

basename_exists() {
    grep -qxF -- "$1" "$BASENAME_MANIFEST"
}

# Real root-level (or root-adjacent) files whose all-caps names would
# otherwise look exactly like the planning-doc pattern this pass hunts for.
# basename_exists already covers all of these; this list is a second,
# explicit guard so a future rename of one of them can't turn this pass into
# a silent no-op for its own sake.
# Deliberately omits the root agent-instruction files that the #509 cutover
# excludes: a prose reference to one of them in a public document is exactly
# what this pass should then flag (see #504's privacy guard).
KNOWN_REAL_FILES=(
    README.md CONTRIBUTING.md SECURITY.md LICENSE CHANGELOG.md
    CODE_OF_CONDUCT.md INSTALL.md
)

is_known_real_file() {
    local token="$1" known
    for known in "${KNOWN_REAL_FILES[@]}"; do
        [[ "$token" == "$known" ]] && return 0
    done
    return 1
}

# Drop fenced code blocks, preserving line numbering so a reported line still
# matches the file. Fenced samples are where `[$style]`, `[fg:$bg bg:default]`
# and `[bold green]` live; parsing them as links is the single largest source
# of false positives in a naive scan.
strip_fences() {
    awk '
        # A fence delimiter is at least three backticks or tildes, optionally
        # indented, and a fence only closes with the same character.
        /^[[:space:]]*(```|~~~)/ {
            here = ($0 ~ /^[[:space:]]*```/) ? "`" : "~"
            if (!in_fence) { in_fence = 1; fence = here; print ""; next }
            if (fence == here) { in_fence = 0 }
            print ""
            next
        }
        in_fence { print ""; next }
        { print }
    ' "$1"
}

# Additionally drop inline code spans and HTML comments. Applied when hunting
# for links -- `[bold green]` in prose is a code span, not a link -- but NOT
# when collecting heading anchors, because GitHub slugifies the text inside a
# code span: `#### \`gpy doctor\`` is reachable as #gpy-doctor.
strip_inline_code() {
    # The sed script is a literal backtick pattern, not shell expansion.
    # shellcheck disable=SC2016
    sed -e 's/`[^`]*`//g' -e 's/<!--.*-->//g'
}

# Collapse "." and ".." lexically. Realpath is unusable here: the target of a
# broken link does not exist, so it has no canonical path to resolve to.
normalize_path() {
    local path="$1" part out=()
    local oldifs="$IFS"
    IFS='/'
    # shellcheck disable=SC2206  # deliberate word splitting on the path
    local parts=($path)
    IFS="$oldifs"

    for part in "${parts[@]}"; do
        case "$part" in
            '' | .) ;;
            ..)
                if [[ ${#out[@]} -gt 0 && "${out[${#out[@]} - 1]}" != ".." ]]; then
                    unset "out[${#out[@]}-1]"
                    # Re-pack so the next index arithmetic stays contiguous.
                    out=(${out[@]+"${out[@]}"})
                else
                    out+=("..")
                fi
                ;;
            *) out+=("$part") ;;
        esac
    done

    local oldifs2="$IFS"
    IFS='/'
    printf '%s' "${out[*]-}"
    IFS="$oldifs2"
}

# GitHub's heading-to-anchor rule: lowercase, drop everything that is not a
# word character, hyphen or space, then spaces to hyphens.
slugify() {
    # The trailing newline matters: anchors_of emits one slug per line, and
    # without it every heading in a file collapses into a single token.
    printf '%s\n' "$1" |
        tr '[:upper:]' '[:lower:]' |
        sed -e 's/[^a-z0-9 _-]//g' -e 's/ /-/g'
}

# Every anchor a Markdown file defines: one per ATX heading, plus explicit
# HTML `name=`/`id=` attributes, which several docs use for stable anchors.
anchors_of() {
    local file="$1"
    strip_fences "$file" |
        sed -n 's/^#\{1,6\}[[:space:]]\{1,\}//p' |
        while IFS= read -r heading; do
            # Trailing "#" runs and surrounding whitespace are not part of the
            # rendered heading text.
            heading="${heading%"${heading##*[![:space:]#]}"}"
            slugify "$heading"
        done
    grep -oE '(name|id)="[^"]+"' "$file" 2>/dev/null | sed 's/.*="//; s/"$//' || true
}

report_broken() {
    echo "✗ $1:$2 -> $3"
    [[ -n "$4" ]] && printf '  %s\n' "$4"
    broken=$((broken + 1))
    return 0
}

# Pull every link target out of one file, as "<line>\t<target>". NR is the
# real line number because strip_fences blanks fenced lines rather than
# deleting them.
link_targets() {
    strip_fences "$1" | strip_inline_code | awk '
        {
            # Inline links and images: [text](target), ![alt](target).
            rest = $0
            while (match(rest, /\]\([^()]*\)/)) {
                print NR "\t" substr(rest, RSTART + 2, RLENGTH - 3)
                rest = substr(rest, RSTART + RLENGTH)
            }
            # Reference definitions: [label]: target
            if (match($0, /^\[[^]]+\]:[[:space:]]*[^[:space:]]+/)) {
                target = substr($0, RSTART, RLENGTH)
                sub(/^\[[^]]+\]:[[:space:]]*/, "", target)
                print NR "\t" target
            }
        }
    '
}

# Verify one "#anchor" against the headings of the file it points into.
# Anchors are only checkable against Markdown this checker can read.
check_anchor() {
    local file="$1" lineno="$2" target="$3" anchor="$4" resolved="$5"

    [[ "$resolved" == *.md && -f "$resolved" ]] || return 0

    # The anchor list is materialised before matching: piping it into `grep -q`
    # would let grep exit on the first hit, SIGPIPE the producer, and -- under
    # `set -o pipefail` -- fail every check.
    local slug found
    slug="$(slugify "$anchor")"
    found="$(anchors_of "$resolved" | grep -cxF -- "$slug" || true)"
    [[ "$found" -eq 0 ]] &&
        report_broken "$file" "$lineno" "$target" "no heading anchor #$anchor in $resolved"
    return 0
}

check_file() {
    local file="$1" dir lineno target path anchor resolved
    dir="$(dirname "$file")"

    while IFS=$'\t' read -r lineno target; do
        [[ -n "$target" ]] || continue

        # A link title -- [text](path "Title") -- is not part of the target.
        target="${target%%[[:space:]]*}"
        # Angle-bracket link syntax: [text](<path with spaces>)
        target="${target#<}"
        target="${target%>}"

        [[ -n "$target" ]] || continue

        case "$target" in
            *://* | mailto:* | tel:*) continue ;;
            # Placeholders in templates, e.g. [docs](PATH_TO_DOC) with $VAR.
            *'$'* | *'{{'*) continue ;;
        esac

        path="${target%%#*}"
        anchor=""
        [[ "$target" == *'#'* ]] && anchor="${target#*#}"

        # A bare "#anchor" is a jump within the same file.
        if [[ -z "$path" ]]; then
            [[ -n "$anchor" ]] || continue
            check_anchor "$file" "$lineno" "$target" "$anchor" "$file"
            continue
        fi

        # Percent-escapes only ever appear here as %20 for a literal space.
        path="${path//%20/ }"

        if [[ "$path" == /* ]]; then
            # A leading slash is repository-root-relative on GitHub.
            resolved="$(normalize_path "${path#/}")"
        else
            resolved="$(normalize_path "$dir/$path")"
        fi

        if [[ -z "$resolved" || "$resolved" == ..* ]]; then
            report_broken "$file" "$lineno" "$target" "escapes the repository root"
            continue
        fi

        if ! path_exists "$resolved"; then
            report_broken "$file" "$lineno" "$target" "no such tracked file or directory: $resolved"
            continue
        fi

        if [[ -n "$anchor" ]]; then
            check_anchor "$file" "$lineno" "$target" "$anchor" "$resolved"
        fi
    done < <(link_targets "$file")
}

# Flag a plain-text mention of a SCREAMING_CASE.md filename (the shape planning
# docs like TODO.md, AGENT.md, and DESIGN_DECISIONS.md take) when no tracked
# file anywhere in the repository has that basename. Skips docs/archive/:
# archived material is allowed to describe files that no longer exist because
# it is documenting history, not current guidance, and already carries the
# "> **Archived.**" marker that says so.
check_prose_refs() {
    local file="$1"

    case "$file" in
        docs/archive/*) return 0 ;;
    esac

    while IFS=: read -r lineno token; do
        [[ -n "$token" ]] || continue
        is_known_real_file "$token" && continue
        basename_exists "$token" && continue
        report_broken "$file" "$lineno" "$token" "no tracked file named $token"
        prose_broken=$((prose_broken + 1))
    done < <(strip_fences "$file" | strip_inline_code | grep -noE '\b[A-Z][A-Z0-9_]+\.md\b')
}

echo "Documentation link scan"

files=0
while IFS= read -r file; do
    [[ -f "$file" ]] || continue
    files=$((files + 1))
    check_file "$file"
    check_prose_refs "$file"
done < <(git ls-files -- '*.md' '*.markdown')

if [[ $broken -ne 0 ]]; then
    failures=$((failures + 1))
fi

if [[ $failures -eq 0 ]]; then
    echo "✓ All repository-relative links and plain-text file references in $files tracked Markdown files resolve"
    exit 0
fi

link_broken=$((broken - prose_broken))
echo "$link_broken broken link(s) and $prose_broken broken plain-text reference(s) across $files tracked Markdown files" >&2
exit 1
