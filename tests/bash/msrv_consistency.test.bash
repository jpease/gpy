#!/usr/bin/env bash
# tests/bash/msrv_consistency.test.bash
# Cargo.toml rust-version, .clippy.toml msrv and rust-toolchain.toml channel
# must agree (CI overrides the toolchain file, hiding drift).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
CLIPPY="gpy-agent/.clippy.toml"
TOOLCHAIN="gpy-agent/rust-toolchain.toml"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

version_key() {
    awk -F. '{ printf "%d%03d%03d", $1, ($2 == "" ? 0 : $2), ($3 == "" ? 0 : $3) }' <<<"$1"
}

echo "--- all three MSRV declarations are present ---"
rust_version="$(grep -m1 '^rust-version' "$MANIFEST" | sed -E 's/.*"([^"]+)".*/\1/')"
clippy_msrv="$(grep -m1 '^msrv' "$CLIPPY" | sed -E 's/.*"([^"]+)".*/\1/')"
channel="$(grep -m1 '^channel' "$TOOLCHAIN" | sed -E 's/.*"([^"]+)".*/\1/')"

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

echo "--- no comment claims a match that does not hold ---"
if grep -qiE '#.*match.*rust-version|#.*matches.*Cargo\.toml' "$CLIPPY"; then
    if [[ "$(version_key "$clippy_msrv")" != "$(version_key "$rust_version")" ]]; then
        fail "$CLIPPY claims in a comment that its msrv matches Cargo.toml, and it does not"
    fi
fi

test_finish
