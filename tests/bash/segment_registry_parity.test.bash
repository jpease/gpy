#!/usr/bin/env bash
# tests/bash/segment_registry_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# The CLI registry (`gpy __complete segment` in an empty sandbox), the
# fish/bash/zsh segment files and the `gpy enable` list in
# docs/user/cli-reference.md (minus aliases) are the same set.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

DOC="$ROOT/docs/user/cli-reference.md"
GPY="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$GPY" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy) || {
        echo "FAIL: could not build the gpy binary"
        exit 1
    }
fi

# Names the docs list for a segment that are aliases, not segment names.
#   `lang` -- accepted by `gpy enable` as an alias for `language`.
DOC_ALIASES="lang"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-segparity.XXXXXX")" || exit 1
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/home" "$SANDBOX/config" "$SANDBOX/cache" "$SANDBOX/run"
chmod 700 "$SANDBOX/run"

registry="$(
    env -i PATH="$PATH" HOME="$SANDBOX/home" XDG_CONFIG_HOME="$SANDBOX/config" \
        XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX/run" \
        "$GPY" __complete segment | sort -u
)"
if [ -z "$registry" ]; then
    echo "FAIL: 'gpy __complete segment' printed nothing"
    exit 1
fi

# `compare_sets LABEL_A SET_A LABEL_B SET_B` -- both are sorted newline lists.
compare_sets() {
    local label_a="$1" set_a="$2" label_b="$3" set_b="$4" name bad=0
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        if ! printf '%s\n' "$set_b" | grep -qxF -- "$name"; then
            fail "'$name' is in $label_a but missing from $label_b"
            bad=1
        fi
    done <<EOF
$set_a
EOF
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        if ! printf '%s\n' "$set_a" | grep -qxF -- "$name"; then
            fail "'$name' is in $label_b but missing from $label_a"
            bad=1
        fi
    done <<EOF
$set_b
EOF
    [ "$bad" -eq 0 ] && pass "$label_a == $label_b"
}

# The segment names a shell ships: <shell>/segments/<name>.<ext>.
shipped_segments() {
    local shell="$1" ext="$2" file
    for file in "$ROOT/$shell/segments/"*."$ext"; do
        [ -e "$file" ] || continue
        basename "$file" ".$ext"
    done | sort -u
}

for pair in fish:fish bash:bash zsh:zsh; do
    shell="${pair%%:*}"
    ext="${pair##*:}"
    shipped="$(shipped_segments "$shell" "$ext")"
    if [ -z "$shipped" ]; then
        fail "no $shell segment files found under $shell/segments/"
        continue
    fi
    compare_sets "the builtin segment registry" "$registry" "$shell/segments/*.$ext" "$shipped"
done

# The `gpy enable` "Valid Segments" bullets: "- `name` - text" or
# "- `alias` / `name` - text". Scoped from the `gpy enable` heading to the
# next heading so other lists of backticked names are never read.
documented="$(
    awk '
        /^#### `gpy enable / { in_enable = 1; next }
        in_enable && /^#### / { exit }
        in_enable && /^\*\*Valid Segments/ { in_list = 1; next }
        in_list && /^\*\*/ { exit }
        in_list && /^- `/ {
            line = $0
            sub(/^- /, "", line)
            sub(/ - .*$/, "", line)
            n = split(line, parts, " / ")
            for (i = 1; i <= n; i++) {
                gsub(/`/, "", parts[i])
                print parts[i]
            }
        }
    ' "$DOC" | while IFS= read -r name; do
        case " $DOC_ALIASES " in
            *" $name "*) ;;
            *) printf '%s\n' "$name" ;;
        esac
    done | sort -u
)"
if [ -z "$documented" ]; then
    fail "found no Valid Segments list under \`gpy enable\` in docs/user/cli-reference.md"
else
    compare_sets "the builtin segment registry" "$registry" "docs/user/cli-reference.md (gpy enable)" "$documented"
fi

test_finish "builtin segments, shipped shell segment files and documented segments agree"
