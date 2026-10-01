#!/usr/bin/env fish
# tests/fish/starship_parity.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test: Native Starship vs GPY (starship theme) prompt parity
# ---------------------------------------------------------------------------
# Verifies that GPY's `starship` theme (applied with `gpy theme use starship
# --force`) renders the same prompt as native `starship` for a set of
# directories.
#
# HOW IT WORKS
#   * GPY side runs in a fully ISOLATED config dir (temp $XDG_CONFIG_HOME). The
#     real user config is never touched. `gpy theme use starship --force` is
#     invoked there exactly as a user would, then each segment is rendered with
#     `gpy-agent oneshot <segment> --format ansi` and concatenated in the
#     configured order to reconstruct the prompt's module line.
#   * Starship side runs with the parity baseline config
#     (tests/fish/fixtures/starship_match.toml) via `starship prompt`.
#   * Parity is checked two ways: (1) visible text (ANSI stripped, trimmed) for
#     layout, and (2) a per-character COLOUR signature (the active SGR code on
#     each non-space glyph) for colour. A raw byte compare is meaningless — the
#     two renderers group/repeat SGR codes differently and colour whitespace
#     differently — but the colour signature catches real colour drift (e.g.
#     swift red vs 256-colour 202) that stripping alone would hide.
#
# GATING (must hold; a regression fails the suite)
#   Full module-line parity AND character parity (text + colour) for every
#   built-in fixture (plain dir, clean git repo, dirty git repo). GPY's starship
#   theme reproduces Starship's hostname, directory, git (branch spacing +
#   `[!?]`-style status with no counts), language (symbol shown even without a
#   version; fish excluded), command-duration, and character output
#   identically — same visible text and same per-character colour.
#
#   Hostname is a special case: both Starship and GPY hide it off-SSH by
#   default (`ssh_only`/`show_always`), so the fixture sets `ssh_only = false`
#   and the isolated GPY config is patched to `show_always = true` with no
#   icon for this comparison only (see `__sp_patch_hostname_visible`) — this
#   exercises `<hostname> in ` text+colour parity, but NOT the `ssh_symbol`/
#   icon glyph itself, since real Starship suppresses `ssh_symbol` off-SSH no
#   matter what `ssh_only` is set to. The icon is gated by the shell's
#   `segment_hostname_detect` over real SSH instead (#259), which this
#   non-SSH harness cannot exercise.
#
# DELIBERATE EXCLUSION
#   * package module: Starship shows `is 📦 v1.2.3`; GPY does not emulate it, so
#     the baseline config disables it (see the fixture header).
#
# KNOWN REMAINING DIVERGENCE (only on repos that exercise it; not in fixtures)
#   * git stash: Starship's git_status shows `$` for stashed changes; GPY does
#     not track stashes, so a stashed repo differs (e.g. `[$!?]` vs `[!?]`).
#     Surfaced by the report on user-supplied dirs; not gated.
#
#   Pass extra directories as arguments for ad-hoc test beds (report-only):
#       fish tests/fish/starship_parity.test.fish ~/some/repo ~/another/dir
#
# Skips cleanly (exit 0) when `starship` is not installed (e.g. CI).

# Resolve to an ABSOLUTE repo root: helpers below cd into fixture dirs before
# using $match_cfg / the binaries, so relative paths (status dirname is relative
# when the suite sources this file) would break after the cd.
set -l test_root (status dirname)/../..
set -g repo_root (path resolve $test_root)

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
# Shared skip contract (#650): exits 0 locally, fails under CI.
source (dirname (status filename))/../lib/test_helpers.fish
if not command -q starship
    test_skip "starship not installed — parity comparison unavailable"
end

# Globals (not `set -l`): helper functions below must see these.
set -g gpy_bin $repo_root/gpy-agent/target/debug/gpy
set -g agent_bin $repo_root/gpy-agent/target/debug/gpy-agent
for bin in $gpy_bin $agent_bin
    if not test -x $bin
        echo "❌ binary not found at $bin"
        echo "   Build it first: (cd gpy-agent && RUSTC_WRAPPER= cargo build)"
        exit 1
    end
end

