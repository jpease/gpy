#!/usr/bin/env bash
# tests/bash/release_asset_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #493: the release asset names exist in three independent
# places -- the build matrix in .github/workflows/release.yml (producer),
# EXPECTED_ASSETS in scripts/package-release.sh (packager), and the
# platform/arch `case` blocks in install.sh and install-oneline.sh (consumers).
# Before this test, each of those was checked against a fourth hardcoded list
# copied into the test files, so all four could agree with the constant in the
# test and still disagree with each other.
#
# Nothing here hardcodes an asset name. Every list is parsed out of the file
# that owns it and diffed against the others, so renaming a target, adding a
# platform, or dropping an installer arm fails on the mismatch itself:
#
#   1. Each matrix row's asset names are DERIVED from its Rust target triple,
#      so a row cannot claim a name that misidentifies what it builds.
#   2. All ten names are unique -- the original #493 bug was five agent builds
#      flattening onto one `gpy-agent` basename.
#   3. package-release.sh's EXPECTED_ASSETS == the matrix set exactly.
#   4. install.sh's resolved names == the matrix set exactly.
#   5. install-oneline.sh's resolved names == the matrix set minus the Windows
#      assets (curl|sh has no Windows path; install.sh keeps the mingw arm).
#   6. Every installer arm pairs an agent asset with the CLI asset for the same
#      platform/arch, so a swapped or copy-pasted arm is caught even though the
#      set as a whole would still balance.
#   7. The release job publishes the verification metadata the installers now
#      require -- checksums, SBOM and attestation (#494).

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

WORKFLOW="$ROOT/.github/workflows/release.yml"
PACKAGER="$ROOT/scripts/package-release.sh"
INSTALLER="$ROOT/install.sh"
ONELINE="$ROOT/install-oneline.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

for required in "$WORKFLOW" "$PACKAGER" "$INSTALLER" "$ONELINE"; do
    [[ -f "$required" ]] || {
        echo "FAIL: missing source of truth: $required"
        exit 1
    }
done

# --- helpers ----------------------------------------------------------------

# Compare two newline-separated sets. Reports what each side is missing rather
# than just "they differ", so a failure names the file that needs the edit.
diff_sets() {
    local left_label="$1" left="$2" right_label="$3" right="$4"
    local left_file="$WORKDIR/left" right_file="$WORKDIR/right"

    printf '%s\n' "$left" | grep -v '^$' | sort -u >"$left_file"
    printf '%s\n' "$right" | grep -v '^$' | sort -u >"$right_file"

    local only_left only_right
    only_left="$(comm -23 "$left_file" "$right_file")"
    only_right="$(comm -13 "$left_file" "$right_file")"

    if [[ -n "$only_left" ]]; then
        fail "$right_label is missing asset(s) declared by $left_label: $(tr '\n' ' ' <<<"$only_left")"
    fi
    if [[ -n "$only_right" ]]; then
        fail "$left_label is missing asset(s) declared by $right_label: $(tr '\n' ' ' <<<"$only_right")"
    fi
    [[ -z "$only_left" && -z "$only_right" ]]
}

# gpy-agent-macos-aarch64 -> macos-aarch64; gpy-windows-x86_64.exe -> windows-x86_64
asset_slug() {
    local asset="${1%.exe}"
    case "$asset" in
        gpy-agent-*) printf '%s' "${asset#gpy-agent-}" ;;
        gpy-*) printf '%s' "${asset#gpy-}" ;;
        *) return 1 ;;
    esac
}

# --- 1. build matrix: names derived from the target triple ------------------

echo "--- build matrix (release.yml) ---"

# One row per target. Only the `strategy.matrix` block is parsed -- `target:`
# is also a key of the setup-rust-toolchain step below it, and reading the
# whole file would let that step's `${{ matrix.target }}` overwrite the last
# row's real triple.
matrix_block="$(awk '
    /^[[:space:]]+matrix:[[:space:]]*$/ { inside = 1; next }
    inside && /^[[:space:]]*runs-on:/   { inside = 0 }
    inside
' "$WORKFLOW")"

if [[ -z "$matrix_block" ]]; then
    echo "FAIL: found no strategy.matrix block in $WORKFLOW"
    exit 1
fi

row=-1
targets=()
agent_assets=()
cli_assets=()
agent_artifacts=()
cli_artifacts=()

