#!/usr/bin/env bash
# tests/bash/agent_tooling_privacy_claims.test.bash
#
# Locks in the two acceptance criteria of #504 that are already satisfied, so
# they stay satisfied through the #509 fresh-repo cutover.
#
# The settled decision on #504: the agent tooling stays OUT of the public
# repository but stays in the working tree, excluded via `.git/info/exclude`
# written in the NEW repo at the #509 cutover. Private: `.agents/`, `.codex/`,
# `.raven/`, `docs/superpowers/`, `AGENTS.md`, `CLAUDE.md`. `.claude/` is
# already gitignored. That only works if the tooling is still ON DISK when the
# cutover happens -- an exclude for a path nobody kept is an exclude for
# nothing, and the paths look unused precisely because they are about to stop
# being tracked.
#
# The symlink half is a repeat offender. `scripts/list_open_issues.py` was a
# tracked symlink pointing outside the checkout: it resolved on the
# maintainer's workstation and landed as a dangling file in every other clone.
# `CLAUDE.md` was a tracked symlink to `AGENTS.md` and was converted to a
# one-line `@AGENTS.md` regular file, because git materializes a symlink on
# Windows as a plain text file holding the target path unless the user has
# Developer Mode or `core.symlinks=true`. Both are fixed; neither fix is
# defended by anything.
#
# Every assertion derives its subject from git rather than from a hardcoded
# list of known-bad paths. A new symlink added tomorrow is judged by the same
# rule that condemned `list_open_issues.py`, not by its name. The one list
# that is written out -- the six private paths -- is the decision text itself,
# and it is cross-checked against `.git/info/exclude` rather than trusted.
#
# The detector is exercised against throwaway repositories with planted
# symlinks, not only against the real tree. Asserting only that this repo is
# clean would pass just as happily against a detector that finds nothing.
#
# Deliberately NOT asserted:
#   * The contents of `.git/info/exclude`. Writing it is #509's cutover step;
#     this test only requires that whatever it already excludes stays
#     consistent with a clean `git status`.
#   * `CONTRIBUTING.md` being self-contained -- that is #501's deliverable.
#   * Anything under `.claude/`, which is gitignored and therefore invisible
#     to a tracked test. That includes the GitNexus managed block's broken
#     `.claude/skills/gitnexus/<skill>/SKILL.md` routes in `AGENTS.md`: the
#     block is vendor-generated and regenerated on the next `gitnexus` run,
#     and every path it names lives under a gitignored directory, so there is
#     no repository-visible state to assert on. Recorded on #504, left alone.
#   * Workstation paths and private codenames in tracked files -- owned by
#     #498 and asserted by tests/bash/privacy_patterns.test.bash.
#
# Asserts:
#   (a) no tracked symlink resolves outside the repository root, judged
#       lexically from the index so a target that does not exist locally is
#       still caught
#   (b) no tracked symlink exists at all, since any of them breaks a Windows
#       checkout without Developer Mode; and `CLAUDE.md` in particular is a
#       regular file that still points at an `AGENTS.md` which exists
#   (c) the detector in (a)/(b) actually fires, proven against scratch repos
#       holding a planted escaping symlink, a planted internal symlink, and a
#       planted `CLAUDE.md` symlink
#   (d) every path the #504 decision marks private is present on disk, is not
#       itself a symlink, and carries real content
#   (e) any private path already excluded by `.git/info/exclude` leaves
#       `git status` clean (inert until #509 writes those entries)
#   (f) every `SKILL.md` under the canonical `.agents/skills/` tree is
#       tracked, for as long as `.agents/` is tracked at all
#   (g) no tracked public file references a path the cutover removes
#   (h) every `.raven/` path a public file depends on is itself tracked
#   (i) no directory's contents are kept out of git solely by a `.gitignore`
#       the tool that owns the directory writes inside it
#   (j) no tracked public file references `.raven/manifest.json`, and no
#       hook or script under `.raven/git-hooks/`, `.pre-commit-config.yaml`,
#       `justfile`, or `scripts/` reads it

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# The private set from the #504 decision. This is the decision text, not a
# sample: adding a path to it is a decision change, and section (e) checks it
# against `.git/info/exclude` rather than taking it on faith.
#
# `.raven/` itself was in this list and is not any more. Nine of its ten
# tracked files are hook infrastructure the PUBLIC repository runs:
# `.pre-commit-config.yaml` dispatches three hooks out of
# `.raven/git-hooks/lib/`, the `justfile` sources a fourth, `.gitattributes`
# pins `eol=lf` on seven of them, and the hook scripts read
# `.raven/config.toml`. CONTRIBUTING.md tells every contributor to run
# `./scripts/install-hooks.sh`, so excluding the whole tree would ship a
# public repo whose documented first step installs hooks that are not there.
# Section (h) holds that line.
#
# The tenth, `.raven/manifest.json`, is the exception: a 2026-09-06 audit
# (#504) found it is machine-written installer state -- a hashed inventory
# dominated by entries under `.agents/` and `.claude/`, the very trees this
# decision keeps private -- and that no public hook or script reads it. It
# rejoined the private set on that decision even though the rest of `.raven/`
# did not. Section (j) holds that no public file references it and no public
# hook reads it.
#
# `.gemini/` and `GEMINI.md` (the Raven Gemini CLI adapter) joined the set on
# 2026-10-01: its hooks name `AGENTS.md`, `.codex/`, `docs/superpowers/` and
# `.raven/manifest.json`, so tracking it published references to everything
# above. Kept on disk, excluded via `.git/info/exclude` like the rest.
PRIVATE_PATHS=(.agents .codex docs/superpowers AGENTS.md CLAUDE.md .raven/manifest.json .gemini GEMINI.md)

