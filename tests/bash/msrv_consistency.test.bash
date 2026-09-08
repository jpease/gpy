#!/usr/bin/env bash
# tests/bash/msrv_consistency.test.bash
#
# Regression test for #516. The repository declares its minimum Rust version in
# three places and they had drifted apart, each failure invisible to the person
# who could have noticed it:
#
#   * gpy-agent/Cargo.toml       rust-version = "1.98"   (what cargo enforces)
#   * gpy-agent/.clippy.toml     msrv = "1.90"           (what clippy lints against)
#   * gpy-agent/rust-toolchain.toml  channel = "1.96.0"  (what a clone installs)
#
# The clippy value silently disabled every lint gated on a newer MSRV --
# clippy::duration_suboptimal_units was suppressed at three sites. The toolchain
# pin was worse: it is BELOW the declared rust-version, so `cargo check` in a
# fresh clone failed with "rustc 1.96.0 is not supported ... requires rustc
# 1.98". Neither the maintainer (RUSTUP_TOOLCHAIN set) nor CI (every workflow
# installs dtolnay/rust-toolchain@stable, overriding the file) ever ran it.
#
# Values are read from the files rather than hardcoded here, so bumping the MSRV
# in one place fails until the other two follow.
#
# Asserts:
#   (a) all three declarations are present and parseable
#   (b) .clippy.toml's msrv equals Cargo.toml's rust-version exactly
#   (c) rust-toolchain.toml's channel is >= Cargo.toml's rust-version, so a
#       fresh clone can build what the manifest requires
#   (d) no comment claims these match while they do not

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

MANIFEST="gpy-agent/Cargo.toml"
CLIPPY="gpy-agent/.clippy.toml"
TOOLCHAIN="gpy-agent/rust-toolchain.toml"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Normalize "1.98" and "1.98.0" to a comparable integer triple.
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
        fail "$CLIPPY msrv is $clippy_msrv but $MANIFEST declares rust-version $rust_version; every lint gated above $clippy_msrv is silently disabled (#516)"
    fi
fi

if [[ -n "$rust_version" && -n "$channel" ]]; then
    echo "--- a fresh clone installs a toolchain that can build the manifest ---"
    # A named channel (stable/beta/nightly) cannot be compared numerically and
    # is a different decision; only a pinned x.y.z is checked here.
    if [[ "$channel" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
        if [[ "$(version_key "$channel")" -lt "$(version_key "$rust_version")" ]]; then
            fail "$TOOLCHAIN pins $channel but $MANIFEST requires rust-version $rust_version; a fresh clone fails with 'rustc $channel is not supported' (#516)"
        fi
    fi
fi

echo "--- no comment claims a match that does not hold ---"
# The stale value survived because .clippy.toml asserted in a comment that it
# matched Cargo.toml. A comment making that claim is only allowed to stand when
# the values above actually agree, which (b) has already established.
if grep -qiE '#.*match.*rust-version|#.*matches.*Cargo\.toml' "$CLIPPY"; then
    if [[ "$(version_key "$clippy_msrv")" != "$(version_key "$rust_version")" ]]; then
        fail "$CLIPPY claims in a comment that its msrv matches Cargo.toml, and it does not"
    fi
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
