#!/usr/bin/env bash
# tests/bash/release_glibc_floor.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Linux release binaries must run on every distro docs/INSTALL.md claims;
# GPY_GLIBC_FLOOR in release.yml drives everything:
#   (a) the workflow declares GPY_GLIBC_FLOOR as MAJOR.MINOR
#   (b) every linux-gnu target uses `cargo zigbuild --target <triple>.${GPY_GLIBC_FLOOR}`
#   (c) a Linux build step runs scripts/check-glibc-floor.sh on both binaries
#   (d) the checker rejects newer or absent GLIBC_ versions and accepts the floor
#   (e) the smoke job runs both x86_64 binaries in GPY_GLIBC_FLOOR_IMAGE
#   (f) docs/INSTALL.md states the same floor

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

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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

floor="$(sed -n 's/^  GPY_GLIBC_FLOOR: *"\{0,1\}\([0-9][0-9.]*\)"\{0,1\} *$/\1/p' "$WORKFLOW")"
if [[ "$floor" =~ ^[0-9]+\.[0-9]+$ ]]; then
    pass "release.yml declares GPY_GLIBC_FLOOR=$floor"
else
    fail "release.yml has no workflow-level GPY_GLIBC_FLOOR: \"MAJOR.MINOR\" (got '$floor')"
    floor="__missing__"
fi

# The workflow is run, not grepped: every `run:` step that applies to a matrix
# row is extracted, its `${{ matrix.* }}` expressions are filled in from that
# row, and it executes against stub `cargo`, checker and `docker` commands that
# record what they were asked to do. Rewording a step cannot break this; a step
# that stops building against the floor, or stops checking it, does.

# Matrix `include:` rows of a job block, one per line as tab-separated key=value.
matrix_rows() {
    awk '
        function kv(line,    k) { k = line; sub(/:.*/, "", k); sub(/^[^:]*: */, "", line); sub(/ *$/, "", line); return k "=" line }
        /^ *#/ { next }
        /^          - [a-z_]+:/ { if (row != "") print row; line = $0; sub(/^ +- /, "", line); row = kv(line); next }
        row != "" && /^            [a-z_]+:/ { line = $0; sub(/^ +/, "", line); row = row "\t" kv(line); next }
        row != "" { print row; row = "" }
        END { if (row != "") print row }
    ' <<<"$1"
}

# Value of key $2 in matrix row $1.
row_get() {
    local field
    local -a fields
    IFS=$'\t' read -r -a fields <<<"$1"
    for field in "${fields[@]}"; do
        [[ "${field%%=*}" == "$2" ]] && printf '%s' "${field#*=}" && return 0
    done
    return 1
}

# Write each step of job block $1 to $2/<n>.{if,wd,run}; print the step count.
split_steps() {
    awk -v dir="$2" '
        /^      - / { n++; inrun = 0; sub(/^      - /, "        ") }
        n == 0 { next }
        inrun && (/^          / || /^$/) { line = $0; sub(/^          /, "", line); print line > (dir "/" n ".run"); next }
        { inrun = 0 }
        /^        if: / { v = $0; sub(/^        if: */, "", v); print v > (dir "/" n ".if"); next }
        /^        working-directory: / { v = $0; sub(/^        working-directory: */, "", v); print v > (dir "/" n ".wd"); next }
        /^        run: \|$/ { inrun = 1; next }
        /^        run: / { v = $0; sub(/^        run: */, "", v); print v > (dir "/" n ".run"); next }
        END { print n + 0 }
    ' <<<"$1"
}

# Whether a step's `if:` holds for a target and runner OS: 0 runs, 1 skips,
# 2 is an expression this test cannot evaluate (a failure, not a skip).
step_applies() {
    local expr="$1" target="$2" runner_os="$3"
    local ends_re="^endsWith\\(matrix\\.target, '([^']*)'\\)\$"
    local os_re="^runner\\.os (==|!=) '([^']*)'\$"
    [[ -z "$expr" ]] && return 0
    if [[ "$expr" =~ $ends_re ]]; then
        [[ "$target" == *"${BASH_REMATCH[1]}" ]]
        return
    fi
    if [[ "$expr" =~ $os_re ]]; then
        if [[ "${BASH_REMATCH[1]}" == "==" ]]; then
            [[ "$runner_os" == "${BASH_REMATCH[2]}" ]]
        else
            [[ "$runner_os" != "${BASH_REMATCH[2]}" ]]
        fi
        return
    fi
    return 2
}

runner_os_of() {
    case "$1" in
        ubuntu-*) echo Linux ;;
        macos-*) echo macOS ;;
        windows-*) echo Windows ;;
        *) echo unknown ;;
    esac
}

