#!/usr/bin/env bash
# Comprehensive code quality check script for GPY

set -euo pipefail

# When this script runs from a git hook (e.g. pre-push), git exports GIT_DIR,
# GIT_WORK_TREE and friends pointing at THIS repo. `git -C <tmp>` does NOT
# override those env vars, so the shell integration tests — which create their
# own throwaway repos and run `git worktree add` — would otherwise operate on
# the real repo, moving HEAD and leaking branches/worktrees into it (#275).
# Clear the repo-scoping git env for the whole run so every child git command
# resolves its repository from its own cwd/-C argument.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

# Pick a bash >= 4 for the bash test suites. macOS's system /bin/bash is 3.2,
# which lacks associative arrays and resets traps in subshells, so the suites
# must run under a newer bash (e.g. Homebrew's). Falls back to PATH bash.
gpy_select_bash() {
    local candidate major
    for candidate in "$(command -v bash)" /opt/homebrew/bin/bash /usr/local/bin/bash; do
        [[ -x "$candidate" ]] || continue
        # Evaluated by $candidate, not by this shell, so it stays unexpanded.
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

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_step() {
    echo -e "${BLUE}==> $1${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# Track results
CHECKS_PASSED=0
CHECKS_FAILED=0
# Checks whose output carried a `SKIP:` line (the shared skip contract,
# #650). Locally a skip passes; the tally makes it visible at the end so a
# green run never quietly means "half the suite did not execute". Under CI
# the helpers exit 1 on a skip, so these land in CHECKS_FAILED instead.
CHECKS_SKIPPED_NAMES=""

run_check() {
    local name="$1"
    local cmd="$2"
    local check_log skip_lines

    print_step "Running $name"

    # Stream the output and keep a copy to count skips; `pipefail` (set
    # above) makes the pipeline's status the command's own.
    check_log="$(mktemp "${TMPDIR:-/tmp}/gpy-qc-check.XXXXXX")"
    if eval "$cmd" 2>&1 | tee "$check_log"; then
        print_success "$name passed"
        ((CHECKS_PASSED++))
        skip_lines=$(grep -c '^SKIP:' "$check_log" 2>/dev/null || true)
        if [[ "${skip_lines:-0}" -gt 0 ]]; then
            CHECKS_SKIPPED_NAMES="${CHECKS_SKIPPED_NAMES}${CHECKS_SKIPPED_NAMES:+, }$name"
        fi
        rm -f "$check_log"
    else
        print_error "$name failed"
        ((CHECKS_FAILED++))
        rm -f "$check_log"
        return 1
    fi
}

# `Skipped: N (names)` for the summaries below (#650).
print_skip_tally() {
    local count=0
    if [[ -n "$CHECKS_SKIPPED_NAMES" ]]; then
        count=$(printf '%s' "$CHECKS_SKIPPED_NAMES" | awk -F', ' '{print NF}')
        echo -e "Skipped: ${YELLOW}$count${NC} ($CHECKS_SKIPPED_NAMES)"
    else
        echo -e "Skipped: ${GREEN}0${NC}"
    fi
}

# gpy-agent#620: profile.default no longer retries the whole suite, so a
# retry now only happens for a named, issue-linked override (see
# gpy-agent/.config/nextest.toml) -- but nextest's default terminal output
# doesn't surface that on its own. `status-level = "retry"` and
# `final-status-level = "flaky"` in that config make nextest print a
# "TRY 2 ..." line for each retried attempt and a "FLAKY N/M ..." line per
# retried test in the final summary; this greps a captured nextest run's
# output for those and prints one non-failing warning line so a retry is
# visible in the gate's own output rather than silently green. Takes the
# path to a file already holding that run's captured stdout/stderr.
report_nextest_retries() {
    local log_file="$1" retry_count

    [[ -f "$log_file" ]] || return 0

    retry_count=$(grep -cE '^ *FLAKY [0-9]+/[0-9]+ ' "$log_file" 2>/dev/null || true)
    retry_count=${retry_count:-0}

    if ((retry_count > 0)); then
        print_warning "$retry_count test(s) needed a retry (see $log_file for which)"
    fi
}

cleanup_test_agents() {
    if [[ -x "./scripts/cleanup-test-agents.sh" ]]; then
        print_step "Cleaning orphaned test agents"
        ./scripts/cleanup-test-agents.sh
    fi
}

# Derive a human-readable run_check label from a discovered shell test file,
# e.g. shell_test_label "Bash" "is_first_instant_cache.test.bash" ->
# "Bash Is first instant cache tests". Exact wording isn't load-bearing, only
# readability/distinguishability in output (#483).
shell_test_label() {
    local shell_name="$1" base="$2" words first_char rest
    words="${base%.test.*}"
    words="${words//_/ }"
    first_char="$(printf '%s' "${words:0:1}" | tr '[:lower:]' '[:upper:]')"
    rest="${words:1}"
    printf '%s %s%s tests' "$shell_name" "$first_char" "$rest"
}

# Check Fish syntax for all .fish files
# Dispatched by name through run_check, which evals its second argument,
# so shellcheck cannot see the call site.
# shellcheck disable=SC2329
check_fish_syntax() {
    local failed=0

    print_step "Checking Fish syntax"

    # Every .fish file must parse. (A `tests/fish/render/` exemption for a
    # fishtape-era framework used to live here; those files are gone, #655.)
    while IFS= read -r -d '' file; do

        if fish -n "$file" 2>/dev/null; then
            echo "  ✓ $file"
        else
            print_error "Syntax error in $file"
            fish -n "$file" 2>&1 | sed 's/^/    /'
            failed=1
        fi
    done < <(find . -name "*.fish" -type f -not -path "*/target/*" -not -path "*/.git/*" -not -path "*/demo/.fixture-home/*" -print0)

    if [ "$failed" -eq 0 ]; then
        print_success "All Fish files have valid syntax"
        return 0
    else
        print_error "Some Fish files have syntax errors"
        return 1
    fi
}

# Check Fish code formatting (if fish_indent is available)
# Dispatched by name through run_check, which evals its second argument,
# so shellcheck cannot see the call site.
# shellcheck disable=SC2329
check_fish_formatting() {
    if ! command -v fish_indent >/dev/null 2>&1; then
        print_warning "fish_indent not available, skipping formatting check"
        return 0
    fi

    local failed=0

    print_step "Checking Fish formatting"

    while IFS= read -r -d '' file; do
        # Create temp file for formatted version
        local temp_file
        temp_file=$(mktemp)
        fish_indent < "$file" > "$temp_file" 2>/dev/null || {
            print_error "Failed to format $file"
            rm -f "$temp_file"
            failed=1
            continue
        }

        # Compare with original
        if ! diff -q "$file" "$temp_file" >/dev/null 2>&1; then
            print_error "Formatting issues in $file"
            failed=1
        fi

        rm -f "$temp_file"
    done < <(find . -name "*.fish" -type f -not -path "*/target/*" -not -path "*/.git/*" -not -path "*/tests/*" -not -path "*/demo/.fixture-home/*" -print0)

    if [ "$failed" -eq 0 ]; then
        print_success "All Fish files are properly formatted"
        return 0
    else
        print_error "Some Fish files need formatting (run with --fix to auto-format)"
        return 1
    fi
}

# Shellcheck every shell script in the repository.
#
# #494 introduced this scoped to the installers -- the scripts that run on a
# stranger's machine as `curl | sh` -- because the rest of the tree carried a
# backlog of findings. #510 cleared that backlog (78 findings across 21 files),
# so the scope is now everything.
#
# Discovered by glob rather than listed. A hand-maintained list drifts: a new
# script is simply never checked, and nothing says so. The same reasoning gave
# tests/{bash,zsh} their glob-plus-meta-check in #483.
#
# install-oneline.sh is the one exception to shebang-based detection: it is
# POSIX sh, because it runs under whatever /bin/sh the user has.
#
# Keep "shellcheck" off the start of a wrapped comment line: a comment
# beginning `# shellcheck ...` is parsed as a directive, and an unparseable
# one (SC1072/SC1073) aborts analysis of the whole file.
# Invoked as `done < <(shellcheck_targets)`; SC2329 does not trace process
# substitution back to the definition.
# shellcheck disable=SC2329
shellcheck_targets() {
    printf '%s\n' install.sh install-oneline.sh
    # Nullglob is not assumed: the -f guard in the loop skips a stale match.
    printf '%s\n' scripts/*.sh tests/bash/*.test.bash
    # The shipped Bash integration is sourced into every interactive shell
    # (#819).
    printf '%s\n' bash/*.bash bash/*/*.bash
}

# Dispatched by name through run_check, which evals its second argument,
# so shellcheck cannot see the call site.
# shellcheck disable=SC2329
check_shellcheck() {
    if ! command -v shellcheck >/dev/null 2>&1; then
        print_warning "shellcheck not available, skipping shell lint (brew install shellcheck)"
        return 0
    fi

    local failed=0 target checked=0

    print_step "Shellchecking all shell scripts"

    while IFS= read -r target; do
        [[ -f "$target" ]] || continue
        checked=$((checked + 1))

        # -x follows `source`d files instead of reporting SC1091 for each one,
        # which both silences 13 false positives and checks more (#510).
        local shellcheck_ok=0
        if [[ "$target" == "install-oneline.sh" ]]; then
            shellcheck -s sh "$target" || shellcheck_ok=1
        else
            shellcheck -x "$target" || shellcheck_ok=1
        fi

        if [[ $shellcheck_ok -eq 0 ]]; then
            echo "  ✓ $target"
        else
            print_error "shellcheck findings in $target"
            failed=1
        fi
    done < <(shellcheck_targets)

    # A glob that matches nothing would otherwise pass silently.
    if [[ "$checked" -eq 0 ]]; then
        print_error "shellcheck matched no files -- the discovery glob is broken"
        failed=1
    fi

    if [ "$failed" -eq 0 ]; then
        print_success "All $checked shell scripts are shellcheck-clean"
        return 0
    else
        print_error "Some shell scripts have shellcheck findings"
        return 1
    fi
}

# Run the shell integration test suites (Fish, Bash, Zsh).
#
# These exercise the agent's rendered output as consumed by each shell, so they
# must run whenever either the shells OR the agent change — e.g. starship_parity
# checks fish rendering against the agent binary. `test_fish.sh` builds the debug
# agent binary the parity tests depend on. Assumes the caller has cd'd to the
# project root.
run_shell_tests() {
    # Tripwire: the shell suites spin up throwaway git repos; none of them should
    # ever touch THIS repo. Snapshot HEAD + worktree count before running and
    # verify afterwards, so a future isolation regression (see #275) fails loudly
    # here instead of silently corrupting the developer's checkout.
    local repo_head_before repo_worktrees_before
    repo_head_before="$(git rev-parse HEAD 2>/dev/null || echo none)"
    repo_worktrees_before="$(git worktree list 2>/dev/null | wc -l | tr -d ' ')"

    # Fish integration suite (auto-discovers tests/fish/*.test.fish).
    run_check "Fish tests" "./scripts/test_fish.sh" || true

    # Bash/Zsh integration suites, discovered by glob (tests/{bash,zsh}/*.test.*)
    # so a new test file runs with no further edits (#483). A couple of tests
    # need per-file env overrides; those are looked up via a `case` (not an
    # associative array -- this script's own shebang is not guaranteed a
    # bash >= 4, only the child test processes are routed through $GPY_BASH).
    local shell_tests_invoked=""

    # Hermetic XDG dirs for every Bash/Zsh test (#632): __gpy_load_theme sources
    # $XDG_CACHE_HOME/gpy/theme-export.<shell> when it exists (#614), so without
    # this a developer's own theme cache leaks into tests that assert on the
    # built-in defaults. Mirrors the Fish runner's `fish --no-config` (#630).
    local shell_xdg_root
    shell_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-qc-xdg.XXXXXX")"
    mkdir -p "$shell_xdg_root/cache" "$shell_xdg_root/config"
    local shell_xdg_env="XDG_CACHE_HOME=$shell_xdg_root/cache XDG_CONFIG_HOME=$shell_xdg_root/config "

    if command -v bash &>/dev/null; then
        local bash_test bash_base bash_env bash_label
        for bash_test in tests/bash/*.test.bash; do
            [[ -e "$bash_test" ]] || continue
            bash_base="$(basename "$bash_test")"
            case "$bash_base" in
                integration.test.bash)
                    bash_env="GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=/tmp/.gpy-qc-missing-$$.sock "
                    ;;
                e2e_*.test.bash)
                    # Live-daemon tests (#646): tests/lib/shell_e2e.sh builds
                    # the sandbox (HOME, XDG_*, a socket under the GPY test
                    # root) itself, so nothing here may point them at a
                    # missing socket or a disabled supervisor.
                    bash_env=""
                    ;;
                *)
                    bash_env=""
                    ;;
            esac
            bash_label="$(shell_test_label "Bash" "$bash_base")"
            run_check "$bash_label" "${shell_xdg_env}${bash_env}$GPY_BASH $bash_test" || true
            shell_tests_invoked="$shell_tests_invoked $bash_test"
        done
    fi
    if command -v zsh &>/dev/null; then
        local zsh_test zsh_base zsh_env zsh_label
        for zsh_test in tests/zsh/*.test.zsh; do
            [[ -e "$zsh_test" ]] || continue
            zsh_base="$(basename "$zsh_test")"
            case "$zsh_base" in
                integration.test.zsh)
                    zsh_env="GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=/tmp/.gpy-qc-missing-$$.sock "
                    ;;
                e2e_*.test.zsh)
                    # Live-daemon tests (#646); see the bash case above.
                    zsh_env=""
                    ;;
                *)
                    zsh_env=""
                    ;;
            esac
            zsh_label="$(shell_test_label "Zsh" "$zsh_base")"
            run_check "$zsh_label" "${shell_xdg_env}${zsh_env}zsh $zsh_test" || true
            shell_tests_invoked="$shell_tests_invoked $zsh_test"
        done
    fi
    rm -rf "$shell_xdg_root"

    # Meta-check (#483): fail loudly if a tests/{bash,zsh}/*.test.* file on
    # disk was not actually invoked above -- e.g. because bash/zsh wasn't
    # found, or a future filtering bug narrows the glob. Catches a regression
    # in the discovery wiring itself, not just in an individual test.
    local expected_test found_test is_skipped
    for expected_test in tests/bash/*.test.bash tests/zsh/*.test.zsh; do
        [[ -e "$expected_test" ]] || continue
        is_skipped=1
        for found_test in $shell_tests_invoked; do
            if [[ "$found_test" == "$expected_test" ]]; then
                is_skipped=0
                break
            fi
        done
        if [[ "$is_skipped" -eq 1 ]]; then
            print_error "Shell test file $expected_test exists but was not run by this gate (see #483)."
            CHECKS_FAILED=$((CHECKS_FAILED + 1))
        fi
    done

    local repo_head_after repo_worktrees_after
    repo_head_after="$(git rev-parse HEAD 2>/dev/null || echo none)"
    repo_worktrees_after="$(git worktree list 2>/dev/null | wc -l | tr -d ' ')"
    if [[ "$repo_head_before" != "$repo_head_after" || "$repo_worktrees_before" != "$repo_worktrees_after" ]]; then
        print_error "A shell test mutated THIS repo (HEAD ${repo_head_before} -> ${repo_head_after}, worktrees ${repo_worktrees_before} -> ${repo_worktrees_after})."
        print_error "Test git-repo isolation has regressed — see #275. Inspect 'git reflog' and 'git worktree list'."
        CHECKS_FAILED=$((CHECKS_FAILED + 1))
    fi

    cleanup_test_agents
}

# Fail fast when the active rustc is not the gpy-agent/rust-toolchain.toml pin
# (#823). RUSTUP_TOOLCHAIN outranks the file, and a newer rustc raises lints the
# pin does not, so without this the gate reports "findings" that are really a
# toolchain override. Aborts the whole run: every Rust check below would be
# measuring the wrong compiler, and the clippy run is the slow part to waste.
require_pinned_toolchain() {
    if ! "$(dirname "$0")/check-active-toolchain.sh"; then
        print_error "Active Rust toolchain is not the pin; refusing to run Rust checks (see message above)"
        exit 1
    fi
}

# Main quality checks
main() {
    require_pinned_toolchain

    echo -e "${BLUE}🐟 GPY Code Quality Check${NC}"
    echo "=========================="
    echo

    # Move to project root
    cd "$(dirname "$0")/.."
    PROJECT_ROOT="$(pwd)"

    # Clear any RUSTC_WRAPPER environment override so the wrapper configured
    # in Cargo's config (e.g. kache) is the one these checks use.
    if [[ -n "${RUSTC_WRAPPER:-}" ]]; then
        print_warning "Clearing RUSTC_WRAPPER=${RUSTC_WRAPPER} for quality checks"
        unset RUSTC_WRAPPER
    fi

    # Check if required tools are installed
    print_step "Checking required tools"

    # Core Rust toolchain
    for tool in cargo rustc rustfmt; do
        if command -v "$tool" >/dev/null 2>&1; then
            print_success "$tool is installed"
        else
            print_error "$tool is not installed"
            exit 1
        fi
    done

    # Check clippy availability via cargo
    if cargo clippy -V >/dev/null 2>&1; then
        print_success "cargo clippy is available"
    else
        print_error "cargo clippy is not available (install with: rustup component add clippy)"
        exit 1
    fi

    # Check nextest availability
    if cargo nextest --version >/dev/null 2>&1; then
        print_success "cargo nextest is available"
    else
        print_error "cargo nextest is not installed (install with: brew install cargo-nextest or cargo install cargo-nextest)"
        exit 1
    fi

    # Fish shell
    if command -v fish >/dev/null 2>&1; then
        print_success "fish is installed ($(fish --version))"
    else
        print_error "fish is not installed"
        exit 1
    fi

    # Optional but recommended tools
    if command -v fish_indent >/dev/null 2>&1; then
        print_success "fish_indent is available"
    else
        print_warning "fish_indent not installed (formatting checks will be skipped)"
    fi

    # Supply-chain security tools (required for the full quality gate)
    local security_ok=1
    for tool in cargo-audit cargo-deny; do
        if command -v "$tool" >/dev/null 2>&1; then
            print_success "$tool is installed"
        else
            print_error "$tool is not installed (required — install with: cargo install $tool)"
            security_ok=0
        fi
    done
    if [[ $security_ok -eq 0 ]]; then
        echo
        print_error "Missing required security tools. Install them and re-run:"
        echo "    cargo install cargo-audit cargo-deny"
        exit 1
    fi

    echo

    # License metadata (Cargo, Fisher, Homebrew, release workflow must agree)
    run_check "License metadata" "./scripts/check-license-metadata.sh" || true

    # Privacy patterns (no workstation paths, private identifiers, or hardcoded sockets)
    run_check "Privacy patterns" "./scripts/check-privacy-patterns.sh" || true

    # Documentation links (every repository-relative Markdown link resolves)
    run_check "Documentation links" "./scripts/check-doc-links.sh" || true

    # Fish checks
    run_check "Fish syntax" "check_fish_syntax" || true
    run_check "Fish formatting" "check_fish_formatting" || true

    # Installer/release script lint (#494)
    run_check "Shellcheck (installers)" "check_shellcheck" || true

    # Shell integration suites (Fish + Bash + Zsh)
    run_shell_tests

    # Rust checks - run in gpy-agent directory
    pushd gpy-agent >/dev/null

    # Run format check
    run_check "Rust format check" "cargo fmt -- --check" || true

    # Run clippy (--features test-support so the #460 PTY test-support code,
    # otherwise invisible under default features, is linted too — see the
    # matching "Rust tests" note below). The lint policy itself (deny
    # warnings, deny clippy::pedantic) lives in gpy-agent/Cargo.toml's
    # [lints] table (#584), not on this command line — a bare `cargo clippy
    # --all-targets --features test-support` run by hand matches this check
    # exactly, no extra flags needed.
    run_check "Clippy" "cargo clippy --all-targets --features test-support" || true

    # Run tests (--features test-support additionally compiles the
    # PTY-backed wizard-terminal-restoration regression test, #460; the
    # feature gates no dependencies, only code visibility, so it's a no-op
    # on any platform other than the unix ones this script targets)
    # --no-fail-fast: report every failure, not just the first (#544).
    # nextest stops the run at the first failing test by default, which on
    # ubuntu meant the gate reported one failure while carrying nine -- the
    # run aborted around test 326 of 1901 and the eight after it were
    # invisible for as long as anyone cares to look back. Each fix then
    # bought one more 7-minute round trip to reveal the next "first"
    # failure. Seeing the whole board costs ~80s, and only when something is
    # already broken. `--fast` mode keeps fail-fast, which is the right
    # trade for local iteration.
    nextest_log="$(mktemp)"
    run_check "Rust tests" "cargo nextest run --features test-support --no-fail-fast 2>&1 | tee \"$nextest_log\"" || true
    report_nextest_retries "$nextest_log"
    rm -f "$nextest_log"

    # Run doc tests
    run_check "Doc tests" "cargo test --doc" || true

    # Security audit (mandatory — tools verified above)
    run_check "Security audit" "cargo audit" || true

    # Dependency/license check (mandatory — tools verified above)
    run_check "Dependency check" "cargo deny check" || true

    # Check for unused dependencies (if cargo-machete is available).
    # --with-metadata resolves bench, example and dev targets through cargo
    # metadata rather than guessing from src/ alone. It is strictly the better
    # check: plain `cargo machete` missed an unused tokio-test dev-dependency
    # that this mode caught (#532). It does not rewrite Cargo.lock despite the
    # tool's general warning -- verified against a checksum before and after.
    if command -v cargo-machete >/dev/null 2>&1; then
        run_check "Unused dependencies" "cargo machete --with-metadata" || true
    else
        print_warning "Skipping unused dependency check (cargo-machete not installed)"
    fi

    # Build release version to catch release-specific issues. Skipped in the
    # default local `just check` because it is the single most expensive leg
    # (~38s on this repo) and shares no artifacts with the debug builds above.
    # Coverage is preserved where it matters: the pre-push gate and CI both run
    # it via `just check-rust` (--rust-only), so release-only breakage is still
    # caught before code leaves the machine. Force it locally with
    # GPY_GATE_RELEASE=1 (or it runs automatically when CI is set).
    if [[ -n "${CI:-}" || -n "${GPY_GATE_RELEASE:-}" ]]; then
        run_check "Release build" "cargo build --release" || true
    else
        print_warning "Skipping release build in local full check (runs in pre-push/CI via --rust-only; set GPY_GATE_RELEASE=1 to force)"
    fi

    popd >/dev/null

    # Performance regression checks (optional, requires jq and bc)
    if [[ "${RUN_PERF_CHECKS:-false}" == "true" ]]; then
        echo
        print_step "Running performance regression checks"

        if command -v jq >/dev/null 2>&1 && command -v bc >/dev/null 2>&1; then
            run_check "Performance baselines" "$PROJECT_ROOT/scripts/perf-baseline.sh --compare" || true
        else
            print_warning "Skipping performance checks (jq and bc required)"
            echo "  Install: sudo apt install jq bc  (or equivalent for your OS)"
        fi
    fi

    # Summary
    echo
    echo "=========================="
    echo -e "${BLUE}Quality Check Summary${NC}"
    echo "=========================="
    echo -e "Checks passed: ${GREEN}$CHECKS_PASSED${NC}"
    echo -e "Checks failed: ${RED}$CHECKS_FAILED${NC}"
    print_skip_tally

    if [ $CHECKS_FAILED -eq 0 ]; then
        echo -e "${GREEN}🎉 All quality checks passed!${NC}"
        echo -e "${BLUE}Tip:${NC} for fast iteration use 'just check-fast' (skips the full shell suites, doc tests, audits, and release build)."
        exit 0
    else
        echo -e "${RED}❌ Some quality checks failed${NC}"
        exit 1
    fi
}

# Show help
show_help() {
    cat << EOF
GPY Code Quality Check Script

Usage: $0 [OPTIONS]

OPTIONS:
    -h, --help          Show this help message
    --fix              Run auto-fix where possible (format, clippy)
    --fast             Skip slower checks (release build, security audits)
    --with-perf        Include performance regression checks (requires jq, bc)
    --rust-only        Run only the Rust toolchain checks (fmt, clippy, tests,
                       doc tests, audit, deny, release build)
    --shell-only       Run only the shell checks (Fish lint + Fish/Bash/Zsh
                       integration suites); skips the Rust toolchain
    --fish-only        Run only Fish lint (syntax + formatting); no test suites

EXAMPLES:
    $0                  Run all quality checks
    $0 --fix           Run checks and apply auto-fixes
    $0 --fast          Run fast checks only
    $0 --with-perf     Run all checks including performance baselines
    $0 --rust-only     Run Rust toolchain checks only
    $0 --shell-only    Run shell (Fish/Bash/Zsh) checks only
    $0 --fish-only     Run Fish lint only

REQUIRED TOOLS:
    - cargo, rustc, clippy, rustfmt (via rustup)
    - fish shell

OPTIONAL TOOLS:
    - fish_indent: for Fish code formatting checks
    - cargo-deny: cargo install cargo-deny
    - cargo-audit: cargo install cargo-audit
    - cargo-machete: cargo install cargo-machete
    - jq, bc: for performance regression checks (--with-perf)
EOF
}

# Handle command line arguments
case "${1:-}" in
    -h|--help)
        show_help
        exit 0
        ;;
    --fix)
        require_pinned_toolchain
        print_step "Running auto-fixes"

        # Format Fish files
        if command -v fish_indent >/dev/null 2>&1; then
            print_step "Formatting Fish files"
            find . -name "*.fish" -type f -not -path "*/target/*" -not -path "*/.git/*" -not -path "*/tests/*" -not -path "*/demo/.fixture-home/*" -print0 | \
                while IFS= read -r -d '' file; do
                    fish_indent -w "$file" 2>/dev/null || print_warning "Could not format $file"
                done
            print_success "Fish files formatted"
        fi

        # Format Rust files
        pushd gpy-agent >/dev/null
        print_step "Formatting Rust files"
        cargo fmt
        print_success "Rust files formatted"

        print_step "Running clippy fixes"
        cargo clippy --fix --allow-dirty --allow-staged --all-targets 2>/dev/null || print_warning "Some clippy fixes require manual intervention"
        print_success "Clippy fixes applied"
        popd >/dev/null

        echo
        echo -e "${GREEN}Auto-fixes applied. Re-running checks...${NC}"
        echo
        main
        ;;
    --fast)
        require_pinned_toolchain
        print_step "Running fast checks only"
        cd "$(dirname "$0")/.."

        run_check "License metadata" "./scripts/check-license-metadata.sh" || true

        # Privacy patterns (no workstation paths, private identifiers, or hardcoded sockets)
        run_check "Privacy patterns" "./scripts/check-privacy-patterns.sh" || true

        # Documentation links (every repository-relative Markdown link resolves)
        run_check "Documentation links" "./scripts/check-doc-links.sh" || true

        # Fish checks
        run_check "Fish syntax" "check_fish_syntax" || true

        run_check "Shellcheck (installers)" "check_shellcheck" || true

        # Bash/Zsh parity tests
        if command -v bash &>/dev/null; then
            run_check "Bash parity tests" "$GPY_BASH tests/bash/parity.test.bash" || true
        fi
        if command -v zsh &>/dev/null; then
            run_check "Zsh parity tests" "zsh tests/zsh/parity.test.zsh" || true
        fi

        # Rust checks. Clippy's lint policy lives in gpy-agent/Cargo.toml's
        # [lints] table (#584) and applies regardless of CLI flags, so
        # --fast is no less strict here than the full run -- the speed
        # difference in this mode comes from skipping the slower non-Rust
        # checks and audits above/below, not from a relaxed clippy pass.
        pushd gpy-agent >/dev/null
        run_check "Rust format check" "cargo fmt -- --check" || true
        run_check "Clippy" "cargo clippy --all-targets --features test-support" || true
        nextest_log="$(mktemp)"
        run_check "Rust tests" "cargo nextest run --features test-support 2>&1 | tee \"$nextest_log\"" || true
        report_nextest_retries "$nextest_log"
        rm -f "$nextest_log"
        popd >/dev/null

        echo
        echo "=========================="
        echo -e "${BLUE}Fast Check Summary${NC}"
        echo "=========================="
        echo -e "Checks passed: ${GREEN}$CHECKS_PASSED${NC}"
        echo -e "Checks failed: ${RED}$CHECKS_FAILED${NC}"
        print_skip_tally

        if [ $CHECKS_FAILED -eq 0 ]; then
            echo -e "${GREEN}🎉 All fast checks passed!${NC}"
            exit 0
        else
            echo -e "${RED}❌ Some fast checks failed${NC}"
            exit 1
        fi
        ;;
    --rust-only)
        require_pinned_toolchain
        print_step "Running Rust checks only"
        cd "$(dirname "$0")/.."

        run_check "License metadata" "./scripts/check-license-metadata.sh" || true

        # Privacy patterns (no workstation paths, private identifiers, or hardcoded sockets)
        run_check "Privacy patterns" "./scripts/check-privacy-patterns.sh" || true

        # Documentation links (every repository-relative Markdown link resolves)
        run_check "Documentation links" "./scripts/check-doc-links.sh" || true

        pushd gpy-agent >/dev/null

        run_check "Rust format check" "cargo fmt -- --check" || true
        run_check "Clippy" "cargo clippy --all-targets --features test-support" || true
        # --no-fail-fast: report every failure, not just the first (#544).
        # nextest stops the run at the first failing test by default, which on
        # ubuntu meant the gate reported one failure while carrying nine -- the
        # run aborted around test 326 of 1901 and the eight after it were
        # invisible for as long as anyone cares to look back. Each fix then
        # bought one more 7-minute round trip to reveal the next "first"
        # failure. Seeing the whole board costs ~80s, and only when something is
        # already broken. `--fast` mode keeps fail-fast, which is the right
        # trade for local iteration.
        nextest_log="$(mktemp)"
        run_check "Rust tests" "cargo nextest run --features test-support --no-fail-fast 2>&1 | tee \"$nextest_log\"" || true
        report_nextest_retries "$nextest_log"
        rm -f "$nextest_log"
        run_check "Doc tests" "cargo test --doc" || true

        # Supply-chain checks (match the full gate). Warn-skip if the tool is
        # absent rather than failing the whole gate.
        if command -v cargo-audit >/dev/null 2>&1; then
            run_check "Security audit" "cargo audit" || true
        else
            print_warning "Skipping security audit (cargo-audit not installed)"
        fi
        if command -v cargo-deny >/dev/null 2>&1; then
            run_check "Dependency check" "cargo deny check" || true
        else
            print_warning "Skipping dependency check (cargo-deny not installed)"
        fi

        run_check "Release build" "cargo build --release" || true

        popd >/dev/null

        echo
        echo "=========================="
        echo -e "${BLUE}Rust Check Summary${NC}"
        echo "=========================="
        echo -e "Checks passed: ${GREEN}$CHECKS_PASSED${NC}"
        echo -e "Checks failed: ${RED}$CHECKS_FAILED${NC}"
        print_skip_tally

        if [ $CHECKS_FAILED -eq 0 ]; then
            echo -e "${GREEN}🎉 All Rust checks passed!${NC}"
            exit 0
        else
            echo -e "${RED}❌ Some Rust checks failed${NC}"
            exit 1
        fi
        ;;
    --shell-only)
        print_step "Running shell checks (Fish + Bash + Zsh, no Rust toolchain)"
        cd "$(dirname "$0")/.."

        run_check "License metadata" "./scripts/check-license-metadata.sh" || true

        # Privacy patterns (no workstation paths, private identifiers, or hardcoded sockets)
        run_check "Privacy patterns" "./scripts/check-privacy-patterns.sh" || true

        # Documentation links (every repository-relative Markdown link resolves)
        run_check "Documentation links" "./scripts/check-doc-links.sh" || true

        run_check "Fish syntax" "check_fish_syntax" || true
        run_check "Fish formatting" "check_fish_formatting" || true
        run_check "Shellcheck (installers)" "check_shellcheck" || true
        run_shell_tests

        echo
        echo "=========================="
        echo -e "${BLUE}Shell Check Summary${NC}"
        echo "=========================="
        echo -e "Checks passed: ${GREEN}$CHECKS_PASSED${NC}"
        echo -e "Checks failed: ${RED}$CHECKS_FAILED${NC}"
        print_skip_tally

        if [ $CHECKS_FAILED -eq 0 ]; then
            echo -e "${GREEN}🎉 All shell checks passed!${NC}"
            exit 0
        else
            echo -e "${RED}❌ Some shell checks failed${NC}"
            exit 1
        fi
        ;;
    --fish-only)
        print_step "Running Fish lint only (syntax + formatting)"
        cd "$(dirname "$0")/.."

        run_check "Fish syntax" "check_fish_syntax" || true
        run_check "Fish formatting" "check_fish_formatting" || true

        echo
        echo "=========================="
        echo -e "${BLUE}Fish Check Summary${NC}"
        echo "=========================="
        echo -e "Checks passed: ${GREEN}$CHECKS_PASSED${NC}"
        echo -e "Checks failed: ${RED}$CHECKS_FAILED${NC}"
        print_skip_tally

        if [ $CHECKS_FAILED -eq 0 ]; then
            echo -e "${GREEN}🎉 All Fish checks passed!${NC}"
            exit 0
        else
            echo -e "${RED}❌ Some Fish checks failed${NC}"
            exit 1
        fi
        ;;
    --with-perf)
        export RUN_PERF_CHECKS=true
        main
        ;;
    "")
        main
        ;;
    *)
        echo "Unknown option: $1"
        echo "Use --help for usage information"
        exit 1
        ;;
esac
