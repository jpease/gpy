# tests/docker/scenario-lib.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Shared helpers for tests/docker/scenarios/*.sh, sourced inside the
# fresh-install image (as /opt/scenario-lib.sh). Every scenario runs in its own
# container, so $HOME starts empty and nothing leaks between scenarios.
#
# Exit codes, read by scripts/test-fresh-install.sh:
#   0  every assertion passed
#   1  an assertion failed (or a precondition the test relies on did not hold)
#   3  a published release asset could not be downloaded (network or GitHub),
#      so nothing was tested
# shellcheck shell=bash

EXIT_ASSERTION=1
EXIT_UNAVAILABLE=3

SCENARIO="${SCENARIO:-$(basename "${BASH_SOURCE[1]:-$0}" .sh)}"
failures=0
REPO_SLUG="jpease/gpy"
PAYLOAD=/opt/payload
# The release under test without its `v`, for --version and status checks.
# shellcheck disable=SC2034
BARE="${GPY_RELEASE_VERSION#v}"
LOCAL_BIN="$HOME/.local/bin"
export PATH="$LOCAL_BIN:$PATH"
export TERM="${TERM:-xterm-256color}"
# No glyph probing: a container has no terminal to ask.
export GPY_NERD_FONT=none
WORK="$(mktemp -d "${TMPDIR:-/tmp}/gpy-scenario.XXXXXX")"
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE 2>/dev/null || true

pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
step() { echo "--- $* ---"; }

# `abort MESSAGE` -- a broken precondition: later assertions would mean nothing.
abort() {
    echo "FAIL: $*"
    echo "SCENARIO $SCENARIO: assertion failed"
    exit "$EXIT_ASSERTION"
}

# `unavailable MESSAGE` -- the network or GitHub failed us; nothing was tested.
unavailable() {
    echo "UNAVAILABLE: $*"
    echo "SCENARIO $SCENARIO: network/asset unavailable"
    exit "$EXIT_UNAVAILABLE"
}

# `finish` -- the scenario's verdict from the failures counted so far.
finish() {
    if (( failures > 0 )); then
        echo "SCENARIO $SCENARIO: $failures assertion(s) failed"
        exit "$EXIT_ASSERTION"
    fi
    echo "SCENARIO $SCENARIO: all assertions passed"
    exit 0
}

# `poll SECS CMD...` -- every 100 ms until CMD exits 0 or SECS pass.
poll() {
    local attempts=$(( $1 * 10 ))
    shift
    while (( attempts > 0 )); do
        "$@" && return 0
        sleep 0.1
        attempts=$((attempts - 1))
    done
    return 1
}

# `show_log FILE` -- the first lines of a log, indented, for a failure.
show_log() { sed -n '1,80p' "$1" | sed 's/^/    | /'; }

case "$(uname -m)" in
    x86_64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) abort "unsupported architecture $(uname -m)" ;;
esac

