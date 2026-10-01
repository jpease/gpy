#!/usr/bin/env fish
# tests/fish/starship_preset_import_parity.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test: `gpy theme import` render parity against a REAL, unmodified Starship
# preset (Starship's official "Pure Preset")
# ---------------------------------------------------------------------------
# `tests/fish/starship_parity.test.fish` proves GPY's hand-written `starship`
# theme (config/themes/starship.toml) matches Starship's own defaults. This
# test instead proves the *import pipeline* end to end: `gpy theme import`
# converts a genuine third-party preset — not a GPY-authored theme — into a
# palette + theme that renders like native Starship for the same config file.
#
# Fixture: tests/fish/fixtures/pure_preset.toml is a verbatim copy of
# Starship's official "Pure Preset" (docs/public/presets/toml/pure-preset.toml
# @ starship/starship). It is used as BOTH the source fed to `gpy theme
# import` AND the native Starship config (via STARSHIP_CONFIG) — there is no
# separate hand-curated baseline (contrast starship_match.toml): both sides
# derive from the exact same real, unmodified upstream file.
#
# HOW IT WORKS
#   * GPY side runs the actual import pipeline in a fully ISOLATED config dir
#     (temp $XDG_CONFIG_HOME/$HOME; the real user config is never touched):
#       gpy theme import pure_preset.toml --name pure-preset --force --apply-layout
#       gpy theme use pure-preset --force
#       gpy palette use pure-preset
#     exactly as a user would after importing a preset. Each segment is then
#     rendered individually via `gpy-agent oneshot <segment> --format ansi`.
#   * Starship side renders each segment individually via `starship module
#     <name>` (not `starship prompt`) with STARSHIP_CONFIG pointing at the
#     same pure_preset.toml. Rendering per-segment (rather than the single
#     concatenated module line the sibling test uses) is what lets this test
#     gate everything that legitimately matches while precisely excluding
#     only the specific, documented divergence points below — instead of an
#     all-or-nothing pass/fail per fixture.
#   * Verified equivalent to reading the segment out of a full `starship
#     prompt` line: invoking a module standalone frames it with its own
#     leading/trailing SGR reset, which real `starship prompt` sometimes
#     omits between adjacent modules — but this never changes the ACTIVE
#     colour on any visible glyph, so it is invisible to both comparisons
#     below (colour-signature ignores whitespace; a reset immediately
#     followed by a fresh colour code before the next glyph is equivalent to
#     no reset at all).
#   * Parity is checked two ways per segment: (1) visible text (ANSI
#     stripped, trimmed), and (2) a per-character COLOUR signature (see
#     `__spi_color_sig`) — same two-pronged method as starship_parity.test.fish.
#
# NAMESPACING
#   `scripts/test_fish.sh` runs every `tests/fish/*.test.fish` file in its own
#   `fish -c '... source FILE'` invocation (verified by reading the runner),
#   so this file's functions/globals never coexist with starship_parity.
#   test.fish's `__sp_*`/`SP_*` names in the same process. This file still
#   uses a distinct `__spi_*`/`SPI_*`/`spi_*` namespace anyway — defence in
#   depth against a future change to the runner (e.g. sourcing multiple test
#   files in one process) or manual `source`-ing both files interactively.
#
# GATING (must hold; a regression fails the suite)
#   Per-segment text+colour parity for username, hostname, directory, git,
#   duration, language (python), and character, for every built-in fixture
#   (plain non-git dir, clean git repo, dirty git repo) — EXCEPT the one
#   documented, empirically-confirmed divergence point below, which is
#   reported (with exact escapes) but never gated.
#
# DELIBERATE EXCLUSION (inherited from the preset itself, nothing to gate)
#   * Pure Preset's format has no `package` module (unlike starship_parity.
#     test.fish's baseline, which explicitly disables it) — nothing to
#     exclude here.
#
# KNOWN REMAINING DIVERGENCE (exempted from gating; reported every run)
#
#   1. git status glyphs — "git" segment, DIRTY fixture only. Pure Preset's
#      [git_status] format is `"[[(*$conflicted$untracked...)](218)
#      ($ahead_behind$stashed)]($style)"`: a literal `*` prefix ahead of a
#      group of per-flag vars, each configured to an invisible zero-width
#      space (U+200B) except `stashed = "≡"`. Reproducing this byte-for-byte
#      would require GPY to interpret git_status as a small format-template
#      engine (literal text interleaved with conditionally-empty groups, plus
#      one arbitrary-string glyph per flag), not just more `GitTheme` icon
#      fields — `GitTheme` already has one override icon per bucket
#      (staged/unstaged/untracked/conflicts/stash — see model.rs), but
#      `status_text()` (`git_resolver.rs`) always renders a fixed-order
#      aggregate of visible icon+count pairs, with no literal-text or
#      group-suppression concept, and the Starship importer
#      (`collapse_granular_status_flags` / `translate_git` in
#      `import/starship/modules.rs`) only rewrites the format template's
#      variable names — it never reads the flags' own glyph values
#      (`conflicted`/`untracked`/.../`stashed` keys) at all. Confirmed
#      empirically: on the dirty fixture, GPY renders visible aggregate icons
#      (`✱1?1`-style) where Starship renders `*` + near-invisible
#      zero-width-space glyphs. Investigated in #350 (fixed GPY showing NO
#      dirty indicator at all) and #369 (root-caused the remaining glyph
#      mismatch and accepted it as a permanent, documented limitation of the
#      aggregate `$status` model — not pursuing a format-template engine for
#      this). Branch name, directory, duration, language, and character
#      segments on the SAME dirty fixture are still gated normally — only the
#      "git" segment (which combines branch+status into one GPY-rendered
#      string, see `translate_git`) is exempted, and only on this one
#      fixture; the clean fixture's "git" segment (no status glyphs to show)
#      gates as usual.
#
#   Previously documented here and now FIXED (both gate normally as of
#   #355/#356):
#     - directory text (plain non-git fixture): `translate_directory_layout`
#       now always recommends `display = "truncated"` + `truncate_to_repo =
#       true`, matching Starship's actual default behavior, regardless of
#       whether the source `[directory]` table overrides truncation_length/
#       truncation_symbol.
#     - character segment colour (all fixtures): `translate_character` now
#       derives the `bold` attribute from the source's actual
#       `success_symbol`/`error_symbol` styles instead of hardcoding it.
#
#   Additionally, `git_state` (rebase/merge progress) has no GPY equivalent
#   and is dropped at import time with a warning (visible if you drop
#   `>/dev/null 2>&1` from the setup calls below) — this cannot matter for
#   gating since none of the built-in fixtures are mid-rebase/merge.
#
#   Pass extra directories as arguments for ad-hoc test beds (report-only,
#   no gating — there's no fixture label to look up an exemption by):
#       fish tests/fish/starship_preset_import_parity.test.fish ~/some/repo
#
# Skips cleanly (exit 0) when `starship` is not installed (e.g. CI).

