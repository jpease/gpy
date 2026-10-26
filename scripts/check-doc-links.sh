#!/usr/bin/env bash
# Fail if a tracked Markdown file links to a missing repository path or heading
# anchor, or names a nonexistent SCREAMING_CASE.md file in prose. External URLs
# are never fetched: a network failure is not something a commit can fix.

set -euo pipefail
cd "$(dirname "$0")/.."

failures=0
broken=0
prose_broken=0

# Tracked files plus their ancestor directories. Not `[[ -e ]]`: macOS is
# case-insensitive and would pass a link that breaks every Linux checkout.
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

# Basenames for the prose pass, which has no directory context.
BASENAME_MANIFEST="$(mktemp)"
trap 'rm -f "$MANIFEST" "$BASENAME_MANIFEST"' EXIT

git ls-files | awk -F/ '{ print $NF }' | sort -u >"$BASENAME_MANIFEST"

basename_exists() {
    grep -qxF -- "$1" "$BASENAME_MANIFEST"
}

# Explicit guard on top of basename_exists. Root agent-instruction files are
# deliberately absent: they are not published, so a prose reference is a bug.
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

# Blank fenced blocks, keeping line numbers; samples like `[bold green]` are not links.
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

# Links only, not anchors: GitHub slugifies code-span heading text.
strip_inline_code() {
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
    # Trailing newline: anchors_of emits one slug per line.
    printf '%s\n' "$1" |
        tr '[:upper:]' '[:lower:]' |
        sed -e 's/[^a-z0-9 _-]//g' -e 's/ /-/g'
}

# ATX headings plus explicit HTML name=/id= anchors.
anchors_of() {
    local file="$1"
    strip_fences "$file" |
        sed -n 's/^#\{1,6\}[[:space:]]\{1,\}//p' |
        while IFS= read -r heading; do
            heading="${heading%"${heading##*[![:space:]#]}"}"
            slugify "$heading"
        done
    grep -oE '(name|id)="[^"]+"' "$file" 2>/dev/null | sed 's/.*="//; s/"$//' || true
}

report_broken() {
    echo "FAIL: $1:$2 -> $3"
    [[ -n "$4" ]] && printf '  %s\n' "$4"
    broken=$((broken + 1))
    return 0
}

# "<line>\t<target>" per link; strip_fences keeps NR accurate.
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

check_anchor() {
    local file="$1" lineno="$2" target="$3" anchor="$4" resolved="$5"

    [[ "$resolved" == *.md && -f "$resolved" ]] || return 0

    # Materialised first: `| grep -q` would SIGPIPE the producer under pipefail.
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

# Plain-text SCREAMING_CASE.md mentions with no tracked file of that basename.
check_prose_refs() {
    local file="$1"

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
    echo "ok: all repository-relative links and plain-text file references in $files tracked Markdown files resolve"
    exit 0
fi

link_broken=$((broken - prose_broken))
echo "$link_broken broken link(s) and $prose_broken broken plain-text reference(s) across $files tracked Markdown files" >&2
exit 1
