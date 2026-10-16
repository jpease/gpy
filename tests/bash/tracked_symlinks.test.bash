#!/usr/bin/env bash
# tests/bash/tracked_symlinks.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Tracked-tree hygiene, each detector also shown firing on planted violations:
#   (a) no tracked symlink escapes the repo (judged lexically from the index)
#   (b) no tracked symlink at all (Windows checks them out as text files)
#   (c) no tracked file matches a tracked ignore rule

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"


# Lexical, not realpath: targets may not exist in this clone. Bash 3.2 compatible.
normalize_path() {
    local p="$1" out="" part old_ifs
    old_ifs="$IFS"
    set -f
    IFS='/'
    # shellcheck disable=SC2086  # intentional word split on '/' with globbing off
    set -- $p
    IFS="$old_ifs"
    set +f
    for part in "$@"; do
        case "$part" in
            '' | '.') ;;
            '..')
                case "$out" in
                    '' | '..' | */..)
                        out="${out:+$out/}.."
                        ;;
                    */*)
                        out="${out%/*}"
                        ;;
                    *)
                        out=""
                        ;;
                esac
                ;;
            *)
                out="${out:+$out/}$part"
                ;;
        esac
    done
    printf '%s' "$out"
}

# Resolve a link target relative to its directory; prints the repo-relative path,
# or an absolute target unchanged.
resolve_link() {
    local link="$1" target="$2" dir
    case "$target" in
        /*)
            printf '%s' "$target"
            return
            ;;
    esac
    dir="$(dirname "$link")"
    if [[ "$dir" == "." ]]; then
        normalize_path "$target"
    else
        normalize_path "$dir/$target"
    fi
}

# True for any absolute path or a relative one that climbs past the root.
escapes_repo() {
    local resolved="$1"
    case "$resolved" in
        /*) return 0 ;;
        '..' | ../*) return 0 ;;
        *) return 1 ;;
    esac
}

# Emit "<path>\t<target>" for every symlink in repo $1's index (what a clone
# materializes; Windows worktree entries are not symlinks).
tracked_symlinks() {
    local repo="$1" rec meta path mode sha target
    while IFS= read -r -d '' rec; do
        meta="${rec%%$'\t'*}"
        path="${rec#*$'\t'}"
        mode="${meta%% *}"
        [[ "$mode" == "120000" ]] || continue
        sha="$(printf '%s\n' "$meta" | awk '{print $2}')"
        target="$(git -C "$repo" cat-file blob "$sha" 2>/dev/null)"
        printf '%s\t%s\n' "$path" "$target"
    done < <(git -C "$repo" ls-files -s -z)
}

# Emit "<path>\t<target>" for tracked symlinks whose target escapes the repo.
escaping_symlinks() {
    local repo="$1" path target
    while IFS=$'\t' read -r path target; do
        [[ -n "$path" ]] || continue
        if escapes_repo "$(resolve_link "$path" "$target")"; then
            printf '%s\t%s\n' "$path" "$target"
        fi
    done < <(tracked_symlinks "$repo")
}

echo "--- no tracked symlink resolves outside the repository ---"
escaping="$(escaping_symlinks "$ROOT")"
if [[ -n "$escaping" ]]; then
    while IFS=$'\t' read -r path target; do
        fail "tracked symlink $path -> $target resolves outside the repository; it is a workstation-only path that lands dangling in every other clone"
    done <<<"$escaping"
fi

echo "--- no tracked symlink survives a Windows checkout ---"
all_links="$(tracked_symlinks "$ROOT")"
if [[ -n "$all_links" ]]; then
    while IFS=$'\t' read -r path target; do
        fail "tracked symlink $path -> $target materializes as a plain text file on a Windows checkout without Developer Mode or core.symlinks=true"
    done <<<"$all_links"
fi

echo "--- the detector must flag planted symlinks ---"

plant_repo() {
    local repo="$1" name="$2" target="$3"
    mkdir -p "$repo/$(dirname "$name")"
    git -C "$repo" init -q
    ln -s "$target" "$repo/$name"
    git -C "$repo" add -A
}

# An escaping symlink to a workstation-only script.
escape_repo="$WORKDIR/escape"
mkdir -p "$escape_repo"
plant_repo "$escape_repo" "scripts/list_open_issues.py" "../../elsewhere/list_open_issues.py"
if [[ -z "$(escaping_symlinks "$escape_repo")" ]]; then
    fail "detector missed a tracked symlink escaping the repository root"
fi
if [[ -z "$(tracked_symlinks "$escape_repo")" ]]; then
    fail "detector missed a tracked symlink entirely"
fi

# An absolute symlink also escapes every other clone.
abs_repo="$WORKDIR/absolute"
mkdir -p "$abs_repo"
plant_repo "$abs_repo" "scripts/tool.py" "/opt/elsewhere/tool.py"
if [[ -z "$(escaping_symlinks "$abs_repo")" ]]; then
    fail "detector missed a tracked symlink with an absolute target"
fi

# An in-repo symlink passes (a) but must fail (b).
inside_repo="$WORKDIR/inside"
mkdir -p "$inside_repo/docs"
: >"$inside_repo/docs/real.md"
plant_repo "$inside_repo" "docs/alias.md" "real.md"
if [[ -n "$(escaping_symlinks "$inside_repo")" ]]; then
    fail "detector flagged an internal symlink as escaping the repository"
fi
if [[ -z "$(tracked_symlinks "$inside_repo")" ]]; then
    fail "detector missed an internal tracked symlink; it still breaks a Windows checkout"
fi

# A symlink-free repo must yield no findings.
clean_repo="$WORKDIR/clean"
mkdir -p "$clean_repo"
git -C "$clean_repo" init -q
printf '# notes\n' >"$clean_repo/README.md"
git -C "$clean_repo" add -A
if [[ -n "$(tracked_symlinks "$clean_repo")" ]]; then
    fail "detector reported a symlink in a repository that has none"
fi


# The lexical resolver itself, since (a) rests entirely on it.
echo "--- lexical resolution ---"
check_resolve() {
    local link="$1" target="$2" expected="$3" got
    got="$(resolve_link "$link" "$target")"
    [[ "$got" == "$expected" ]] ||
        fail "resolve_link($link, $target) = '$got', expected '$expected'"
}
check_resolve "scripts/list_open_issues.py" "../../elsewhere/x.py" "../elsewhere/x.py"
check_resolve "scripts/tool.py" "../docs/tool.py" "docs/tool.py"
check_resolve "docs/a/b.md" "../../README.md" "README.md"
check_resolve "docs/a/b.md" "../../../README.md" "../README.md"
check_resolve "README.md" "docs/README.md" "docs/README.md"
check_resolve "a/b.md" "./c.md" "a/c.md"
check_resolve "a/b.md" "/etc/passwd" "/etc/passwd"
escapes_repo "../elsewhere/x.py" || fail "escapes_repo missed a '..' escape"
escapes_repo "/etc/passwd" || fail "escapes_repo missed an absolute path"
if escapes_repo "docs/tool.py"; then
    fail "escapes_repo flagged an in-repo path"
fi

echo "--- no tracked file is matched by a tracked ignore rule ---"

# `--no-index` makes check-ignore answer for tracked paths too.
while IFS= read -r ignored; do
    [[ -n "$ignored" ]] || continue
    source_rule="$(git check-ignore -v --no-index -- "$ignored" 2>/dev/null)"
    fail "tracked file '$ignored' is matched by an ignore rule ($source_rule); a fresh 'git add' would drop it and nothing would report the loss"
done < <(git ls-files | git check-ignore --stdin --no-index 2>/dev/null || true)

# The detector has to be shown firing.
drop_repo="$WORKDIR/silent-drop"
mkdir -p "$drop_repo"
git -C "$drop_repo" init -q .
printf '*.snap\n' >"$drop_repo/.gitignore"
printf 'golden\n' >"$drop_repo/golden.snap"
# Force it tracked, the way a real file ends up there: added before the
# ignore rule existed, or added with -f.
git -C "$drop_repo" add -f golden.snap .gitignore
planted_drop="$(git -C "$drop_repo" ls-files | git -C "$drop_repo" check-ignore --stdin --no-index 2>/dev/null || true)"
if [[ "$planted_drop" != "golden.snap" ]]; then
    fail "the silent-drop detector missed a planted tracked-but-ignored file (got '${planted_drop:-nothing}'); it is not doing its job"
fi
printf '*.snap\n!golden.snap\n' >"$drop_repo/.gitignore"
if [[ -n "$(git -C "$drop_repo" ls-files | git -C "$drop_repo" check-ignore --stdin --no-index 2>/dev/null || true)" ]]; then
    fail "the silent-drop detector still flags a tracked file the .gitignore un-ignores"
fi

test_finish "no escaping or tracked symlinks, no tracked-but-ignored files"
