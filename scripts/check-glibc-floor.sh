#!/usr/bin/env bash
# scripts/check-glibc-floor.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fail unless every GLIBC_x.y symbol version a Linux binary requires is at or
# below a floor (#694). The release workflow runs it on every Linux binary
# after the build, with the floor from GPY_GLIBC_FLOOR in
# .github/workflows/release.yml, so a binary that would not start on the
# documented oldest distro (docs/INSTALL.md) never ships. Runs on any Linux
# build:
#
#   scripts/check-glibc-floor.sh gpy-agent/target/<triple>/release-dist/gpy-agent 2.31
#
# The versions come from `objdump -T` (the dynamic symbol table). Ubuntu's
# stock objdump also reads the other architecture's ELF (x86_64 <-> aarch64);
# set OBJDUMP to use a different one, e.g. OBJDUMP=aarch64-linux-gnu-objdump.

set -euo pipefail

usage() {
    echo "usage: $0 <binary> <floor, e.g. 2.31>" >&2
    exit 2
}

[[ $# -eq 2 ]] || usage
binary="$1"
floor="$2"
objdump="${OBJDUMP:-objdump}"

[[ "$floor" =~ ^[0-9]+\.[0-9]+$ ]] || {
    echo "❌ glibc floor must look like 2.31, got '$floor'" >&2
    exit 2
}
[[ -f "$binary" ]] || {
    echo "❌ no such binary: $binary" >&2
    exit 1
}
command -v "$objdump" >/dev/null 2>&1 || {
    echo "❌ $objdump not found (install binutils)" >&2
    exit 1
}

if ! symbols="$("$objdump" -T "$binary" 2>&1)"; then
    echo "❌ $objdump -T failed on $binary:" >&2
    printf '%s\n' "$symbols" >&2
    exit 1
fi

# GLIBC_PRIVATE and friends carry no number and are skipped by the pattern.
versions="$(grep -o 'GLIBC_[0-9][0-9.]*' <<<"$symbols" | sed 's/^GLIBC_//' | sort -Vu || true)"
if [[ -z "$versions" ]]; then
    # A glibc-linked release binary always references some GLIBC_ version;
    # none means this is not the binary the floor is meant to guard.
    echo "❌ $binary references no GLIBC_ symbol versions; is it a dynamically linked glibc binary?" >&2
    exit 1
fi

max="$(sort -V <<<"$versions" | tail -n 1)"
highest="$(printf '%s\n%s\n' "$max" "$floor" | sort -V | tail -n 1)"
if [[ "$highest" != "$floor" ]]; then
    echo "❌ $binary requires GLIBC_$max, above the floor GLIBC_$floor" >&2
    echo "   Symbols above the floor:" >&2
    while IFS= read -r version; do
        [[ "$(printf '%s\n%s\n' "$version" "$floor" | sort -V | tail -n 1)" == "$floor" ]] && continue
        { grep -E "GLIBC_${version//./\\.}([^0-9.]|$)" <<<"$symbols" || true; } | awk '{print "     " $NF " (GLIBC_'"$version"')"}' >&2
    done <<<"$versions"
    exit 1
fi

echo "✅ $binary: max GLIBC_$max <= floor GLIBC_$floor"