# Run every step of the job split into $1 ($2 steps) that applies to matrix
# row $3 and whose script mentions $4, in sandbox $5. GitHub runs bash steps
# as `bash --noprofile --norc -eo pipefail`. Returns non-zero when a step
# fails, an `if:` cannot be evaluated, or an expression is left unfilled.
run_job_steps() {
    local steps="$1" count="$2" row="$3" pattern="$4" sandbox="$5"
    local target os n expr script field key status=0
    local -a fields
    target="$(row_get "$row" target)"
    os="$(runner_os_of "$(row_get "$row" os)")"
    for ((n = 1; n <= count; n++)); do
        [[ -f "$steps/$n.run" ]] || continue
        expr="$(cat "$steps/$n.if" 2>/dev/null)"
        step_applies "$expr" "$target" "$os"
        case $? in
            0) ;;
            1) continue ;;
            *)
                echo "unsupported step condition: $expr" >&2
                return 1
                ;;
        esac
        script="$(cat "$steps/$n.run")"
        grep -qE -- "$pattern" <<<"$script" || continue
        IFS=$'\t' read -r -a fields <<<"$row"
        for field in "${fields[@]}"; do
            key="${field%%=*}"
            script="${script//"\${{ matrix.$key }}"/${field#*=}}"
        done
        if [[ "$script" == *'${{'* ]]; then
            echo "unfilled expression in step $n: $script" >&2
            return 1
        fi
        printf '%s\n' "$script" >"$steps/$n.filled"
        (cd "$sandbox/$(cat "$steps/$n.wd" 2>/dev/null)" &&
            bash --noprofile --norc -eo pipefail "$steps/$n.filled") || status=1
    done
    return "$status"
}

STUBS="$WORKDIR/stubs"
SANDBOX="$WORKDIR/sandbox"
mkdir -p "$STUBS" "$SANDBOX/gpy-agent" "$SANDBOX/scripts" "$WORKDIR/tmp"
cat >"$STUBS/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$CALL_LOG"
EOF
cat >"$SANDBOX/scripts/check-glibc-floor.sh" <<'EOF'
#!/usr/bin/env bash
printf 'check %s\n' "$*" >>"$CALL_LOG"
EOF
chmod +x "$STUBS/cargo" "$SANDBOX/scripts/check-glibc-floor.sh"

build_job="$(job_block build)"
build_rows="$(matrix_rows "$build_job")"
mkdir -p "$WORKDIR/build_steps"
build_step_count="$(split_steps "$build_job" "$WORKDIR/build_steps")"
linux_rows="$(grep -E '(^|	)target=[a-z0-9_]+-unknown-linux-gnu(	|$)' <<<"$build_rows")"
if [[ -z "$linux_rows" ]]; then
    fail "no *-unknown-linux-gnu target in the build matrix"
fi

while IFS= read -r row; do
    [[ -n "$row" ]] || continue
    target="$(row_get "$row" target)"
    log="$WORKDIR/calls-$target"
    : >"$log"
    if ! CALL_LOG="$log" GPY_GLIBC_FLOOR="$floor" PATH="$STUBS:$PATH" TMPDIR="$WORKDIR/tmp" \
        run_job_steps "$WORKDIR/build_steps" "$build_step_count" "$row" 'cargo|check-glibc-floor' "$SANDBOX"; then
        fail "$target: the build job's steps did not run cleanly: $(cat "$log")"
        continue
    fi

    build_line="$(grep -n '^cargo ' "$log" | head -n 1)"
    if [[ "$build_line" == *"cargo zigbuild "* && " ${build_line#*:} " == *" --target $target.$floor "* ]]; then
        pass "$target links with cargo zigbuild against $target.$floor"
    else
        fail "$target is not built with cargo zigbuild --target $target.$floor (cargo calls: $(grep '^cargo ' "$log"))"
    fi
    if grep -q '^cargo build' "$log"; then
        fail "$target also runs a plain cargo build"
    fi
    profile="$(sed -n 's/.* --profile \([^ ]*\).*/\1/p' <<<"${build_line#*:}")"
    profile="${profile:-debug}"

    for bin in "$(row_get "$row" artifact_name)" "$(row_get "$row" cli_artifact_name)"; do
        check_line="$(grep -nxF "check gpy-agent/target/$target/$profile/$bin $floor" "$log" | head -n 1)"
        if [[ -n "$check_line" && "${check_line%%:*}" -gt "${build_line%%:*}" ]]; then
            pass "$target: the built $bin is checked against glibc $floor"
        else
            fail "$target: gpy-agent/target/$target/$profile/$bin is not checked against glibc $floor after the build (calls: $(cat "$log"))"
        fi
    done
done <<<"$linux_rows"

if [[ ! -x "$CHECKER" ]]; then
    fail "scripts/check-glibc-floor.sh is missing or not executable"
else
    # A stub objdump prints a canned symbol table, so this runs on any host.
    # It insists on -T: only the dynamic symbol table carries GLIBC_ versions.
    stub="$WORKDIR/objdump"
    cat >"$stub" <<'EOF'
#!/usr/bin/env bash
[[ " $* " == *" -T "* ]] || {
    echo "objdump stub: called without -T: $*" >&2
    exit 2
}
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

# Run the smoke job's floor step for a Linux runner. Stub `docker` runs the
# container command on the host with the mount point mapped to stubs of every
# release binary, which record how they were invoked.
FAKE_MOUNT="$WORKDIR/mount"
mkdir -p "$FAKE_MOUNT" "$WORKDIR/smoke_steps"
cat >"$STUBS/tar" <<'EOF'
#!/usr/bin/env bash
printf 'tar %s\n' "$*" >>"$CALL_LOG"
EOF
cat >"$STUBS/ldd" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$STUBS/docker" <<'EOF'
#!/usr/bin/env bash
# docker run [opts] IMAGE CMD...; only -v HOST:CONTAINER[:MODE] is honoured.
[[ "$1" == run ]] || exit 64
shift
mount=""
while [[ "$1" == -* ]]; do
    case "$1" in
        -v)
            mount="${2#*:}"
            mount="${mount%%:*}"
            shift 2
            ;;
        *) shift ;;
    esac
