#!/usr/bin/env bash
# tests/bash/release_glibc_floor.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #694: the Linux release binaries were built with a plain
# `cargo build` on ubuntu-latest (glibc 2.39), so they required GLIBC_2.39 and
# did not start on Ubuntu 20.04/22.04 or Debian 11/12, which docs/INSTALL.md
# claimed. Nothing in the release ran them on an older glibc.
#
# One floor, GPY_GLIBC_FLOOR in .github/workflows/release.yml, now drives
# everything. Asserts:
#   (a) the workflow declares GPY_GLIBC_FLOOR as MAJOR.MINOR
#   (b) every *-unknown-linux-gnu matrix target builds with
#       `cargo zigbuild ... --target <triple>.${GPY_GLIBC_FLOOR}` and no Linux
#       leg falls through to a plain `cargo build`
#   (c) a Linux-only build step runs scripts/check-glibc-floor.sh on both
#       binaries against "$GPY_GLIBC_FLOOR"
#   (d) scripts/check-glibc-floor.sh reads `objdump -T`, rejects a binary that
#       needs a newer glibc (naming the offending symbol), accepts one at the
#       floor, and rejects one with no GLIBC_ versions at all
#   (e) the smoke job runs both packaged x86_64 binaries' --version inside
#       GPY_GLIBC_FLOOR_IMAGE, and that image's glibc is the floor
#   (f) docs/INSTALL.md states the same floor in the platform table and in the
#       Linux notes, and no longer claims a distro below it

# Single-quoted `${{ … }}` and `$var` below are literal workflow/script text.
# shellcheck disable=SC2016

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/release.yml"
CHECKER="$ROOT/scripts/check-glibc-floor.sh"
INSTALL_DOC="$ROOT/docs/INSTALL.md"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}
pass() {
    echo "PASS: $*"
}

for required in "$WORKFLOW" "$INSTALL_DOC"; do
    [[ -f "$required" ]] || {
        echo "FAIL: missing $required"
        exit 1
    }
done

# Print the body of one top-level job (`  <name>:` up to the next job).
job_block() {
    awk -v job="  $1:" '
        $0 == job { in_job = 1; print; next }
        in_job && /^  [A-Za-z0-9_-]+:$/ { exit }
        in_job { print }
    ' "$WORKFLOW"
}

# --- (a) the floor constant -------------------------------------------------
floor="$(sed -n 's/^  GPY_GLIBC_FLOOR: *"\{0,1\}\([0-9][0-9.]*\)"\{0,1\} *$/\1/p' "$WORKFLOW")"
if [[ "$floor" =~ ^[0-9]+\.[0-9]+$ ]]; then
    pass "release.yml declares GPY_GLIBC_FLOOR=$floor"
else
    fail "release.yml has no workflow-level GPY_GLIBC_FLOOR: \"MAJOR.MINOR\" (got '$floor')"
    floor="__missing__"
fi

# --- (b) Linux legs build against the floor ---------------------------------
build_job="$(job_block build)"
linux_targets="$(grep -oE 'target: *[a-z0-9_]+-unknown-linux-gnu *$' <<<"$build_job" | sed 's/target: *//; s/ *$//')"
if [[ -z "$linux_targets" ]]; then
    fail "no *-unknown-linux-gnu target in the build matrix"
fi

zig_line='cargo zigbuild --locked --profile release-dist --target "${{ matrix.target }}.${GPY_GLIBC_FLOOR}"'
if grep -qF -- "$zig_line" <<<"$build_job"; then
    pass "the build step links Linux legs with cargo zigbuild against <triple>.\${GPY_GLIBC_FLOOR}"
else
    fail "build job lacks: $zig_line"
fi

# The zigbuild line must sit in the `*-unknown-linux-gnu)` arm of the case,
# so every Linux target takes it and nothing else does.
build_case="$(awk '/case "\$\{\{ matrix.target \}\}" in/,/esac/' <<<"$build_job")"
linux_arm="$(awk '/\*-unknown-linux-gnu\)/,/;;/' <<<"$build_case")"
if grep -qF -- "$zig_line" <<<"$linux_arm"; then
    pass "the zigbuild command is the *-unknown-linux-gnu arm of the build case"
