#!/usr/bin/env bash
# tests/repo/msrv_consistency.test.bash
# Cargo.toml rust-version, .clippy.toml msrv and rust-toolchain.toml channel
# must agree (CI overrides the toolchain file, hiding drift).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
CLIPPY="gpy-agent/.clippy.toml"
TOOLCHAIN="gpy-agent/rust-toolchain.toml"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

require_yq

version_key() {
    local major minor patch
    IFS=. read -r major minor patch <<<"$1"
    printf '%d%03d%03d' "${major:-0}" "${minor:-0}" "${patch:-0}"
}

echo "--- all three MSRV declarations are present ---"
rust_version="$(yq -p toml -o yaml '.package."rust-version" // ""' "$MANIFEST")"
clippy_msrv="$(yq -p toml -o yaml '.msrv // ""' "$CLIPPY")"
channel="$(yq -p toml -o yaml '.toolchain.channel // ""' "$TOOLCHAIN")"

[[ -n "$rust_version" ]] || fail "$MANIFEST declares no rust-version"
[[ -n "$clippy_msrv" ]] || fail "$CLIPPY declares no msrv"
[[ -n "$channel" ]] || fail "$TOOLCHAIN declares no channel"
echo "  Cargo.toml rust-version : ${rust_version:-<none>}"
echo "  .clippy.toml msrv       : ${clippy_msrv:-<none>}"
echo "  rust-toolchain channel  : ${channel:-<none>}"

if [[ -n "$rust_version" && -n "$clippy_msrv" ]]; then
    echo "--- clippy lints against the declared MSRV ---"
    if [[ "$(version_key "$clippy_msrv")" != "$(version_key "$rust_version")" ]]; then
        fail "$CLIPPY msrv is $clippy_msrv but $MANIFEST declares rust-version $rust_version; every lint gated above $clippy_msrv is silently disabled"
    fi
fi

if [[ -n "$rust_version" && -n "$channel" ]]; then
    echo "--- a fresh clone installs a toolchain that can build the manifest ---"
# A named channel (stable/beta/nightly) is not compared; only a pinned x.y.z.
    if [[ "$channel" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
        if [[ "$(version_key "$channel")" -lt "$(version_key "$rust_version")" ]]; then
            fail "$TOOLCHAIN pins $channel but $MANIFEST requires rust-version $rust_version; a fresh clone fails with 'rustc $channel is not supported'"
        fi
    fi
fi

test_finish