while IFS= read -r line; do
    trimmed="${line#"${line%%[![:space:]]*}"}"
    case "$trimmed" in
        '#'*) continue ;;
        '- os: '*)
            row=$((row + 1))
            targets[row]=""
            agent_assets[row]=""
            cli_assets[row]=""
            agent_artifacts[row]=""
            cli_artifacts[row]=""
            ;;
        'target: '*) [[ $row -ge 0 ]] && targets[row]="${trimmed#*: }" ;;
        'asset_name: '*) [[ $row -ge 0 ]] && agent_assets[row]="${trimmed#*: }" ;;
        'cli_asset_name: '*) [[ $row -ge 0 ]] && cli_assets[row]="${trimmed#*: }" ;;
        'artifact_name: '*) [[ $row -ge 0 ]] && agent_artifacts[row]="${trimmed#*: }" ;;
        'cli_artifact_name: '*) [[ $row -ge 0 ]] && cli_artifacts[row]="${trimmed#*: }" ;;
    esac
done <<<"$matrix_block"

row_count=$((row + 1))
if [[ $row_count -lt 1 ]]; then
    echo "FAIL: parsed 0 build-matrix rows from $WORKFLOW (parser or workflow layout changed)"
    exit 1
fi
echo "parsed $row_count matrix row(s)"

matrix_assets=""
i=0
while [[ $i -lt $row_count ]]; do
    target="${targets[i]}"
    if [[ -z "$target" ]]; then
        fail "matrix row $i declares no target"
        i=$((i + 1))
        continue
    fi

    arch="${target%%-*}"
    case "$target" in
        *-linux-* | *-linux) os="linux" ;;
        *-apple-darwin) os="macos" ;;
        *-windows-*) os="windows" ;;
        *)
            fail "matrix row $i target '$target' maps to no known asset platform"
            i=$((i + 1))
            continue
            ;;
    esac
    ext=""
    [[ "$os" == "windows" ]] && ext=".exe"

    want_agent="gpy-agent-$os-$arch$ext"
    want_cli="gpy-$os-$arch$ext"

    [[ "${agent_assets[i]}" == "$want_agent" ]] ||
        fail "matrix row $i ($target) declares asset_name '${agent_assets[i]}', expected '$want_agent'"
    [[ "${cli_assets[i]}" == "$want_cli" ]] ||
        fail "matrix row $i ($target) declares cli_asset_name '${cli_assets[i]}', expected '$want_cli'"
    [[ "${agent_artifacts[i]}" == "gpy-agent$ext" ]] ||
        fail "matrix row $i ($target) declares artifact_name '${agent_artifacts[i]}', expected 'gpy-agent$ext'"
    [[ "${cli_artifacts[i]}" == "gpy$ext" ]] ||
        fail "matrix row $i ($target) declares cli_artifact_name '${cli_artifacts[i]}', expected 'gpy$ext'"

    matrix_assets="$matrix_assets${agent_assets[i]}
${cli_assets[i]}
"
    i=$((i + 1))
done

# --- 2. every asset name is unique ------------------------------------------

total_names="$(printf '%s' "$matrix_assets" | grep -cv '^$')"
unique_names="$(printf '%s' "$matrix_assets" | grep -v '^$' | sort -u | wc -l | tr -d ' ')"
if [[ "$total_names" -ne "$unique_names" ]]; then
    fail "matrix declares $total_names asset names but only $unique_names are unique (targets would overwrite each other)"
else
    echo "$total_names asset name(s), all unique"
fi

# --- 3. packager: EXPECTED_ASSETS == matrix ---------------------------------

echo "--- packager (scripts/package-release.sh) ---"