set -g match_cfg $repo_root/tests/fish/fixtures/starship_match.toml
if not test -f $match_cfg
    echo "❌ baseline starship config missing: $match_cfg"
    exit 1
end

# Fixed inputs so both renderers see identical dynamic context.
set -g SP_STATUS 0
set -g SP_DURATION 5000 # > Starship's 2s threshold so the duration module shows

# ---------------------------------------------------------------------------
# Isolated GPY config: apply the starship theme without touching real config
# ---------------------------------------------------------------------------
# Override the isolated copy's [segments.hostname] block so the segment is
# visible for this comparison, without the icon.
#
# The shipped preset is SSH-only (show_always = false) with an icon set to
# Starship's ssh_symbol default — correct for real use (SSH shows icon+host,
# a local session shows nothing), since #259 means the agent cannot SSH-gate
# the icon itself; that gating is the shell's segment_hostname_detect. This
# harness never runs over SSH, so it forces the segment on for comparison —
# and drops the icon, because real Starship suppresses `ssh_symbol` off-SSH
# too (verified: `ssh_only = false` alone does not surface it — see
# fixtures/starship_match.toml). Only the isolated test copy is patched; the
# shipped starship.toml (and real users' SSH-only default) is untouched.
function __sp_patch_hostname_visible --argument-names theme_path
    set -l in_hostname 0
    set -l out_lines
    for ln in (cat $theme_path)
        if string match -qr '^\[' -- $ln
            set in_hostname 0
            test "$ln" = '[segments.hostname]'; and set in_hostname 1
        end
        if test "$in_hostname" = 1
            switch $ln
                case 'show_always = false'
                    set -a out_lines 'show_always = true'
                    continue
                case 'icon = *'
                    continue
                case '*'
                    set -a out_lines $ln
            end
        else
            set -a out_lines $ln
        end
    end
    printf '%s\n' $out_lines >$theme_path
end

set -g SP_ISO (mktemp -d)
mkdir -p $SP_ISO/gpy/themes
cp $repo_root/config/themes/starship.toml $SP_ISO/gpy/themes/starship.toml
__sp_patch_hostname_visible $SP_ISO/gpy/themes/starship.toml

set -gx XDG_CONFIG_HOME $SP_ISO
set -gx HOME $SP_ISO
set -gx MISE_DISABLE 1

if not $gpy_bin theme use starship --force >/dev/null 2>&1
    echo "❌ 'gpy theme use starship --force' failed in isolated config"
    rm -rf $SP_ISO
    exit 1
