#!/usr/bin/env bash
# Regenerate gpy-agent/src/theme/released_hashes.rs: the SHA-256 of every
# released version of each builtin theme, which `gpy setup` uses to retire user
# copies that carry no edits (gpy-agent/src/theme/retire.rs).
#
# Sources, merged and deduplicated:
#   - config/themes/*.toml at every vMAJOR.MINOR.PATCH tag of this repository
#     and of each --git-dir (e.g. a pre-squash mirror; only read);
#   - the entries already in the output file, so hashes from a repository not
#     passed this time are kept;
#   - config/themes/*.toml in the working tree when no tag has those bytes yet,
#     labelled v<Cargo.toml version> before that tag exists, else "unreleased".
#     These labels are recomputed on every run, so re-run after tagging.
#
# The output is deterministic: entries sorted by theme name, then by the first
# release that shipped them; each lists every tag with those exact bytes.
#
# Usage:
#   scripts/gen-released-theme-hashes.sh [--git-dir DIR]... [--output FILE]

set -euo pipefail
export LC_ALL=C

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="$REPO_ROOT/gpy-agent/src/theme/released_hashes.rs"
THEMES_PATH="config/themes"
GIT_DIRS=("$(git -C "$REPO_ROOT" rev-parse --absolute-git-dir)")

die() {
    echo "gen-released-theme-hashes: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --git-dir) [[ -n "${2:-}" ]] || die "--git-dir needs a directory"; GIT_DIRS+=("$2"); shift 2 ;;
        --output) [[ -n "${2:-}" ]] || die "--output needs a file"; OUTPUT="$2"; shift 2 ;;
        -h|--help) sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 | cut -d' ' -f1; }
else
    die "needs sha256sum or shasum"
fi

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO_ROOT/gpy-agent/Cargo.toml" | sed -n 1p)"
[[ -n "$version" ]] || die "no version in gpy-agent/Cargo.toml"
if [[ -n "$(git -C "$REPO_ROOT" tag --list "v$version")" ]]; then
    provisional="unreleased"
else
    provisional="v$version"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
triples="$work/triples"   # "<name> <tag> <sha256>" per line
: >"$triples"

# Entries already in the output, minus provisional labels. Every hash in the
# file must parse, so a hand-reformatted table fails instead of losing entries.
if [[ -f "$OUTPUT" ]]; then
    tags=""
    parsed=0
    entry_re='^    \("([a-z0-9_-]+)", "([0-9a-f]{64})"\),$'
    while IFS= read -r line; do
        if [[ "$line" =~ ^"    // "(v.*|unreleased.*)$ ]]; then
            tags="${BASH_REMATCH[1]}"
        elif [[ "$line" =~ $entry_re ]]; then
            parsed=$((parsed + 1))
            for tag in $tags; do
                [[ "$tag" == "unreleased" || "$tag" == "$provisional" ]] && continue
                echo "${BASH_REMATCH[1]} $tag ${BASH_REMATCH[2]}" >>"$triples"
            done
            tags=""
        fi
    done <"$OUTPUT"
    found="$(grep -cE '"[0-9a-f]{64}"' "$OUTPUT" || true)"
    [[ "$parsed" -eq "$found" ]] ||
        die "parsed $parsed of the $found hashes in $OUTPUT; restore its generated layout (git checkout) and re-run"
fi

# Every release tag of every source repository.
for git_dir in "${GIT_DIRS[@]}"; do
    [[ -e "$git_dir" ]] || die "no git directory at $git_dir"
    while IFS= read -r tag; do
        [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
        while IFS= read -r path; do
            [[ "$path" == *.toml ]] || continue
            name="$(basename "$path" .toml)"
            hash="$(git --git-dir="$git_dir" cat-file blob "$tag:$path" | sha256)"
            echo "$name $tag $hash" >>"$triples"
        done < <(git --git-dir="$git_dir" ls-tree --name-only "$tag" -- "$THEMES_PATH/")
    done < <(git --git-dir="$git_dir" tag --list 'v*')
done

# Working-tree builtins no tag has shipped yet.
pending="$work/pending"
: >"$pending"
for path in "$REPO_ROOT/$THEMES_PATH"/*.toml; do
    [[ -f "$path" ]] || continue
    name="$(basename "$path" .toml)"
    hash="$(sha256 <"$path")"
    grep -q "^$name [^ ]* $hash\$" "$triples" || echo "$name $provisional $hash" >>"$pending"
done

# One entry per (name, hash) in first-release order; pending entries last.
entries="$(
    { sort -u "$triples" | sort -t' ' -k1,1 -k2,2V; cat "$pending"; } |
        awk '{
            key = $1 " " $3
            if (!(key in idx)) { idx[key] = ++n; name[n] = $1; hash[n] = $3; tags[n] = $2 }
            else { tags[idx[key]] = tags[idx[key]] " " $2 }
        }
        END { for (i = 1; i <= n; i++) printf "%s\t%06d\t%s\t%s\n", name[i], i, hash[i], tags[i] }' |
        sort -t"$(printf '\t')" -k1,1 -k2,2n
)"
[[ -n "$entries" ]] || die "no builtin theme found under $THEMES_PATH"

{
    cat <<'EOF'
//! SHA-256 of every released version of each builtin theme file
//! (`config/themes/<name>.toml`), the tags that shipped it in the comment above
//! each entry. `gpy setup` retires a user copy matching one of these
//! (`theme::retire`).
//!
//! Generated by `scripts/gen-released-theme-hashes.sh`; do not edit by hand.
//! A builtin theme change needs a re-run (`theme::retire` tests fail until
//! then), and so does each release (docs/dev/releasing.md).

/// `(builtin theme name, lowercase hex SHA-256 of the file's exact bytes)`.
// One entry per line: the generator reads its previous output back.
#[rustfmt::skip]
pub(super) const RELEASED_THEME_HASHES: &[(&str, &str)] = &[
EOF
    while IFS=$'\t' read -r name _ hash tag_list; do
        printf '    // %s\n    ("%s", "%s"),\n' "$tag_list" "$name" "$hash"
    done <<<"$entries"
    echo "];"
} >"$work/out.rs"
mv "$work/out.rs" "$OUTPUT"
echo "Wrote $(wc -l <<<"$entries" | tr -d ' ') entries to ${OUTPUT#"$REPO_ROOT"/}"
