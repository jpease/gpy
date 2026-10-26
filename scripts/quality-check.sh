#!/usr/bin/env bash
# Full quality gate for GPY: run before committing. See --help for modes.

set -euo pipefail

# Under a git hook, GIT_DIR & co. point at this repo and `git -C` does not
# override them, so the shell suites' throwaway repos would act on it.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

# The bash suites need bash >= 4; macOS's /bin/bash is 3.2.
gpy_select_bash() {
    local candidate major
    for candidate in "$(command -v bash)" /opt/homebrew/bin/bash /usr/local/bin/bash; do
        [[ -x "$candidate" ]] || continue
        # shellcheck disable=SC2016
        major="$("$candidate" -c 'echo ${BASH_VERSINFO[0]}' 2>/dev/null || echo 0)"
        if [[ "${major:-0}" -ge 4 ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    command -v bash
}
GPY_BASH="$(gpy_select_bash)"

cd "$(dirname "$0")/.."
QC_ROOT="$PWD"

step() { echo "==> $1"; }
ok() { echo "ok: $1"; }
warn() { echo "warn: $1"; }
err() { echo "FAIL: $1"; }

CHECKS_PASSED=0
CHECKS_FAILED=0
# Checks that printed a `SKIP:` line. Locally a skip passes but is listed at
# the end; under CI the test helpers exit 1 on a skip instead.
CHECKS_SKIPPED_NAMES=""

run_check() {
    local name="$1" cmd="$2" check_log
    step "$name"
    check_log="$(mktemp "${TMPDIR:-/tmp}/gpy-qc-check.XXXXXX")"
    if eval "$cmd" 2>&1 | tee "$check_log"; then
        ok "$name"
        CHECKS_PASSED=$((CHECKS_PASSED + 1))
        if grep -q '^SKIP:' "$check_log"; then
            CHECKS_SKIPPED_NAMES="${CHECKS_SKIPPED_NAMES}${CHECKS_SKIPPED_NAMES:+, }$name"
        fi
        rm -f "$check_log"
    else
        err "$name"
        CHECKS_FAILED=$((CHECKS_FAILED + 1))
        rm -f "$check_log"
        return 1
    fi
}

# Missing optional tool: a skip locally, a failure under CI.
skip_tool() {
    echo "SKIP: $1 not available"
    [[ -z "${CI:-}" ]]
}

# with_tool TOOL CMD...: run CMD, or skip_tool when the TOOL binary is missing.
with_tool() {
    local tool="$1"
    shift
    command -v "$tool" >/dev/null 2>&1 || { skip_tool "$tool"; return; }
    "$@"
}

fish_files() {
    find . -name "*.fish" -type f -not -path "*/target/*" -not -path "*/.git/*" \
        -not -path "*/demo/.fixture-home/*" "$@" -print0
}

# The check_* functions are dispatched through run_check's eval.
# shellcheck disable=SC2329
check_fish_syntax() {
    local failed=0 file
    while IFS= read -r -d '' file; do
        if ! fish -n "$file" 2>/dev/null; then
            err "syntax error in $file"
            fish -n "$file" 2>&1 | sed 's/^/    /'
            failed=1
        fi
    done < <(fish_files)
    return "$failed"
}

# shellcheck disable=SC2329
check_fish_formatting() {
    command -v fish_indent >/dev/null 2>&1 || { skip_tool fish_indent; return; }
    local failed=0 file
    while IFS= read -r -d '' file; do
        if ! diff -q "$file" <(fish_indent <"$file" 2>/dev/null) >/dev/null 2>&1; then
            err "needs formatting (run --fix): $file"
            failed=1
        fi
    done < <(fish_files -not -path "*/tests/*")
    return "$failed"
}

# Discovered by glob so a new script cannot go unchecked. install-oneline.sh
# is POSIX sh: it runs under whatever /bin/sh the user has.
# shellcheck disable=SC2329
check_shellcheck() {
    command -v shellcheck >/dev/null 2>&1 || { skip_tool shellcheck; return; }
    local failed=0 checked=0 target
    for target in install.sh install-oneline.sh scripts/*.sh scripts/lib/*.sh tests/bash/*.test.bash \
        benchmarks/*/*.sh bash/*.bash bash/*/*.bash; do
        [[ -f "$target" ]] || continue
        checked=$((checked + 1))
        if [[ "$target" == install-oneline.sh ]]; then
            shellcheck -s sh "$target" || failed=1
        else
            shellcheck -x "$target" || failed=1
        fi
    done
    if [[ "$checked" -eq 0 ]]; then
        err "shellcheck matched no files"
        failed=1
    fi
    return "$failed"
}

# Stop any gpy-agent still bound under the hermetic XDG root $1 (agent binary
# in $2) before the root is deleted: politely through its socket, then
# TERM -> bounded wait -> KILL of whatever holds a socket under $1. Never by
# name, which would hit the developer's own agents.
stop_shell_test_agents() {
    local xdg_root="$1" agent_dir="$2"
    local sock pid pids="" tries alive
    if [[ -S "$xdg_root/run/gpy/gpy.sock" ]]; then
        XDG_RUNTIME_DIR="$xdg_root/run" XDG_CACHE_HOME="$xdg_root/cache" XDG_CONFIG_HOME="$xdg_root/config" \
            "$agent_dir/gpy-agent" stop &>/dev/null || true
    fi
    command -v lsof &>/dev/null || return 0
    while IFS= read -r sock; do
        pids+=" $(lsof -t "$sock" 2>/dev/null | tr '\n' ' ')"
    done < <(find "$xdg_root" -type s 2>/dev/null)
    [[ -n "${pids// /}" ]] || return 0
    for pid in $pids; do
        kill "$pid" 2>/dev/null || true
    done
    for ((tries = 0; tries < 30; tries++)); do
        alive=0
        for pid in $pids; do
            if kill -0 "$pid" 2>/dev/null; then alive=1; fi
        done
        ((alive)) || return 0
        sleep 0.1
    done
    for pid in $pids; do
        kill -9 "$pid" 2>/dev/null || true
    done
    return 0
}

SHELL_TEST_XDG_ROOT=""
SHELL_TEST_AGENT_DIR=""

# The parts --shell-only runs: `checks` (repo checks, Fish lint, shellcheck)
# and one entry per suite. CI shards the shell gate by part; the shard list
# in pr-gate.yml must cover every part (tests/bash/ci_gate_shape).
SHELL_PARTS_ALL="checks,fish,bash,zsh"
SHELL_PARTS="$SHELL_PARTS_ALL"

shell_part() { [[ ",$SHELL_PARTS," == *",$1,"* ]]; }

# Idempotent: also runs from the traps while a test file is running.
shell_test_root_end() {
    [[ -n "$SHELL_TEST_XDG_ROOT" ]] || return 0
    stop_shell_test_agents "$SHELL_TEST_XDG_ROOT" "$SHELL_TEST_AGENT_DIR"
    rm -rf "$SHELL_TEST_XDG_ROOT"
    SHELL_TEST_XDG_ROOT=""
}

# Runs each test file matching $3 with interpreter $2. Every file gets its own
# XDG root, so a developer's theme cache cannot leak in and whatever agent the
# file autostarted is stopped as soon as it ends.
run_shell_suite() {
    local label="$1" runner="$2" pattern="$3" test_file base xdg
    local -a files=()
    for test_file in $pattern; do
        [[ -e "$test_file" ]] && files+=("$test_file")
    done
    [[ ${#files[@]} -gt 0 ]] || return 0
    if ! command -v "$runner" &>/dev/null; then
        err "$label: ${#files[@]} test file(s) not run: $runner not found"
        CHECKS_FAILED=$((CHECKS_FAILED + 1))
        return 0
    fi
    for test_file in "${files[@]}"; do
        base="$(basename "$test_file")"
        SHELL_TEST_XDG_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/gpy-qc-xdg.XXXXXX")"
        mkdir -p "$SHELL_TEST_XDG_ROOT/cache" "$SHELL_TEST_XDG_ROOT/config" "$SHELL_TEST_XDG_ROOT/run"
        chmod 700 "$SHELL_TEST_XDG_ROOT/run"
        xdg="XDG_RUNTIME_DIR=$SHELL_TEST_XDG_ROOT/run XDG_CACHE_HOME=$SHELL_TEST_XDG_ROOT/cache XDG_CONFIG_HOME=$SHELL_TEST_XDG_ROOT/config "
        run_check "$label $base" "${xdg}PATH=\"\$SHELL_TEST_AGENT_DIR:\$PATH\" $runner $test_file" || true
        shell_test_root_end
    done
}

# Fish, Bash and Zsh suites against this checkout's debug agent. They run
# whenever the shells or the agent change, since they check rendered output.
run_shell_tests() {
    shell_part fish || shell_part bash || shell_part zsh || return 0
    # The suites must never touch this repo; fail loudly if they did.
    local head_before worktrees_before head_after worktrees_after
    head_before="$(git rev-parse HEAD 2>/dev/null || echo none)"
    worktrees_before="$(git worktree list 2>/dev/null | wc -l | tr -d ' ')"

    # Always build: an existence check would let a stale binary pass.
    if ! run_check "Build debug binaries" "(cd gpy-agent && cargo build --quiet --bin gpy --bin gpy-agent)"; then
        return 0
    fi
    if shell_part fish; then
        run_check "Fish tests" "./scripts/test_fish.sh" || true
    fi

    SHELL_TEST_AGENT_DIR="${CARGO_TARGET_DIR:-$PWD/gpy-agent/target}/debug"
    trap 'shell_test_root_end' EXIT
    trap 'shell_test_root_end; exit 130' INT
    trap 'shell_test_root_end; exit 143' TERM
    if shell_part bash; then
        run_shell_suite "Bash" "$GPY_BASH" "tests/bash/*.test.bash"
    fi
    if shell_part zsh; then
        run_shell_suite "Zsh" "zsh" "tests/zsh/*.test.zsh"
    fi
    trap - EXIT INT TERM

    head_after="$(git rev-parse HEAD 2>/dev/null || echo none)"
    worktrees_after="$(git worktree list 2>/dev/null | wc -l | tr -d ' ')"
    if [[ "$head_before" != "$head_after" || "$worktrees_before" != "$worktrees_after" ]]; then
        err "a shell test mutated this repo (HEAD $head_before -> $head_after, worktrees $worktrees_before -> $worktrees_after); inspect 'git reflog' and 'git worktree list'"
        CHECKS_FAILED=$((CHECKS_FAILED + 1))
    fi
}

# RUSTUP_TOOLCHAIN outranks rust-toolchain.toml, and a newer rustc raises lints
# the pin does not; abort before spending a clippy run on the wrong compiler.
require_pinned_toolchain() {
    if ! ./scripts/check-active-toolchain.sh; then
        err "active Rust toolchain is not the pin; refusing to run Rust checks"
        exit 1
    fi
}

require_tools() {
    local tool missing=0
    for tool in "$@"; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            err "$tool is not installed"
            missing=1
        fi
    done
    ((missing == 0)) || exit 1
}

run_repo_checks() {
    run_check "License metadata" "./scripts/check-license-metadata.sh" || true
    run_check "Privacy patterns" "./scripts/check-privacy-patterns.sh" || true
    run_check "Documentation links" "./scripts/check-doc-links.sh" || true
    run_check "File size" "python3 ./scripts/check_file_size.py" || true
}

run_fish_lint() {
    run_check "Fish syntax" "check_fish_syntax" || true
    run_check "Fish formatting" "check_fish_formatting" || true
}

run_shell_checks() {
    if shell_part checks; then
        run_fish_lint
        run_check "Shellcheck" "check_shellcheck" || true
    fi
    run_shell_tests
}

# Every Clippy gate (here, `just lint`, the CI Rust leg) runs this, from
# gpy-agent/. Lint levels live in Cargo.toml. All targets first, with the
# `test-support` surface the self dev-dependency enables for test and bench
# builds; then the library and binaries as they ship, without it.
run_clippy() {
    cargo clippy --all-targets && cargo clippy --lib --bins
}

# Run from gpy-agent/. $1: extra nextest flags.
run_rust_lint_and_tests() {
    local nextest_flags="${1:-}" nextest_log retries
    run_check "Rust format check" "cargo fmt -- --check" || true
    run_check "Clippy" "run_clippy" || true
    nextest_log="$(mktemp)"
    run_check "Rust tests" "cargo nextest run${nextest_flags:+ $nextest_flags} 2>&1 | tee \"$nextest_log\"" || true
    # Retries come only from named overrides in .config/nextest.toml; its
    # final-status-level = "flaky" prints a FLAKY line per retried test.
    retries=$(grep -cE '^ *FLAKY [0-9]+/[0-9]+ ' "$nextest_log" || true)
    if [[ "${retries:-0}" -gt 0 ]]; then
        warn "$retries test(s) needed a retry"
    fi
    rm -f "$nextest_log"
    # Reap agents orphaned by killed Rust test runs.
    if [[ -x "$QC_ROOT/scripts/cleanup-test-agents.sh" ]]; then
        "$QC_ROOT/scripts/cleanup-test-agents.sh"
    fi
}

run_rust_deep_checks() {
    run_check "Doc tests" "cargo test --doc" || true
    run_check "Security audit" "with_tool cargo-audit cargo audit" || true
    run_check "Dependency check" "with_tool cargo-deny cargo deny check" || true
    # --with-metadata also resolves bench and dev targets, catching unused
    # dev-dependencies.
    run_check "Unused dependencies" "with_tool cargo-machete cargo machete --with-metadata" || true
}

finish_report() {
    echo
    echo "$1 summary: $CHECKS_PASSED passed, $CHECKS_FAILED failed"
    [[ -z "$CHECKS_SKIPPED_NAMES" ]] || echo "Skipped: $CHECKS_SKIPPED_NAMES"
    exit $((CHECKS_FAILED > 0))
}

main() {
    require_pinned_toolchain
    # Use the wrapper from Cargo's config, not an environment override.
    unset RUSTC_WRAPPER
    require_tools cargo rustc rustfmt fish cargo-audit cargo-deny
    cargo clippy -V >/dev/null 2>&1 || { err "cargo clippy missing (rustup component add clippy)"; exit 1; }
    cargo nextest --version >/dev/null 2>&1 || { err "cargo nextest missing (cargo install cargo-nextest)"; exit 1; }

    run_repo_checks
    run_shell_checks

    pushd gpy-agent >/dev/null
    run_rust_lint_and_tests --no-fail-fast
    run_rust_deep_checks
    # The release build is the slowest leg; --rust-only (and CI) always
    # runs it, the local full gate only under CI or GPY_GATE_RELEASE=1.
    if [[ -n "${CI:-}" || -n "${GPY_GATE_RELEASE:-}" ]]; then
        run_check "Release build" "cargo build --release" || true
    else
        warn "skipping release build (set GPY_GATE_RELEASE=1 to run it)"
    fi
    popd >/dev/null

    finish_report "Quality"
}

show_help() {
    cat <<EOF
Usage: $0 [OPTION]

    (none)        Full gate: repo checks, shell lint and suites, Rust checks
    --fast        Repo checks, Fish syntax, shellcheck, parity tests, Rust lint and tests
    --rust-only   fmt, clippy, tests, doc tests, audit, deny, unused dependencies, release build
    --shell-only [PARTS]
                  Fish lint, shellcheck and the Fish/Bash/Zsh suites (no Rust
                  checks). PARTS limits it to a comma-separated subset of
                  $SHELL_PARTS_ALL; \`checks\` is the repo checks, Fish lint
                  and shellcheck
    --fish-only   Fish syntax and formatting
    --clippy      Clippy only: the exact commands the gate runs (also \`just lint\`)
    --fix         Apply fish_indent, cargo fmt and clippy fixes, then run the full gate
    -h, --help    Show this help

Requires cargo (with clippy, rustfmt, nextest), fish, cargo-audit and cargo-deny.
Optional: zsh. A missing fish_indent, shellcheck or cargo-machete (and, under
--rust-only, cargo-audit or cargo-deny) is skipped locally and fails under CI.
EOF
}

case "${1:-}" in
    -h|--help)
        show_help
        ;;
    --clippy)
        require_pinned_toolchain
        pushd gpy-agent >/dev/null
        run_check "Clippy" "run_clippy" || true
        popd >/dev/null
        finish_report "Clippy"
        ;;
    --fix)
        require_pinned_toolchain
        if command -v fish_indent >/dev/null 2>&1; then
            step "Formatting Fish files"
            fish_files -not -path "*/tests/*" | while IFS= read -r -d '' file; do
                fish_indent -w "$file" 2>/dev/null || warn "could not format $file"
            done
        fi
        pushd gpy-agent >/dev/null
        step "Formatting Rust files"
        cargo fmt
        step "Applying clippy fixes"
        cargo clippy --fix --allow-dirty --allow-staged --all-targets 2>/dev/null || warn "some clippy fixes need manual work"
        popd >/dev/null
        main
        ;;
    --fast)
        require_pinned_toolchain
        run_repo_checks
        run_check "Fish syntax" "check_fish_syntax" || true
        run_check "Shellcheck" "check_shellcheck" || true
        run_check "Bash parity tests" "$GPY_BASH tests/bash/parity.test.bash" || true
        if command -v zsh &>/dev/null; then
            run_check "Zsh parity tests" "zsh tests/zsh/parity.test.zsh" || true
        fi
        pushd gpy-agent >/dev/null
        run_rust_lint_and_tests ""
        popd >/dev/null
        finish_report "Fast"
        ;;
    --rust-only)
        require_pinned_toolchain
        run_repo_checks
        pushd gpy-agent >/dev/null
        run_rust_lint_and_tests --no-fail-fast
        run_rust_deep_checks
        run_check "Release build" "cargo build --release" || true
        popd >/dev/null
        finish_report "Rust"
        ;;
    --shell-only)
        if [[ -n "${2:-}" ]]; then
            IFS=, read -r -a parts <<<"$2"
            for part in "${parts[@]}"; do
                if [[ ",$SHELL_PARTS_ALL," != *",$part,"* ]]; then
                    echo "Unknown --shell-only part: '$part' (one of $SHELL_PARTS_ALL)" >&2
                    exit 1
                fi
            done
            SHELL_PARTS="$2"
        fi
        if shell_part checks; then
            run_repo_checks
        fi
        run_shell_checks
        finish_report "Shell"
        ;;
    --fish-only)
        run_fish_lint
        finish_report "Fish"
        ;;
    "")
        main
        ;;
    *)
        echo "Unknown option: $1 (see --help)" >&2
        exit 1
        ;;
esac