# `fetch_release TAG ASSET DEST` -- download a published asset and its
# .sha256 sidecar over HTTPS only and verify it. A download failure is
# "unavailable"; a digest mismatch fails closed as an assertion.
fetch_release() {
    local tag="$1" asset="$2" dest="$3" url expected actual
    url="https://github.com/$REPO_SLUG/releases/download/$tag/$asset"
    if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --retry-delay 2 -o "$dest" "$url"; then
        unavailable "could not download $url"
    fi
    if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 3 --retry-delay 2 -o "$dest.sha256" "$url.sha256"; then
        unavailable "could not download $url.sha256"
    fi
    expected="$(awk '{print $1; exit}' "$dest.sha256")"
    actual="$(sha256sum "$dest" | awk '{print $1}')"
    [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || abort "$asset.sha256 from $tag holds no digest"
    [[ "$expected" == "$actual" ]] ||
        abort "checksum mismatch for $tag/$asset (published $expected, downloaded $actual); refusing to run it"
}

# `install_release TAG SHELL` -- install a published release with its own
# install-oneline.sh, as a user of that release did. The installer fetches and
# verifies its binaries itself; a download failure inside it is "unavailable".
install_release() {
    local tag="$1" shell="$2" installer="$WORK/install-oneline-$1.sh" log="$WORK/install-$1.log"
    fetch_release "$tag" install-oneline.sh "$installer"
    grep -qxF "RELEASE_VERSION=\"$tag\"" "$installer" ||
        abort "the $tag install-oneline.sh is not stamped with $tag"
    if ! GPY_SHELL="$shell" sh "$installer" </dev/null >"$log" 2>&1; then
        if grep -q 'Failed to download\|Could not download\|Could not resolve' "$log"; then
            show_log "$log"
            unavailable "the $tag installer could not download its assets"
        fi
        show_log "$log"
        abort "the $tag install-oneline.sh failed"
    fi
    pass "installed $tag with its own install-oneline.sh (GPY_SHELL=$shell)"
}

# `stage_upgrade_assets` -- lay the freshly built release out under $WORK/assets
# as GitHub would serve it: each binary, gpy-shell-files.tar.gz, and a .sha256
# beside each.
stage_upgrade_assets() {
    ASSETS="$WORK/assets"
    mkdir -p "$ASSETS" "$WORK/extract"
    tar -xzf "$PAYLOAD/gpy-release.tar.gz" -C "$WORK/extract"
    cp "$WORK/extract/gpy-release/bin/gpy-agent-linux-$ARCH" "$WORK/extract/gpy-release/bin/gpy-linux-$ARCH" "$ASSETS/"
    cp "$PAYLOAD/gpy-shell-files.tar.gz" "$PAYLOAD/install-oneline.sh" "$ASSETS/"
    (cd "$ASSETS" && for f in *; do sha256sum "$f" >"$f.sha256"; done)
    # The `curl` the new install-oneline.sh finds first: it serves the
    # release-download URLs of this version from $ASSETS and nothing else.
    mkdir -p "$WORK/fake-github"
    cat >"$WORK/fake-github/curl" <<EOF
#!/usr/bin/env bash
url="" out=""
while [[ \$# -gt 0 ]]; do
    case "\$1" in
        -o) out="\$2"; shift 2 ;;
        --proto) shift 2 ;;
        https://*) url="\$1"; shift ;;
        *) shift ;;
    esac
done
prefix="https://github.com/$REPO_SLUG/releases/download/$GPY_RELEASE_VERSION/"
[[ "\$url" == "\$prefix"* ]] || { echo "fake curl: unexpected URL \$url" >&2; exit 22; }
file="$ASSETS/\${url#"\$prefix"}"
[[ -f "\$file" ]] || exit 22
if [[ -n "\$out" ]]; then cp "\$file" "\$out"; else cat "\$file"; fi
EOF
    chmod +x "$WORK/fake-github/curl"
}

# `upgrade_to_new SHELL LOG` -- install the freshly built release through the
# new release's install-oneline.sh, fed by the staged assets.
upgrade_to_new() {
    local shell="$1" log="$2"
    [[ -n "${ASSETS:-}" ]] || stage_upgrade_assets
    PATH="$WORK/fake-github:$PATH" GPY_SHELL="$shell" sh "$ASSETS/install-oneline.sh" </dev/null >"$log" 2>&1
}

# `install_new SHELL` -- the freshly built archive's install.sh, as
# smoke-release.sh runs it; fails the scenario when it fails. Sets SHELL_TREES
# to the unpacked archive, a `gpy setup --from` source.
install_new() {
    local log="$WORK/install-new.log"
    mkdir -p "$WORK/archive"
    tar -xzf "$PAYLOAD/gpy-release.tar.gz" -C "$WORK/archive"
    if ! (cd "$WORK/archive/gpy-release" && GPY_SHELL="$1" bash ./install.sh </dev/null) >"$log" 2>&1; then
        show_log "$log"
        abort "install.sh from the new archive failed"
    fi
    # shellcheck disable=SC2034
    SHELL_TREES="$WORK/archive/gpy-release"
}

# `agent_field KEY` -- one field of `gpy-agent status` (key: value lines).
agent_field() {
    "$LOCAL_BIN/gpy-agent" status 2>/dev/null |
        awk -v key="$1" '{ k = $1; sub(/:$/, "", k); sub(/=.*/, "", k) } k == key { sub(/^[^:=]*[:=][ \t]*/, ""); print; exit }'
}

# `ipc JSON` -- one request line to the agent over the socket `gpy-agent
# status` names; prints the reply.
ipc() { printf '%s\n' "$1" | socat -t 5 - "UNIX-CONNECT:$(agent_field socket)"; }

# `make_repo DIR` -- a git repository with one commit.
make_repo() {
    mkdir -p "$1"
    git -C "$1" init -q -b main
    git -C "$1" config user.email scenario@example.com
    git -C "$1" config user.name Scenario
    git -C "$1" config commit.gpgsign false
    echo hello >"$1/tracked.txt"
    git -C "$1" add tracked.txt
    git -C "$1" commit -qm init
}