# Resolve to an ABSOLUTE repo root: helpers below cd into fixture dirs before
# using $spi_match_cfg / the binaries, so relative paths (status dirname is
# relative when the suite sources this file) would break after the cd.
set -l test_root (status dirname)/../..
set -g spi_repo_root (path resolve $test_root)

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
# Shared skip contract (#650): exits 0 locally, fails under CI.
source (dirname (status filename))/../lib/test_helpers.fish
if not command -q starship
    test_skip "starship not installed — parity comparison unavailable"
end

# Globals (not `set -l`): helper functions below must see these.
set -g spi_gpy_bin $spi_repo_root/gpy-agent/target/debug/gpy
set -g spi_agent_bin $spi_repo_root/gpy-agent/target/debug/gpy-agent
for bin in $spi_gpy_bin $spi_agent_bin
    if not test -x $bin
        echo "❌ binary not found at $bin"
        echo "   Build it first: (cd gpy-agent && cargo build)"
        exit 1
    end
end

# The real, unmodified upstream preset — used as BOTH the import source and
# the native Starship config (via STARSHIP_CONFIG). See file header.
set -g spi_match_cfg $spi_repo_root/tests/fish/fixtures/pure_preset.toml
if not test -f $spi_match_cfg
    echo "❌ Pure Preset fixture missing: $spi_match_cfg"
    exit 1