packager_assets="$(awk '
    /^EXPECTED_ASSETS=\(/ { inside = 1; next }
    inside && /^\)/       { inside = 0 }
    inside {
        gsub(/[ \t]/, "")
        if ($0 != "" && $0 !~ /^#/) print
    }
' "$PACKAGER")"

if [[ -z "$packager_assets" ]]; then
    fail "parsed no EXPECTED_ASSETS entries from $PACKAGER"
elif diff_sets "release.yml matrix" "$matrix_assets" "package-release.sh EXPECTED_ASSETS" "$packager_assets"; then
    echo "EXPECTED_ASSETS matches the build matrix"
fi

# --- 4/5. installers: resolved names == matrix ------------------------------

# Assignments look like `x86_64) BINARY="..."; CLI_BINARY="..." ;;`, so the
# agent pattern has to reject the CLI variable that shares its suffix: require
# the character before BINARY to be neither an underscore nor an uppercase
# letter (or line start).
parse_installer() {
    local file="$1" var="$2"
    sed -nE \
        -e "s/^$var=\"([^\"]*)\".*/\1/p" \
        -e "s/.*[^A-Z_]$var=\"([^\"]*)\".*/\1/p" \
        "$file"
}

check_installer_pairs() {
    local label="$1" agents="$2" clis="$3"
    local agent_count cli_count
    agent_count="$(printf '%s\n' "$agents" | grep -cv '^$')"
    cli_count="$(printf '%s\n' "$clis" | grep -cv '^$')"

    if [[ "$agent_count" -ne "$cli_count" ]]; then
        fail "$label resolves $agent_count agent asset(s) but $cli_count CLI asset(s); every arm must set both"
        return
    fi

    # Same file order, so arm N's agent asset and CLI asset must describe the
    # same platform/arch. A swapped arm balances as a set but fails here.
    local n=1 agent cli agent_slug cli_slug
    while [[ $n -le $agent_count ]]; do
        agent="$(printf '%s\n' "$agents" | grep -v '^$' | sed -n "${n}p")"
        cli="$(printf '%s\n' "$clis" | grep -v '^$' | sed -n "${n}p")"
        agent_slug="$(asset_slug "$agent")" ||
            fail "$label arm $n resolves '$agent', which is not a gpy asset name"
        cli_slug="$(asset_slug "$cli")" ||
            fail "$label arm $n resolves '$cli', which is not a gpy asset name"

        case "$agent" in
            gpy-agent-*) ;;
            *) fail "$label arm $n assigns the agent variable '$agent', which is not an agent asset" ;;
        esac
        case "$cli" in
            gpy-agent-*) fail "$label arm $n assigns the CLI variable the agent asset '$cli'" ;;
        esac
        [[ "$agent_slug" == "$cli_slug" ]] ||
            fail "$label arm $n pairs '$agent' with '$cli' (different platform/arch)"
        n=$((n + 1))
    done
}

echo "--- archive installer (install.sh) ---"

installer_agents="$(parse_installer "$INSTALLER" BINARY)"
installer_clis="$(parse_installer "$INSTALLER" CLI_BINARY)"
installer_assets="$installer_agents
$installer_clis"

if [[ -z "$installer_agents" || -z "$installer_clis" ]]; then
    fail "parsed no BINARY/CLI_BINARY assignments from $INSTALLER"
else
    check_installer_pairs "install.sh" "$installer_agents" "$installer_clis"
    if diff_sets "release.yml matrix" "$matrix_assets" "install.sh" "$installer_assets"; then
        echo "install.sh resolves exactly the assets the matrix builds"
    fi
fi

echo "--- one-line installer (install-oneline.sh) ---"

oneline_agents="$(parse_installer "$ONELINE" BINARY_NAME)"
oneline_clis="$(parse_installer "$ONELINE" CLI_BINARY_NAME)"
oneline_assets="$oneline_agents
$oneline_clis"

# install-oneline.sh is the curl|sh entry point and supports linux/darwin only;
# the Windows assets are reached through install.sh's mingw/msys/cygwin arm.
# Subtract them from the matrix set rather than exempting the installer
# wholesale, so a new non-Windows target still has to appear here.
matrix_non_windows="$(printf '%s\n' "$matrix_assets" | grep -v '^$' | grep -v -- '-windows-')"

if [[ -z "$oneline_agents" || -z "$oneline_clis" ]]; then
    fail "parsed no BINARY_NAME/CLI_BINARY_NAME assignments from $ONELINE"
else
    check_installer_pairs "install-oneline.sh" "$oneline_agents" "$oneline_clis"
    if diff_sets "release.yml matrix (non-Windows)" "$matrix_non_windows" "install-oneline.sh" "$oneline_assets"; then
        echo "install-oneline.sh resolves exactly the non-Windows assets the matrix builds"
    fi
fi