done
printf 'image %s\n' "$1" >>"$CALL_LOG"
shift
args=()
for arg in "$@"; do
    args+=("${arg//"$mount/"/"$FAKE_MOUNT/"}")
done
"${args[@]}"
EOF
chmod +x "$STUBS/tar" "$STUBS/ldd" "$STUBS/docker"
while IFS= read -r row; do
    for name in "$(row_get "$row" asset_name)" "$(row_get "$row" cli_asset_name)"; do
        [[ -n "$name" ]] || continue
        printf '#!/usr/bin/env bash\nprintf "ran %%s %%s\\n" "%s" "$*" >>"$CALL_LOG"\nexit "${FAIL_STATUS:-0}"\n' "$name" >"$FAKE_MOUNT/$name"
        chmod +x "$FAKE_MOUNT/$name"
    done
done <<<"$build_rows"

smoke_job="$(job_block smoke)"
smoke_step_count="$(split_steps "$smoke_job" "$WORKDIR/smoke_steps")"
x86_row="$(grep -E '(^|	)target=x86_64-unknown-linux-gnu(	|$)' <<<"$build_rows")"
run_floor_smoke() {
    CALL_LOG="$1" FAIL_STATUS="$2" FAKE_MOUNT="$FAKE_MOUNT" GPY_GLIBC_FLOOR_IMAGE="$image" \
        PATH="$STUBS:$PATH" TMPDIR="$WORKDIR/tmp" \
        run_job_steps "$WORKDIR/smoke_steps" "$smoke_step_count" "os=ubuntu-latest" docker "$SANDBOX"
}
smoke_log="$WORKDIR/calls-smoke"
: >"$smoke_log"
if [[ -z "$x86_row" ]]; then
    fail "no x86_64-unknown-linux-gnu row in the build matrix"
elif ! run_floor_smoke "$smoke_log" 0; then
    fail "the smoke job's floor step did not run cleanly: $(cat "$smoke_log")"
else
    if grep -qxF "image $image" "$smoke_log"; then
        pass "the smoke job's floor step runs in \$GPY_GLIBC_FLOOR_IMAGE ($image)"
    else
        fail "the smoke job's floor step does not run in $image (calls: $(cat "$smoke_log"))"
    fi
    for name in "$(row_get "$x86_row" asset_name)" "$(row_get "$x86_row" cli_asset_name)"; do
        if grep -qxF "ran $name --version" "$smoke_log"; then
            pass "the smoke job runs $name --version on the floor image"
        else
            fail "the smoke job does not run $name --version on the floor image (calls: $(cat "$smoke_log"))"
        fi
    done
    : >"$smoke_log"
    if run_floor_smoke "$smoke_log" 1 2>/dev/null; then
        fail "the smoke job's floor step passes when a binary fails to start"
    else
        pass "the smoke job's floor step fails when a binary fails to start"
    fi
fi

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

test_finish "the Linux release legs are pinned to and gated on glibc $floor"