else
    fail "the zigbuild command is not in a '*-unknown-linux-gnu)' case arm: $build_case"
fi
if grep -q 'cargo build' <<<"$linux_arm"; then
    fail "the Linux arm still runs a plain cargo build"
fi

# --- (c) the release-blocking floor check ------------------------------------
gate_step="$(awk '/- name: Enforce the glibc floor/,/^      - name: Upload agent/' <<<"$build_job")"
if [[ -z "$gate_step" ]]; then
    fail "build job has no 'Enforce the glibc floor' step before the uploads"
else
    if grep -qF "if: endsWith(matrix.target, '-unknown-linux-gnu')" <<<"$gate_step"; then
        pass "the floor check runs on every Linux leg"
    else
        fail "the floor check is not gated on endsWith(matrix.target, '-unknown-linux-gnu')"
    fi
    if grep -qF './scripts/check-glibc-floor.sh "gpy-agent/target/${{ matrix.target }}/release-dist/$bin" "$GPY_GLIBC_FLOOR"' <<<"$gate_step" &&
        grep -qF 'for bin in ${{ matrix.artifact_name }} ${{ matrix.cli_artifact_name }}' <<<"$gate_step"; then
        pass "the floor check covers gpy-agent and gpy against \$GPY_GLIBC_FLOOR"
    else
        fail "the floor check does not run check-glibc-floor.sh on both binaries against \$GPY_GLIBC_FLOOR"
    fi
fi

# --- (d) the checker itself ---------------------------------------------------
if [[ ! -x "$CHECKER" ]]; then
    fail "scripts/check-glibc-floor.sh is missing or not executable"
else
    if grep -q -- '-T "\$binary"' "$CHECKER"; then
        pass "check-glibc-floor.sh reads objdump -T"
    else
        fail "check-glibc-floor.sh does not run objdump -T on the binary"
    fi

    # A stub objdump prints a canned dynamic symbol table, so the decision
    # logic runs on any host without a Linux binary.
    stub="$WORKDIR/objdump"
    cat >"$stub" <<'EOF'
#!/usr/bin/env bash
cat "$STUB_SYMBOLS"
EOF
    chmod +x "$stub"
    touch "$WORKDIR/bin"

    # objdump -T lines as glibc 2.39 (Ubuntu 24.04) leaves them.
    cat >"$WORKDIR/above" <<'EOF'
0000000000000000      DF *UND*  0000000000000000 (GLIBC_2.17) dlsym
0000000000000000      DF *UND*  0000000000000000 (GLIBC_2.34) __libc_start_main
0000000000000000      DF *UND*  0000000000000000 (GLIBC_2.39) pidfd_spawnp
0000000000000000      DO *UND*  0000000000000000 (GLIBC_PRIVATE) _rtld_global
EOF
    # Same table built against the floor (2.10 sorts below 2.9 lexically,
    # above it by version).
    cat >"$WORKDIR/at" <<'EOF'