# --- 7. verification metadata reaches the release ----------------------------
#
# The installers refuse to install a binary without a published checksum
# (#494), so a workflow edit that stops uploading the sidecars would not break
# the release job -- it would break every install from that release instead.
# Assert the publication side of that contract here, where the workflow is
# already parsed.
echo "--- verification metadata is published ---"

release_files="$(awk '
    /^ *files: \|/ { inside = 1; next }
    inside && /^ *[a-z_]+:/ { exit }
    inside { gsub(/^ +| +$/, ""); if ($0 != "") print }
' "$WORKFLOW")"

for required in dist/SHA256SUMS dist/gpy-sbom.cdx.json 'dist/*.sha256' 'dist/gpy-release/bin/*'; do
    if ! grep -qxF "$required" <<<"$release_files"; then
        fail "release.yml does not publish $required"
    fi
done

# bin/* carries the per-binary sidecars install-oneline.sh fetches, so the
# packager must be told to produce an SBOM and the release job must attest.
grep -q -- '--sbom' "$WORKFLOW" ||
    fail "release.yml no longer passes --sbom to package-release.sh (packaging would abort)"
grep -q 'attest-build-provenance' "$WORKFLOW" ||
    fail "release.yml no longer attests build provenance"
grep -q 'attestations: write' "$WORKFLOW" ||
    fail "release.yml is missing the attestations: write permission the attestation step needs"

# --- 8. local bundle tooling and `just install` (#810) -------------------------
#
# `just build-all-platforms` is the only in-repo producer of bin/, and
# install.sh is the only consumer. A checkout has no bin/, so the justfile
# `install` recipe must not run install.sh, and the build script has to emit
# the exact payload install.sh verifies: agent + CLI + a sidecar for each.
echo "--- local bundle tooling (#810) ---"

BUILD_SCRIPT="$ROOT/scripts/build-release-binaries.sh"
JUSTFILE="$ROOT/justfile"
INSTALL_DEV="$ROOT/install-dev.fish"

for required in "$BUILD_SCRIPT" "$JUSTFILE" "$INSTALL_DEV"; do
    [[ -f "$required" ]] || {
        echo "FAIL: missing source of truth: $required"
        exit 1
    }
done

grep -q 'release-dist/gpy"' "$BUILD_SCRIPT" ||
    fail "build-release-binaries.sh never copies the gpy CLI (release-dist/gpy); install.sh refuses a bin/ without it"
grep -q '\.sha256' "$BUILD_SCRIPT" ||
    fail "build-release-binaries.sh writes no .sha256 sidecars; install.sh refuses unverified binaries (#494)"

# Every agent name the build script passes to build_target must be an agent
# asset install.sh resolves, and the CLI name derived from it a CLI asset.
built_agents="$(sed -nE 's/^[[:space:]]*(.*[^[:alnum:]_])?build_target "[^"]+" "([^"]+)".*/\2/p' "$BUILD_SCRIPT" | sort -u)"
if [[ -z "$built_agents" ]]; then
    fail "parsed no build_target calls from $BUILD_SCRIPT"
else
    while IFS= read -r built; do
        # build_target appends .exe itself for the Windows target.
        case "$built" in
            *-windows-*) built="$built.exe" ;;
        esac
        grep -qxF "$built" <<<"$installer_agents" ||
            fail "build-release-binaries.sh builds '$built', which install.sh's platform table does not resolve"
        grep -qxF "gpy-${built#gpy-agent-}" <<<"$installer_clis" ||
            fail "build-release-binaries.sh would stage the CLI for '$built' as 'gpy-${built#gpy-agent-}', which install.sh does not resolve"
    done <<<"$built_agents"
fi

install_recipe="$(awk '
    /^install:/ { inside = 1; next }
    inside && /^[^[:space:]]/ { exit }
    inside
' "$JUSTFILE")"
if [[ -z "$install_recipe" ]]; then
    fail "found no body for the 'install:' recipe in $JUSTFILE (recipe removed? then this check can go)"
elif grep -q 'install\.sh' <<<"$install_recipe"; then
    fail "justfile 'install:' recipe runs install.sh, which needs a release bin/ a checkout does not have"
fi

if grep -q -- '--bundle' "$INSTALL_DEV" && ! grep -q 'aarch64' "$INSTALL_DEV"; then
    fail "install-dev.fish keeps --bundle but never maps arm64 to aarch64 (install.sh expects gpy-agent-macos-aarch64)"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
