#!/usr/bin/env bash
# Assert that the active rustc is the one gpy-agent/rust-toolchain.toml pins.
#
# actions-rust-lang/setup-rust-toolchain silently falls back to `stable` when
# it cannot find the file, so CI asserts the resolution instead of assuming it.
#
# Probes from gpy-agent/: rustup searches the current directory and its
# parents, never children, so at the repo root `rustc` is the machine default.
# A `rustup override set stable` fallback or RUSTUP_TOOLCHAIN still applies
# there and is still caught.
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
    # RUSTUP_TOOLCHAIN outranks directory overrides and the file; usual culprit.
    if [[ -n "${RUSTUP_TOOLCHAIN:-}" ]]; then
        reason="RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN overrides rust-toolchain.toml (unset it, or set it to $pinned)"
    elif command -v rustup >/dev/null 2>&1 && reason="$(rustup show active-toolchain 2>/dev/null | head -n1)" && [[ -n "$reason" ]]; then
        reason="rustup reports: $reason"
    else
        reason="no RUSTUP_TOOLCHAIN set; check for a rustup directory override (rustup override list)"
    fi
    # Renders as a GitHub annotation; inert elsewhere.
    echo "::error::rustc $active is active but rust-toolchain.toml pins $pinned"
    die "toolchain mismatch: expected $pinned, got $active -- $reason"
fi

echo "  toolchain matches the pin"