0000000000000000      DF *UND*  0000000000000000 (GLIBC_2.17) dlsym
0000000000000000      DF *UND*  0000000000000000 (GLIBC_2.9) pipe2
0000000000000000  w   DF *UND*  0000000000000000 (GLIBC_2.31) __cxa_finalize
EOF
    printf '0000000000000000      DF *UND*  0000000000000000  foo\n' >"$WORKDIR/none"

    run_checker() {
        STUB_SYMBOLS="$WORKDIR/$1" OBJDUMP="$stub" "$CHECKER" "$WORKDIR/bin" "$2" >"$WORKDIR/out" 2>&1
    }

    if run_checker above 2.31; then
        fail "check-glibc-floor.sh accepted a binary needing GLIBC_2.39 against floor 2.31"
    elif grep -q 'requires GLIBC_2.39' "$WORKDIR/out" && grep -q 'pidfd_spawnp (GLIBC_2.39)' "$WORKDIR/out" &&
        grep -q '__libc_start_main (GLIBC_2.34)' "$WORKDIR/out" && ! grep -q 'dlsym' "$WORKDIR/out"; then
        pass "check-glibc-floor.sh rejects GLIBC_2.39 and names exactly the symbols above the floor"
    else
        fail "check-glibc-floor.sh rejected GLIBC_2.39 without naming the offending symbols: $(cat "$WORKDIR/out")"
    fi
    if run_checker at 2.31; then
        pass "check-glibc-floor.sh accepts a binary whose max is the floor"
    else
        fail "check-glibc-floor.sh rejected a binary at the floor: $(cat "$WORKDIR/out")"
    fi
    if run_checker at 2.30; then
        fail "check-glibc-floor.sh accepted GLIBC_2.31 against floor 2.30"
    else
        pass "check-glibc-floor.sh compares versions, not strings"
    fi
    if run_checker none 2.31; then
        fail "check-glibc-floor.sh accepted a binary with no GLIBC_ versions"
    else
        pass "check-glibc-floor.sh fails closed on a binary with no GLIBC_ versions"
    fi
fi

# --- (e) the floor smoke run ------------------------------------------------
image="$(sed -n 's/^  GPY_GLIBC_FLOOR_IMAGE: *\([^ ]*\) *$/\1/p' "$WORKFLOW")"
# glibc each known image ships; extend when the floor moves.
case "$image" in
    ubuntu:20.04 | debian:11 | debian:bullseye) image_glibc=2.31 ;;
    ubuntu:22.04) image_glibc=2.35 ;;
    debian:12 | debian:bookworm) image_glibc=2.36 ;;
    *) image_glibc="" ;;
esac
if [[ -z "$image" ]]; then
    fail "release.yml has no workflow-level GPY_GLIBC_FLOOR_IMAGE"
elif [[ "$image_glibc" != "$floor" ]]; then
    fail "GPY_GLIBC_FLOOR_IMAGE=$image ships glibc '${image_glibc:-unknown}', not the floor $floor"
else
    pass "GPY_GLIBC_FLOOR_IMAGE=$image ships glibc $floor"
fi

smoke_job="$(job_block smoke)"
floor_run="$(awk '/docker run --rm .*"\$GPY_GLIBC_FLOOR_IMAGE"/,/^ *'"'"' *$/' <<<"$smoke_job")"
if grep -q '/b/gpy-agent-linux-x86_64 --version' <<<"$floor_run" &&
    grep -q '/b/gpy-linux-x86_64 --version' <<<"$floor_run" &&
    grep -q 'sh -ec' <<<"$floor_run"; then
    pass "the smoke job runs both x86_64 --versions inside \$GPY_GLIBC_FLOOR_IMAGE and fails on non-zero"
else
    fail "the smoke job does not run gpy-agent and gpy --version in \$GPY_GLIBC_FLOOR_IMAGE under sh -e"
fi

# --- (f) the docs state the same floor ----------------------------------------
linux_row="$(grep '^| Linux (x86_64, aarch64) |' "$INSTALL_DOC")"
if grep -qF "glibc $floor or newer" <<<"$linux_row"; then
    pass "the INSTALL.md platform table states glibc $floor"
else
    fail "the INSTALL.md Linux platform row does not say 'glibc $floor or newer'"
fi
linux_notes="$(awk '/^### Linux$/{on=1; next} on && /^### /{exit} on' "$INSTALL_DOC")"
if grep -qF "glibc $floor or newer" <<<"$linux_notes"; then
    pass "the INSTALL.md Linux notes state glibc $floor"
else
    fail "the INSTALL.md '### Linux' notes do not say 'glibc $floor or newer'"
fi
if grep -q 'Tested on Ubuntu 20.04+' <<<"$linux_notes"; then
    fail "the INSTALL.md Linux notes still claim untested distros as tested"
fi

if [[ $failures -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the Linux release legs are pinned to and gated on glibc $floor"
