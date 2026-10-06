#!/usr/bin/env bash
# Assert that the active rustc is the one gpy-agent/rust-toolchain.toml pins.
#
# CI resolves its toolchain from that file rather than installing @stable
# (#518). The mechanism is a silent one: actions-rust-lang/setup-rust-toolchain
# installs from the file only when it finds it in the working directory, and
# falls back to `stable` when it does not -- no warning, no failed step. A
# renamed file, a moved crate, or a dropped `rust-src-dir:` input would restore
# the exact bug #518 fixed while every job stayed green.
#
# So the resolution is asserted rather than assumed. This is also what makes a
# deliberate bump of rust-toolchain.toml observable: bump the pin, and every
# job that has not picked it up fails by name.
#
# WHY THIS PROBES FROM gpy-agent/ AND NOT THE REPOSITORY ROOT (#538)
#
# A toolchain file governs its own directory and the directories below it.
# rustup searches the current directory and its PARENTS for one -- never its
# children. gpy-agent/rust-toolchain.toml therefore says nothing about the
# repository root, where `rustc` falls through to rustup's default toolchain,
# whatever the machine happens to have set.
#
# This script used to probe from the root and so reported that default. It was
# green on ubuntu-latest and windows-latest only because those images ship
# 1.98.0 as their default, and red on macos-latest, which ships 1.97.1 -- while
# all three were in fact building with the pinned 1.98.0. A coincidence of
# runner images is not an assertion; the next image bump would have turned the
# passing runners red the same way, and any contributor whose rustup default is
# not the pin (a nightly default is common) saw the same false alarm locally.
#
# Every Rust command in this repository runs from gpy-agent/ (see the justfile
# and scripts/quality-check.sh), so that is the directory whose active toolchain
# actually decides what gets compiled, and the one worth asserting.
#
# The guard is not weakened by probing there. If the action ever loses the file
# and falls back, it installs `stable` and runs `rustup override set stable`;
# a directory override outranks a toolchain file and applies to subdirectories,
# so the fallback still fails this check. RUSTUP_TOOLCHAIN, which outranks both,
# is likewise still caught.
#
# Run from anywhere; paths resolve against the repository root.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN="$ROOT/gpy-agent/rust-toolchain.toml"

die() {
    echo "check-active-toolchain: $*" >&2
    exit 1
}

[[ -f "$TOOLCHAIN" ]] || die "not found: $TOOLCHAIN"

pinned="$(grep -m1 '^channel' "$TOOLCHAIN" | sed -E 's/.*"([^"]+)".*/\1/')"
[[ -n "$pinned" ]] || die "$TOOLCHAIN declares no channel"

command -v rustc >/dev/null 2>&1 || die "rustc not found"

# Probe from the directory the pin governs, not from wherever this was invoked.
crate_dir="$(dirname "$TOOLCHAIN")"
cd "$crate_dir" || die "cannot enter $crate_dir"
active="$(rustc --version | cut -d' ' -f2)"

echo "  probed from                 : ${PWD#"$ROOT"/}"
echo "  pinned (rust-toolchain.toml): $pinned"
echo "  active (rustc --version)    : $active"

if [[ "$pinned" != "$active" ]]; then
    # Say WHY the pin lost. RUSTUP_TOOLCHAIN outranks both directory overrides
    # and rust-toolchain.toml, so it is the usual culprit (a shell profile or
    # an agent session exporting a newer version); otherwise ask rustup.
    if [[ -n "${RUSTUP_TOOLCHAIN:-}" ]]; then
        reason="RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN overrides rust-toolchain.toml (unset it, or set it to $pinned)"
    elif command -v rustup >/dev/null 2>&1 && reason="$(rustup show active-toolchain 2>/dev/null | head -n1)" && [[ -n "$reason" ]]; then
        reason="rustup reports: $reason"
    else
        reason="no RUSTUP_TOOLCHAIN set; check for a rustup directory override (rustup override list)"
    fi
    # ::error:: renders as an annotation on the job in GitHub's UI and is inert
    # elsewhere, so the same script is useful locally.
    echo "::error::rustc $active is active but rust-toolchain.toml pins $pinned"
    die "toolchain mismatch: expected $pinned, got $active -- $reason"
fi

echo "  toolchain matches the pin"