end

# Fixed inputs so both renderers see identical dynamic context.
set -g SPI_STATUS 0
set -g SPI_DURATION 5000 # > Starship's 2s threshold so the duration module shows
set -g spi_hostname $hostname
set -g spi_username (whoami)

# ---------------------------------------------------------------------------
# Isolated GPY config: run the real import pipeline end to end, exactly as a
# user would after downloading a third-party preset. Never touches the real
# user config.
# ---------------------------------------------------------------------------
set -g SPI_ISO (mktemp -d)
set -gx XDG_CONFIG_HOME $SPI_ISO
set -gx HOME $SPI_ISO
set -gx MISE_DISABLE 1

if not $spi_gpy_bin theme import $spi_match_cfg --name pure-preset --force --apply-layout >/dev/null 2>&1
    echo "❌ 'gpy theme import' failed in isolated config"
    rm -rf $SPI_ISO
    exit 1
end
if not $spi_gpy_bin theme use pure-preset --force >/dev/null 2>&1
    echo "❌ 'gpy theme use pure-preset --force' failed in isolated config"
    rm -rf $SPI_ISO
    exit 1
end
if not $spi_gpy_bin palette use pure-preset >/dev/null 2>&1
    echo "❌ 'gpy palette use pure-preset' failed in isolated config"
    rm -rf $SPI_ISO
    exit 1
