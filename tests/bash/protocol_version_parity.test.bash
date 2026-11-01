#!/usr/bin/env bash
# tests/bash/protocol_version_parity.test.bash
# The IPC protocol version is hand-copied into the agent and the three shell
# integrations; every copy must declare the same value as the agent's
# PROTOCOL_VERSION.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

RUST="gpy-agent/src/ipc/protocol/mod.rs"

rust_version="$(sed -nE 's/^pub const PROTOCOL_VERSION: u8 = ([0-9]+);.*/\1/p' "$RUST" 2>/dev/null)"
if [[ -z "$rust_version" ]]; then
    fail "could not read the protocol version from $RUST"
    test_finish
fi

check_copy() {
    local file="$1" expr="$2" value
    value="$(sed -nE "$expr" "$file" 2>/dev/null)"
    if [[ -z "$value" ]]; then
        fail "could not read the protocol version from $file"
    elif [[ "$value" != "$rust_version" ]]; then
        fail "$file declares $value but $RUST declares $rust_version"
    fi
}

check_copy fish/core/ipc.fish 's/^[[:space:]]*set -l __gpy_protocol_version ([0-9]+).*/\1/p'
check_copy bash/core/ipc.bash 's/^__gpy_protocol_version=([0-9]+).*/\1/p'
check_copy zsh/core/ipc.zsh 's/^typeset -g __gpy_protocol_version=([0-9]+).*/\1/p'

test_finish "every shell declares protocol version $rust_version"