# The subset above that public files may not reference at all. Same list here,
# but they are different claims: (d)/(e) are about the paths existing and being
# excluded, (g) is about nothing public pointing at them once they are gone.
CUTOVER_PATHS=(.agents/ .codex/ docs/superpowers/ AGENTS.md CLAUDE.md .gemini/ GEMINI.md)

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# --- lexical path resolution ------------------------------------------------
#
# Deliberately lexical rather than `realpath`: the failure mode being guarded
# is a symlink whose target does not exist in this clone, and realpath either
# fails or returns nothing useful for those. Written without bash-4 array
# features (negative indices, readarray) so it runs under the macOS system
# bash 3.2 as well as the CI shells.
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

# Resolve a symlink target against the directory holding the link, both
# relative to the repository root. Prints the normalized repo-relative path,
# or the target unchanged when it is absolute.
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

# True when a resolved target lands outside the repository root: any absolute
# path (which is a workstation path, broken in every other clone) or any
# relative path that climbs past the root.
escapes_repo() {
    local resolved="$1"
    case "$resolved" in
        /*) return 0 ;;
        '..' | ../*) return 0 ;;
        *) return 1 ;;
    esac
}

# Emit "<path>\t<target>" for every symlink in repo $1's index. Reads the
# index, not the worktree: the index is what a fresh clone materializes, and
# on a Windows checkout the worktree entry is not a symlink at all.
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

# --- (a) no tracked symlink escapes the repository --------------------------
echo "--- no tracked symlink resolves outside the repository ---"
escaping="$(escaping_symlinks "$ROOT")"
if [[ -n "$escaping" ]]; then
    while IFS=$'\t' read -r path target; do
        fail "tracked symlink $path -> $target resolves outside the repository; it is a workstation-only path that lands dangling in every other clone (#504)"
    done <<<"$escaping"
fi

# --- (b) no tracked symlink at all, and CLAUDE.md is a regular file ---------
echo "--- no tracked symlink survives a Windows checkout ---"
all_links="$(tracked_symlinks "$ROOT")"
if [[ -n "$all_links" ]]; then
    while IFS=$'\t' read -r path target; do
        fail "tracked symlink $path -> $target materializes as a plain text file on a Windows checkout without Developer Mode or core.symlinks=true (#504)"
    done <<<"$all_links"
fi

# CLAUDE.md is checked by name as well as by the sweep above, because the
# recorded fix was a specific conversion: symlink -> one-line pointer. The
# on-disk test keeps holding after #509 untracks the file, when the index
# sweep no longer sees it.
if [[ ! -e CLAUDE.md ]]; then
    fail "CLAUDE.md is missing; it is the pointer that gives Claude Code the AGENTS.md instructions (#504)"
elif [[ -L CLAUDE.md ]]; then
    fail "CLAUDE.md is a symlink; it was converted to a regular file so a Windows checkout gets the pointer rather than the literal text 'AGENTS.md' (#504)"
else
    pointer_target="$(sed -n 's/^@\([^[:space:]]*\).*/\1/p' CLAUDE.md | head -1)"
    if [[ -z "$pointer_target" ]]; then
        fail "CLAUDE.md carries no '@<file>' pointer; replacing the symlink with a file only works if the file points somewhere (#504)"
    elif [[ ! -e "$pointer_target" ]]; then
        fail "CLAUDE.md points at '$pointer_target', which does not exist; the pointer dangles exactly like the symlink it replaced (#504)"
    fi
fi

# --- (c) the detector fires on planted violations ---------------------------
#
# Without this, (a) and (b) are two assertions that have never been observed
# to fail, guarding two criteria that were already satisfied when they were
# written.
echo "--- the detector must flag planted symlinks ---"

plant_repo() {
    local repo="$1" name="$2" target="$3"
    mkdir -p "$repo/$(dirname "$name")"
    git -C "$repo" init -q
    ln -s "$target" "$repo/$name"
    git -C "$repo" add -A
}

# An escaping symlink: the list_open_issues.py failure mode.
escape_repo="$WORKDIR/escape"
mkdir -p "$escape_repo"
plant_repo "$escape_repo" "scripts/list_open_issues.py" "../../elsewhere/list_open_issues.py"
if [[ -z "$(escaping_symlinks "$escape_repo")" ]]; then
    fail "detector missed a tracked symlink escaping the repository root"
fi
if [[ -z "$(tracked_symlinks "$escape_repo")" ]]; then
    fail "detector missed a tracked symlink entirely"
fi

# An absolute symlink is also outside every clone but the author's, even
# though it has no '..' in it.
abs_repo="$WORKDIR/absolute"
mkdir -p "$abs_repo"
plant_repo "$abs_repo" "scripts/tool.py" "/opt/elsewhere/tool.py"
if [[ -z "$(escaping_symlinks "$abs_repo")" ]]; then
    fail "detector missed a tracked symlink with an absolute target"
fi

# A symlink that stays inside the repo is acceptable to (a) but must still be
# caught by (b), which is the Windows criterion. This case is what keeps (a)
# and (b) from collapsing into one rule.
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

# A repo with no symlinks must produce no findings, or every assertion above
# is trivially satisfied by a detector that flags everything.
clean_repo="$WORKDIR/clean"
mkdir -p "$clean_repo"
git -C "$clean_repo" init -q
printf '@AGENTS.md\n' >"$clean_repo/CLAUDE.md"
git -C "$clean_repo" add -A
if [[ -n "$(tracked_symlinks "$clean_repo")" ]]; then
    fail "detector reported a symlink in a repository that has none"
fi

# The CLAUDE.md-specific check must reject the symlink form it replaced.
claude_repo="$WORKDIR/claude"
mkdir -p "$claude_repo"
printf '# AGENTS\n' >"$claude_repo/AGENTS.md"
plant_repo "$claude_repo" "CLAUDE.md" "AGENTS.md"
if [[ -z "$(tracked_symlinks "$claude_repo")" ]]; then
    fail "detector missed a CLAUDE.md restored as a symlink to AGENTS.md"
fi
if [[ ! -L "$claude_repo/CLAUDE.md" ]]; then
    fail "planted CLAUDE.md symlink was not created; the case above proves nothing"
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
check_resolve "CLAUDE.md" "AGENTS.md" "AGENTS.md"
check_resolve "a/b.md" "./c.md" "a/c.md"
check_resolve "a/b.md" "/etc/passwd" "/etc/passwd"
escapes_repo "../elsewhere/x.py" || fail "escapes_repo missed a '..' escape"
escapes_repo "/etc/passwd" || fail "escapes_repo missed an absolute path"
if escapes_repo "docs/tool.py"; then
    fail "escapes_repo flagged an in-repo path"
fi

# --- (d) private paths are present on disk ----------------------------------
#
# The exclude at the #509 cutover needs something to exclude, and these paths
# look deletable to anyone who does not know they are about to stop being
# tracked.
echo "--- #504 private paths present on disk ---"
for private in "${PRIVATE_PATHS[@]}"; do
    before_failures=$failures
    if [[ ! -e "$private" ]]; then
        fail "private path '$private' is missing; #504 keeps the agent tooling in the working tree and excludes it at the #509 cutover, so deleting it loses the tooling rather than hiding it"
        continue
    fi
    if [[ -L "$private" ]]; then
        fail "private path '$private' is a symlink; the tooling has to live in the tree, not point at somewhere outside it (#504)"
        continue
    fi
    if [[ -d "$private" ]]; then
        if [[ -z "$(find "$private" -type f -print -quit 2>/dev/null)" ]]; then
            fail "private path '$private' is an empty directory; it holds no tooling to keep (#504)"
        fi
    elif [[ ! -s "$private" ]]; then
        fail "private path '$private' is empty (#504)"
    fi
    if [[ $failures -eq $before_failures ]]; then
        echo "  private path '$private' present (PASS)"
    fi
done

# --- (e) an already-excluded private path leaves git status clean -----------
#
# Inert until #509 writes those entries. After it, this is the machine-checked
# half of "git status clean with all tooling present on disk": a path that is
# excluded but still shows up in `git status` means the exclude did not take.
echo "--- excluded private paths do not appear in git status ---"
for private in "${PRIVATE_PATHS[@]}"; do
    [[ -e "$private" ]] || continue
    if git check-ignore -q --no-index -- "$private" 2>/dev/null; then
        status_out="$(git status --porcelain --untracked-files=all -- "$private" 2>/dev/null)"
        if [[ -n "$status_out" ]]; then
            fail "private path '$private' is excluded but still reported by git status; the exclude has not taken (#504/#509)"
            printf '%s\n' "$status_out" | sed 's/^/  /'
        fi
    fi
done

# --- (f) canonical skills are tracked ---------------------------------------
#
# AGENTS.md names `.agents/skills/` the canonical home for reusable skills.
# #504's comment recorded the six `gitnexus-*` skill directories as untracked;
# they are tracked now. Derived from the filesystem rather than from that list,
# so a seventh skill added untracked turns this red too.
#
# Skipped once `.agents/` stops being tracked at all, which is what the #509
# cutover does to it.
echo "--- canonical .agents/skills content is tracked ---"
if [[ -d .agents/skills ]]; then
    if [[ -n "$(git ls-files .agents/)" ]]; then
        untracked_skills="$(git ls-files --others --exclude-standard -- '.agents/skills/*/SKILL.md')"
        if [[ -n "$untracked_skills" ]]; then
            fail "SKILL.md files under the canonical .agents/skills/ tree are untracked; they reach no other clone (#504)"
            printf '%s\n' "$untracked_skills" | sed 's/^/  /'
        fi
        skill_count="$(git ls-files -- '.agents/skills/*/SKILL.md' | grep -c . || true)"
        if [[ "$skill_count" -eq 0 ]]; then
            fail ".agents/skills/ exists but tracks no SKILL.md; AGENTS.md calls it the canonical skill location (#504)"
        fi
    else
        echo "  .agents/ is no longer tracked (#509 cutover); skipping"
    fi
else
    fail ".agents/skills/ is missing; AGENTS.md names it the canonical location for reusable skills (#504)"
fi

# --- (g) no public file references a path the cutover removes ---------------
#
# The #509 cutover deletes these paths from the public repository's history.
# A tracked public file that still points at one of them ships a reference
# nobody can follow -- which is the exact argument #504 used to move AGENTS.md
# private in the first place, applied to everything left behind.
#
# Three kinds of file are exempt. Ignore and attribute rules NAME a path rather
# than following it, and keep working when their subject is absent. The two
# bash suites take the private set as their subject and cannot assert on it
# without spelling it out. And `.raven/` is vendored: its hooks act on
# `AGENTS.md`/`CLAUDE.md` by name and skip them when absent (verified for the
# managed-block hook on #504), and `.raven/manifest.json` itself inventories
# the private tooling paths by design -- that inventory is exactly why the
# manifest joined the private set on 2026-09-06 (#504). Editing any of that is
# undone by the next Raven upgrade, which is the same reason #504 left the
# GitNexus managed block alone. Section (j) below checks the reverse
# direction: that nothing public references `.raven/manifest.json`.
reference_exempt() {
    case "$1" in
        .ignore | .gitignore | .gitattributes) return 0 ;;
        .raven/*) return 0 ;;
        tests/bash/agent_tooling_privacy_claims.test.bash) return 0 ;;
        tests/bash/contributing_workflow_docs.test.bash) return 0 ;;
        *) return 1 ;;
    esac
}

# Prints one "file<TAB>pattern" line per reference found. Split out from the
# loop so the planted-violation check below drives the same code the real tree
# is judged by.
private_references_in() {
    local file="$1" pattern
    for pattern in "${CUTOVER_PATHS[@]}"; do
        if grep -qF -- "$pattern" "$file" 2>/dev/null; then
            printf '%s\t%s\n' "$file" "$pattern"
        fi
    done
}

echo "--- no public file references a path the cutover removes ---"
while IFS= read -r tracked; do
    [[ -f "$tracked" ]] || continue
    reference_exempt "$tracked" && continue
    case "$tracked" in
        .agents/* | .codex/* | docs/superpowers/* | AGENTS.md | CLAUDE.md) continue ;;
    esac
    while IFS=$'\t' read -r hit_file hit_pattern; do
        [[ -n "$hit_file" ]] || continue
        fail "public file '$hit_file' references '$hit_pattern', which the #509 cutover removes from the public repository"
    done < <(private_references_in "$tracked")
done < <(git ls-files)

# The detector has to be shown firing: asserting only that the tree is clean
# passes just as well against a grep that never matches.
planted_ref="$WORKDIR/planted-reference.md"
printf 'Setup steps live in AGENTS.md, see also .codex/ for the rest.\n' >"$planted_ref"
planted_hits="$(private_references_in "$planted_ref" | wc -l | tr -d ' ')"
if [[ "$planted_hits" -ne 2 ]]; then
    fail "the private-reference detector found $planted_hits of 2 planted references; it is not doing its job"
fi
clean_ref="$WORKDIR/clean-reference.md"
printf 'See CONTRIBUTING.md for the quality gates.\n' >"$clean_ref"
if [[ -n "$(private_references_in "$clean_ref")" ]]; then
    fail "the private-reference detector flags a file that references nothing private"
fi

# --- (h) public files' .raven dependencies are tracked ----------------------
#
# The other half of taking `.raven/` back out of the private set. Every
# `.raven/` path a public file names has to be tracked, or the public repo
# documents hooks it does not ship. Derived from what the files actually
# reference, so a new hook wired into `.pre-commit-config.yaml` is held to the
# same rule without being named here.
echo "--- public .raven dependencies are tracked ---"
while IFS= read -r tracked; do
    [[ -f "$tracked" ]] || continue
    case "$tracked" in
        # `.raven/` describes itself; the private trees go away at the cutover
        # and are free to name Raven's runtime state (`.raven/session.md` and
        # friends), which is written at runtime and tracked nowhere. Ignore
        # rules name paths without following them.
        .raven/* | .agents/* | .codex/* | docs/superpowers/*) continue ;;
        .ignore | .gitignore) continue ;;
        tests/bash/agent_tooling_privacy_claims.test.bash) continue ;;
    esac
    while IFS= read -r dep; do
        [[ -n "$dep" ]] || continue
        # Trailing punctuation from prose: `.raven/git-hooks/` in a sentence.
        dep="${dep%.}"
        dep="${dep%,}"
        if [[ -z "$(git ls-files -- "$dep")" ]]; then
            fail "public file '$tracked' depends on '$dep', which is not tracked; the public repo would document a hook it does not ship (#504)"
        fi
    done < <(grep -oE '\.raven/[A-Za-z0-9_./-]+' "$tracked" 2>/dev/null | sort -u)
done < <(git ls-files)

# --- (i) no directory self-ignores its way out of the public repo -----------
#
# The #509 cutover recreates history from one `git add` of this working tree.
# Anything git does not ignore at that moment enters public history, and there
# is no second chance: the repo is public from its first push.
#
# `.remember/` (session notes), `.ruby-lsp/` and `.ruff_cache/` were held back
# only by a `.gitignore` the owning tool writes inside the directory --
# `.remember/.gitignore` is two bytes, `*`. That works until the tool stops
# writing it, and nothing in this repository would notice: `git status` reads
# clean either way, because a clean status is exactly what the nested file
# buys. The root `.gitignore` now names all three, so the rule belongs to the
# repository rather than to whichever tool happened to run last.
#
# Derived from the tree, not from those three names: any directory that gets
# its ignore from a file inside itself is judged the same way.
echo "--- no directory is kept out of git only by its own .gitignore ---"

# Prints the ignore-source path for the first file in $1, or nothing when the
# directory is empty or not ignored. Shared with the planted-violation check
# below so both run the same detector.
self_ignore_source() {
    local dir="$1" probe src
    probe="$(find "$dir" -maxdepth 1 -type f 2>/dev/null | head -1)"
    [[ -n "$probe" ]] || return 0
    src="$(git check-ignore -v -- "$probe" 2>/dev/null | cut -d: -f1)"
    [[ -n "$src" ]] || return 0
    case "$src" in
        "$dir"/*) printf '%s\n' "$src" ;;
    esac
}

while IFS= read -r dir; do
    src="$(self_ignore_source "$dir")"
    [[ -n "$src" ]] || continue
    fail "'$dir' is kept out of git only by '$src', which the tool that owns the directory writes; the root .gitignore has to name it or the #509 cutover commits it (#504)"
done < <(find . -mindepth 1 -maxdepth 3 -type d -not -path './.git' -not -path './.git/*' 2>/dev/null | sed 's|^\./||')

# The detector has to be shown firing. A scratch repo, so the planted
# directory cannot be confused with anything in the real tree.
planted_repo="$WORKDIR/self-ignore"
mkdir -p "$planted_repo/toolstate"
git -C "$planted_repo" init -q .
printf '*\n' >"$planted_repo/toolstate/.gitignore"
printf 'session notes\n' >"$planted_repo/toolstate/notes.md"
planted_src="$(cd "$planted_repo" && self_ignore_source toolstate)"
if [[ "$planted_src" != "toolstate/.gitignore" ]]; then
    fail "the self-ignore detector missed a planted nested .gitignore (got '${planted_src:-nothing}'); it is not doing its job"
fi
printf 'toolstate/\n' >"$planted_repo/.gitignore"
if [[ -n "$(cd "$planted_repo" && self_ignore_source toolstate)" ]]; then
    fail "the self-ignore detector still flags a directory the root .gitignore names"
fi

# --- (j) nothing public references or reads .raven/manifest.json -----------
#
# `.raven/manifest.json` joined the private set on 2026-09-06 (#504): a
# machine-written installer inventory dominated by entries under `.agents/`
# and `.claude/`, the trees this decision keeps private, published for no
# runtime benefit -- `rg -n manifest .raven/git-hooks justfile scripts` (the
# audit behind that decision) found no consumer. Unlike `.raven/config.toml`
# and the hook scripts, it is not a public dependency, so it gets the
# opposite check from section (h): nothing public may name it at all.
#
# The private trees (`.agents/`, `.codex/`, `docs/superpowers/`, `AGENTS.md`,
# `CLAUDE.md`) and `.raven/` itself are exempt as sources, same as section
# (g): they are either leaving the public repository at the #509 cutover or
# are the vendored tree that inventories it, and a reference from either one
# is not a reference from a file that stays public.
manifest_referenced_by() {
    local file="$1"
    case "$file" in
        .raven/* | .agents/* | .codex/* | docs/superpowers/*) return 1 ;;
        AGENTS.md | CLAUDE.md) return 1 ;;
        # This suite names the path it is checking for, in comments, in the
        # PRIVATE_PATHS entry, and in this very grep pattern -- it is the
        # detector, not a reference from a file that stays public.
        tests/bash/agent_tooling_privacy_claims.test.bash) return 1 ;;
    esac
    grep -qF -- ".raven/manifest.json" "$file" 2>/dev/null
}

echo "--- no public file references .raven/manifest.json ---"
while IFS= read -r tracked; do
    [[ -f "$tracked" ]] || continue
    if manifest_referenced_by "$tracked"; then
        fail "public file '$tracked' references '.raven/manifest.json', which #504 excludes from the public snapshot"
    fi
done < <(git ls-files)

# The detector has to be shown firing, same as (g)'s and (i)'s.
planted_manifest_ref="$WORKDIR/planted-manifest-reference.md"
printf 'The install inventory lives in .raven/manifest.json.\n' >"$planted_manifest_ref"
if ! manifest_referenced_by "$planted_manifest_ref"; then
    fail "the .raven/manifest.json reference detector did not fire against a planted reference; it is not doing its job"
fi
clean_manifest_ref="$WORKDIR/clean-manifest-reference.md"
printf 'Hook settings live in .raven/config.toml.\n' >"$clean_manifest_ref"
if manifest_referenced_by "$clean_manifest_ref"; then
    fail "the .raven/manifest.json reference detector flagged a file that does not reference the manifest"
fi

echo "--- no public hook or script reads .raven/manifest.json ---"
while IFS= read -r hook_file; do
    [[ -f "$hook_file" ]] || continue
    if grep -qF -- "manifest.json" "$hook_file" 2>/dev/null; then
        fail "'$hook_file' reads manifest.json; #504 keeps .raven/manifest.json out of the public snapshot because no hook or script depends on it"
    fi
done < <(git ls-files -- '.raven/git-hooks/*' '.pre-commit-config.yaml' 'justfile' 'scripts/*')

# --- (k) no tracked file would be dropped by the cutover's fresh `git add` ---
#
# Section (i) catches a directory kept out of git by its own nested ignore.
# This is the inverse, and the invariant the #509 cutover actually rests on:
# git applies .gitignore only to *untracked* files, so a tracked file whose
# path an ignore rule matches stays tracked here and is silently dropped by
# the single `git add` that recreates history in the new repository. Nothing
# reports it -- the file just stops existing, and the loss is only visible
# after the old history is gone.
#
# Three rules were live when this check was written (2026-09-07), between
# them dropping six tracked files: an unanchored `bench-results/` that
# swallowed the tracked docs archive, an `*.snap` whose negation named only
# `gpy-agent/tests/snapshots/` and not the golden theme-export snapshots in
# `gpy-agent/src/theme/snapshots/`, and a nested `benchmarks/.gitignore`
# whose `*.json` matched the tracked `baseline.json` that run.sh reads.
#
# The #504 private paths are the deliberate exception: they are excluded via
# `.git/info/exclude` in the new repository precisely so the fresh add skips
# them. They are excluded there, never in a tracked .gitignore, so this check
# reads only tracked ignore files and does not need to special-case them.
echo "--- no tracked file is matched by a tracked ignore rule ---"

# `--no-index` makes check-ignore answer for tracked paths too; without it
# git reports nothing, because a tracked path is never "ignored" today. That
# is the whole blind spot this section exists to close.
while IFS= read -r ignored; do
    [[ -n "$ignored" ]] || continue
    source_rule="$(git check-ignore -v --no-index -- "$ignored" 2>/dev/null)"
    fail "tracked file '$ignored' is matched by an ignore rule ($source_rule); the #509 cutover's fresh 'git add' would drop it and nothing would report the loss"
done < <(git ls-files | git check-ignore --stdin --no-index 2>/dev/null || true)

# The detector has to be shown firing, same as (g)'s, (i)'s and (j)'s.
drop_repo="$WORKDIR/silent-drop"
mkdir -p "$drop_repo"
git -C "$drop_repo" init -q .
printf '*.snap\n' >"$drop_repo/.gitignore"
printf 'golden\n' >"$drop_repo/golden.snap"
# Force it tracked, reproducing how the real six got there: added before the
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

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