end

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Strip ANSI SGR escape sequences (\e[...m) from stdin/args.
function __sp_strip
    string replace -ra '\x1b\[[0-9;]*m' '' -- $argv
end

# Trim surrounding whitespace (renderers differ on leading/trailing padding).
function __sp_trim
    string trim -- $argv
end

# Colour signature: the active SGR code for each NON-space visible character, in
# render order (one `code|char` per line). This makes the comparison colour-aware
# — catching drift like swift red vs 256-colour 202 that plain ANSI-stripping
# hides — while staying tolerant of the two harmless ways the renderers differ:
#   * which side of a reset a space sits on (a space has no visible colour), and
#   * GPY re-emitting the same SGR code before each token vs Starship emitting it
#     once (same active colour either way).
# Assumes each colour span is set by one combined SGR (e.g. `1;38;5;202`) and
# cleared by `0`/empty — true for both Starship and GPY output.
function __sp_color_sig --argument-names raw
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
function __sp_seg --argument-names seg dir
    set -l out
    switch $seg
        case directory git lang
            set out ($agent_bin oneshot $seg --cwd $dir --format ansi --not-last 2>/dev/null | string collect)
        case duration
            set out ($agent_bin oneshot duration --duration-ms $SP_DURATION --format ansi --not-last 2>/dev/null | string collect)
        case hostname
            # Fish's builtin $hostname; native starship reads the same machine
            # hostname, so both sides render identical text.
            set out ($agent_bin oneshot hostname --hostname $hostname --format ansi --not-last 2>/dev/null | string collect)
    end
    test -n "$out"; or return 0
    # Skip JSON error responses (e.g. "Not in a git repository").
    string match -qr '^\s*\{' -- $out; and return 0
    printf '%s' $out
end

# Build GPY's module line (segments joined in configured order) for a dir.
# Accumulate into a list and join: concatenating an empty command substitution
# directly (`set x $x(sub)`) would collapse the whole value to empty in fish.
function __sp_gpy_modules --argument-names dir
    set -l parts
    for seg in hostname directory git lang duration
        set -l part (__sp_seg $seg $dir)
        test -n "$part"; and set -a parts $part
    end
    string join '' -- $parts
end

# GPY's character segment (line 2 of the prompt).
function __sp_gpy_char
    $agent_bin oneshot character --exit-code $SP_STATUS --format ansi 2>/dev/null | string collect
end

# Native starship prompt for a dir using the baseline config. Echoes the raw
# multi-line output. Starship reads the *process* CWD for git/language/package
# detection (its --path flag only affects path display), so render from inside
# the directory to match what GPY's --cwd sees. Use a child `sh` rather than
# fish `pushd` so changing directory does not fire fish PWD-change hooks (e.g.
# mise) that would spam the report with unrelated errors.
function __sp_starship --argument-names dir
    sh -c 'cd "$1" || exit 1; term="${TERM:-xterm-256color}"; [ "$term" != "dumb" ] || term="xterm-256color"; TERM="$term" STARSHIP_CONFIG="$2" starship prompt --status "$3" --cmd-duration "$4" --terminal-width 80' \
        sh $dir $match_cfg $SP_STATUS $SP_DURATION 2>/dev/null
end

# Return non-blank lines of a (possibly multi-line) string as a list.
function __sp_nonblank
    for ln in (string split \n -- $argv)
        test -n (string trim -- $ln); and echo $ln
    end
end

# ---------------------------------------------------------------------------
# Comparison + report for a single directory.
# Prints the report and sets globals SP_MOD_MATCH / SP_CHR_MATCH (1 = identical
# visible text AND identical per-character colour).
# ---------------------------------------------------------------------------
function __sp_compare --argument-names label dir
    # GPY: module line + character line.
    set -l gpy_mod (__sp_gpy_modules $dir)
    set -l gpy_chr (__sp_gpy_char)
    set -l gpy_mod_v (__sp_trim (__sp_strip $gpy_mod))
    set -l gpy_chr_v (__sp_trim (__sp_strip $gpy_chr))

    # Starship emits a two-line prompt (modules, then character). Collect with
    # newlines preserved, then take first non-blank line = modules, last = char.
    set -l sp_raw (__sp_starship $dir | string collect)
    set -l sp_lines (__sp_nonblank $sp_raw)
    set -l sp_mod ""
    set -l sp_chr ""
    if test (count $sp_lines) -ge 1
        set sp_mod $sp_lines[1]
        set sp_chr $sp_lines[-1]
    end
    set -l sp_mod_v (__sp_trim (__sp_strip $sp_mod))
    set -l sp_chr_v (__sp_trim (__sp_strip $sp_chr))

    # Per-character colour signatures (see __sp_color_sig).
    set -l gpy_mod_sig (__sp_color_sig $gpy_mod | string collect)
    set -l sp_mod_sig (__sp_color_sig $sp_mod | string collect)
    set -l gpy_chr_sig (__sp_color_sig $gpy_chr | string collect)
    set -l sp_chr_sig (__sp_color_sig $sp_chr | string collect)

    # Parity requires both the visible text (layout) AND the per-char colour.
    set -g SP_MOD_MATCH 0
    set -g SP_CHR_MATCH 0
    if test "$gpy_mod_v" = "$sp_mod_v"; and test "$gpy_mod_sig" = "$sp_mod_sig"
        set -g SP_MOD_MATCH 1
    end
    if test "$gpy_chr_v" = "$sp_chr_v"; and test "$gpy_chr_sig" = "$sp_chr_sig"
        set -g SP_CHR_MATCH 1
    end

    echo "── $label ── ($dir)"
    if test "$SP_MOD_MATCH" = 1; and test "$SP_CHR_MATCH" = 1
        echo "  ✅ identical (text + colour)"
        echo "     prompt : $sp_mod_v ⏎ $sp_chr_v"
        return 0
    end
    echo "  ⚠️  differ"
    if test "$gpy_mod_v" != "$sp_mod_v"
        echo "     text     starship : '$sp_mod_v'"
        echo "     text     gpy      : '$gpy_mod_v'"
    else if test "$gpy_mod_sig" != "$sp_mod_sig"
        echo "     text matches; COLOUR differs (see escapes below)"
    end
    if test "$gpy_chr_v" != "$sp_chr_v"; or test "$gpy_chr_sig" != "$sp_chr_sig"
        echo "     char     starship : '$sp_chr_v'"
        echo "     char     gpy      : '$gpy_chr_v'"
    end
    # Escape-level view to expose invisible/byte differences.
    echo "     escapes  starship : "(printf '%s' $sp_mod | cat -v)
    echo "     escapes  gpy      : "(printf '%s' $gpy_mod | cat -v)
end

# Record a gating invariant outcome. $1 = "ok"|"fail", $2 = description.
function __sp_gate --argument-names ok desc
    if test "$ok" = ok
        echo "     ✔ gate: $desc"
    else
        echo "     ✘ GATE FAILED: $desc"
        set -g SP_FAILURES (math $SP_FAILURES + 1)
    end
end

# ---------------------------------------------------------------------------
# Build deterministic fixtures.
# ---------------------------------------------------------------------------
function __sp_mkrepo --argument-names dir
    mkdir -p $dir
    git -C $dir init -q
    git -C $dir config user.email parity@test.local
    git -C $dir config user.name parity
    git -C $dir symbolic-ref HEAD refs/heads/main
end

set -l fixtures_root (mktemp -d)

# Clean git repo (Rust marker) — should achieve full parity.
set -l fx_clean $fixtures_root/clean-rust
__sp_mkrepo $fx_clean
printf '[package]\nname = "demo"\nversion = "0.1.0"\n' >$fx_clean/Cargo.toml
git -C $fx_clean add -A
git -C $fx_clean commit -qm init

# Plain non-git directory — should achieve full parity (directory only).
set -l fx_plain $fixtures_root/plain
mkdir -p $fx_plain

# Dirty git repo — exercises git status (`[!?]` symbols, no counts).
set -l fx_dirty $fixtures_root/dirty
__sp_mkrepo $fx_dirty
echo original >$fx_dirty/a.txt
git -C $fx_dirty add -A
git -C $fx_dirty commit -qm init
echo modified >>$fx_dirty/a.txt
echo new >$fx_dirty/untracked.txt

# ---------------------------------------------------------------------------
# Run.
#
# Gating invariants (must hold; a regression fails the suite): full module-line
# parity AND character parity for every built-in fixture. The starship theme's
# [segments.git]/[segments.language] settings make GPY render Starship-exact
# output for directory, git (branch spacing + `[!?]`-style status), language
# (symbol shown without a version; fish excluded), command-duration, and
# character. The only deliberate exclusion is Starship's `package` module, which
# GPY does not emulate and which the baseline config disables.
# ---------------------------------------------------------------------------
set -g SP_FAILURES 0

echo "=== Gating fixtures ==="
for pair in "plain non-git dir|$fx_plain" "clean git repo|$fx_clean" "dirty git repo|$fx_dirty"
    set -l label (string split -m1 '|' -- $pair)[1]
    set -l dir (string split -m1 '|' -- $pair)[2]
    __sp_compare $label $dir
    test "$SP_MOD_MATCH" = 1; and __sp_gate ok "module parity"; or __sp_gate fail "module parity"
    test "$SP_CHR_MATCH" = 1; and __sp_gate ok "character parity"; or __sp_gate fail "character parity"
end

if test (count $argv) -gt 0
    echo
    echo "=== User-supplied directories (report only) ==="
    for dir in $argv
        set -l abs (path normalize $dir)
        if not test -d $abs
            echo "── (skipped: not a directory) ── ($dir)"
            continue
        end
        __sp_compare "user dir" $abs
    end
end

# ---------------------------------------------------------------------------
# Cleanup + verdict
# ---------------------------------------------------------------------------
rm -rf $SP_ISO $fixtures_root

echo
if test $SP_FAILURES -gt 0
    echo "❌ $SP_FAILURES gating parity check(s) failed"
    exit 1
end
echo "✅ Starship parity gating checks passed"
exit 0