end

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Strip ANSI SGR escape sequences (\e[...m) from stdin/args.
function __spi_strip
    string replace -ra '\x1b\[[0-9;]*m' '' -- $argv
end

# Trim surrounding whitespace (renderers differ on leading/trailing padding).
function __spi_trim
    string trim -- $argv
end

# Colour signature: the active SGR code for each NON-space visible character,
# in render order (one `code|char` per line). Colour-aware comparison that
# stays tolerant of two harmless differences between the two renderers: which
# side of a reset a space sits on (a space has no visible colour), and a
# reset-then-recolour immediately before the next glyph vs no reset at all
# (same active colour either way). See starship_parity.test.fish's
# `__sp_color_sig` (identical logic, renamed to avoid any chance of collision
# — see NAMESPACING above).
function __spi_color_sig --argument-names raw
    set -l esc (printf '\e')
    set -l tokpat $esc'\[[0-9;]*m|[^'$esc']+'
    set -l sgrpat '^'$esc'\[[0-9;]*m$'
    set -l active ""
    for tok in (string match -ra -- $tokpat $raw)
        if string match -qr -- $sgrpat $tok
            set -l code (string replace -r -- '^'$esc'\[([0-9;]*)m$' '$1' $tok)
            if test "$code" = "" -o "$code" = 0
                set active ""
            else
                set active $code
            end
        else
            for ch in (string split '' -- $tok)
                test "$ch" = " "; and continue
                echo "$active|$ch"
            end
        end
    end
end

# Render one GPY segment for a directory. Echoes the raw (ANSI) output, or
# nothing when the segment does not apply (empty or JSON error response).
function __spi_gpy_seg --argument-names seg dir
    set -l out
    switch $seg
        case directory git lang
            set out ($spi_agent_bin oneshot $seg --cwd $dir --format ansi --not-last 2>/dev/null | string collect)
        case duration
            set out ($spi_agent_bin oneshot duration --duration-ms $SPI_DURATION --format ansi --not-last 2>/dev/null | string collect)
        case hostname
            set out ($spi_agent_bin oneshot hostname --hostname $spi_hostname --format ansi --not-last 2>/dev/null | string collect)
        case username
            set out ($spi_agent_bin oneshot username --username $spi_username --format ansi --not-last 2>/dev/null | string collect)
        case character
            set out ($spi_agent_bin oneshot character --exit-code $SPI_STATUS --format ansi 2>/dev/null | string collect)
    end
    test -n "$out"; or return 0
    # Skip JSON error responses (e.g. "Not in a git repository").
    string match -qr '^\s*\{' -- $out; and return 0
    printf '%s' $out
end

# Render the equivalent native-Starship segment for a directory via
# `starship module <name>` (not `starship prompt` — see file header for why
# per-segment rendering is equivalent). cwd-dependent modules run inside a
# child `sh -c` so cd doesn't fire fish PWD hooks (mirrors
# starship_parity.test.fish's `__sp_starship`); cwd-independent ones
# (username/hostname/duration/character) run directly with STARSHIP_CONFIG
# scoped via `set -lx`.
function __spi_sp_seg --argument-names seg dir
    switch $seg
        case username
            set -lx STARSHIP_CONFIG $spi_match_cfg
            starship module username 2>/dev/null | string collect
        case hostname
            set -lx STARSHIP_CONFIG $spi_match_cfg
            starship module hostname 2>/dev/null | string collect
        case duration
            set -lx STARSHIP_CONFIG $spi_match_cfg
            starship module cmd_duration -d $SPI_DURATION 2>/dev/null | string collect
        case character
            set -lx STARSHIP_CONFIG $spi_match_cfg
            starship module character -s $SPI_STATUS 2>/dev/null | string collect
        case directory
            sh -c 'cd "$1" || exit 1; STARSHIP_CONFIG="$2" starship module directory' \
                sh $dir $spi_match_cfg 2>/dev/null | string collect
        case lang
            sh -c 'cd "$1" || exit 1; STARSHIP_CONFIG="$2" starship module python' \
                sh $dir $spi_match_cfg 2>/dev/null | string collect
        case git
            # GPY's "git" segment combines Starship's git_branch + git_state +
            # git_status into one rendered string (see translate_git);
            # concatenate the three native modules to compare like for like.
            sh -c '
                cd "$1" || exit 1
                STARSHIP_CONFIG="$2" starship module git_branch
                STARSHIP_CONFIG="$2" starship module git_state
                STARSHIP_CONFIG="$2" starship module git_status
            ' sh $dir $spi_match_cfg 2>/dev/null | string collect
    end
end

# ---------------------------------------------------------------------------
# Comparison + report for one (fixture, segment) pair. Prints the report and
# sets globals SPI_TEXT_MATCH / SPI_COLOR_MATCH (1 = identical).
# ---------------------------------------------------------------------------
function __spi_compare_segment --argument-names label seg dir
    set -l gpy_raw (__spi_gpy_seg $seg $dir)
    set -l sp_raw (__spi_sp_seg $seg $dir)
    set -l gpy_v (__spi_trim (__spi_strip $gpy_raw))
    set -l sp_v (__spi_trim (__spi_strip $sp_raw))
    set -l gpy_sig (__spi_color_sig $gpy_raw | string collect)
    set -l sp_sig (__spi_color_sig $sp_raw | string collect)

    set -g SPI_TEXT_MATCH 0
    set -g SPI_COLOR_MATCH 0
    test "$gpy_v" = "$sp_v"; and set -g SPI_TEXT_MATCH 1
    test "$gpy_sig" = "$sp_sig"; and set -g SPI_COLOR_MATCH 1

    echo "── $label / $seg ──"
    if test "$SPI_TEXT_MATCH" = 1; and test "$SPI_COLOR_MATCH" = 1
        echo "  ✅ identical (text + colour): '$sp_v'"
        return 0
    end
    echo "  ⚠️  differ"
    if test "$SPI_TEXT_MATCH" = 0
        echo "     text     starship : '$sp_v'"
        echo "     text     gpy      : '$gpy_v'"
    else
        echo "     text matches; COLOUR differs (see escapes below)"
    end
    echo "     escapes  starship : "(printf '%s' $sp_raw | cat -v)
    echo "     escapes  gpy      : "(printf '%s' $gpy_raw | cat -v)
end

# Record a gating invariant outcome. $1 = "ok"|"fail", $2 = description.
function __spi_gate --argument-names ok desc
    if test "$ok" = ok
        echo "     ✔ gate: $desc"
    else
        echo "     ✘ GATE FAILED: $desc"
        set -g SPI_FAILURES (math $SPI_FAILURES + 1)
    end
end

# Documented, empirically-confirmed exemptions from gating (see KNOWN
# REMAINING DIVERGENCE in the file header). Returns 0 (exempt) / 1 (gate it).
function __spi_exempt_text --argument-names label seg
    # #1: git_status format-template parity (literal text + per-flag glyphs)
    # is accepted as a permanent limitation (#369); only the dirty fixture
    # actually shows any status glyphs to diverge on.
    test "$label" = "dirty git repo"; and test "$seg" = git; and return 0
    return 1
end

function __spi_exempt_color --argument-names label seg
    __spi_exempt_text $label $seg
end

# ---------------------------------------------------------------------------
# Build deterministic fixtures (same three shapes as starship_parity.test.fish).
# ---------------------------------------------------------------------------
function __spi_mkrepo --argument-names dir
    mkdir -p $dir
    git -C $dir init -q
    git -C $dir config user.email parity@test.local
    git -C $dir config user.name parity
    git -C $dir symbolic-ref HEAD refs/heads/main
end

set -l fixtures_root (mktemp -d)

# Plain non-git directory.
set -l fx_plain $fixtures_root/plain
mkdir -p $fx_plain

# Clean git repo (trivial commit).
set -l fx_clean $fixtures_root/clean
__spi_mkrepo $fx_clean
echo hello >$fx_clean/README.md
git -C $fx_clean add -A
git -C $fx_clean commit -qm init

# Dirty git repo — exercises git status (staged/unstaged/untracked flags).
set -l fx_dirty $fixtures_root/dirty
__spi_mkrepo $fx_dirty
echo original >$fx_dirty/a.txt
git -C $fx_dirty add -A
git -C $fx_dirty commit -qm init
echo modified >>$fx_dirty/a.txt
echo new >$fx_dirty/untracked.txt

# ---------------------------------------------------------------------------
# Run.
# ---------------------------------------------------------------------------
set -g SPI_FAILURES 0

echo "=== Gating fixtures (starship "(starship --version | head -1)") ==="
for pair in "plain non-git dir|$fx_plain" "clean git repo|$fx_clean" "dirty git repo|$fx_dirty"
    set -l label (string split -m1 '|' -- $pair)[1]
    set -l dir (string split -m1 '|' -- $pair)[2]
    echo
    echo "=== $label ($dir) ==="
    for seg in username hostname directory git duration lang character
        __spi_compare_segment $label $seg $dir
        if __spi_exempt_text $label $seg
            echo "     ⏭  text exempt (KNOWN REMAINING DIVERGENCE — see file header)"
        else
            test "$SPI_TEXT_MATCH" = 1; and __spi_gate ok "$seg text parity"; or __spi_gate fail "$seg text parity"
        end
        if __spi_exempt_color $label $seg
            echo "     ⏭  colour exempt (KNOWN REMAINING DIVERGENCE — see file header)"
        else
            test "$SPI_COLOR_MATCH" = 1; and __spi_gate ok "$seg colour parity"; or __spi_gate fail "$seg colour parity"
        end
    end
end

if test (count $argv) -gt 0
    echo
    echo "=== User-supplied directories (report only, no gating) ==="
    for dir in $argv
        set -l abs (path normalize $dir)
        if not test -d $abs
            echo "── (skipped: not a directory) ── ($dir)"
            continue
        end
        for seg in username hostname directory git duration lang character
            __spi_compare_segment "user dir" $seg $abs
        end
    end
end

# ---------------------------------------------------------------------------
# Cleanup + verdict
# ---------------------------------------------------------------------------
rm -rf $SPI_ISO $fixtures_root

echo
if test $SPI_FAILURES -gt 0
    echo "❌ $SPI_FAILURES gating parity check(s) failed"
    exit 1
end
echo "✅ Starship preset-import parity gating checks passed"
exit 0
