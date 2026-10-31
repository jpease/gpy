#!/usr/bin/env bash
# benchmarks/compare-starship/run.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Reproducible GPY vs Starship comparison of one git-status prompt render.
# Method, fairness rules and how to read the result: see README.md.
#
# Works on macOS's stock Bash 3.2 as well as Bash 4+/5.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

GPY_REPO="jpease/gpy"
STARSHIP_REPO="starship/starship"

# A verdict other than "inconclusive" needs at least this many rounds.
MIN_ROUNDS=5
# Contention monitor (README.md, "Apples to apples"): a timed block is re-run
# when system CPU busy exceeds what the tools themselves used (at least 1.5
# CPUs) by more than this many percentage points.
BUSY_MARGIN=10
MAX_BLOCK_RETRIES=3
# A block is also re-run if swap use grew by more than this (MB).
MAX_SWAP_GROWTH_MB=64

usage() {
    cat <<'EOF'
Usage: run.sh [COMMAND] [OPTIONS]

Commands:
  run        Prepare fixtures and tools, then benchmark (default).
  fixtures   Fetch and verify the pinned benchmark repositories, then stop.
  tools      Resolve and print the GPY and Starship binaries, then stop.

Tools under test:
  --gpy SPEC         pinned (default) | latest | vX.Y.Z | local | ref:REV | PATH
                       pinned   release tag from versions.lock
                       latest   newest GitHub release
                       vX.Y.Z   a specific release tag
                       local    cargo build --release of this working tree
                       ref:REV  cargo build --release of a commit (git archive)
                       PATH     an existing gpy-agent binary
  --starship SPEC    pinned (default) | latest | X.Y.Z | system | PATH
                       pinned   release from versions.lock
                       latest   newest GitHub release
                       X.Y.Z    a specific release
                       system   the starship found on PATH
                       PATH     an existing starship binary

Workload:
  --repos A,B        Only these fixtures (names from repos.lock). Default: all.
  --scenarios A,B    clean, dirty, or both (default: clean,dirty).
  --rounds N         Alternating-order rounds per cell (default 6).
  --runs N           Timed runs per tool per round (default 100).
  --warmup N         Untimed runs before each block (default 5).
  --untracked-cache M  How the fixtures' core.untrackedCache is set: unset (default;
                       key removed, as on a typical machine: the fixture primes
                       the cache, as your own git commands would; GPY's one-shot
                       path does not add it, the agent does; Starship never
                       uses it), on (pinned true, the same for both tools) or
                       off (pinned false).

Environment:
  --home DIR         Cache for fixtures and tools (default: $GPY_BENCH_HOME or
                     ${XDG_CACHE_HOME:-~/.cache}/gpy-bench).
  --out DIR          Result directory (default: bench-results/compare-starship/<UTC time>).
  --force            Run even if the machine does not look quiet.
  -h, --help         Show this help.

Examples:
  run.sh                                  # pinned GPY + pinned Starship, full matrix
  run.sh --gpy latest --starship latest   # newest releases of both
  run.sh --gpy local                      # this working tree vs pinned Starship
  run.sh --gpy ref:HEAD~5 --repos tiny-express --rounds 5 --runs 20   # quick check
EOF
}

log() { printf '%s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required ($2)"; }

COMMAND=run
GPY_SPEC=pinned
STARSHIP_SPEC=pinned
REPO_FILTER=""
SCENARIOS="clean,dirty"
ROUNDS=6
RUNS=100
WARMUP=5
UNTRACKED_CACHE="unset"
FORCE=0
OUT_DIR=""
BENCH_HOME="${GPY_BENCH_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/gpy-bench}"

parse_args() {
    case "${1:-}" in
        run | fixtures | tools) COMMAND=$1; shift ;;
    esac
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --gpy) GPY_SPEC=${2:?--gpy needs a value}; shift 2 ;;
            --starship) STARSHIP_SPEC=${2:?--starship needs a value}; shift 2 ;;
            --repos) REPO_FILTER=${2:?--repos needs a value}; shift 2 ;;
            --scenarios) SCENARIOS=${2:?--scenarios needs a value}; shift 2 ;;
            --rounds) ROUNDS=${2:?--rounds needs a value}; shift 2 ;;
            --runs) RUNS=${2:?--runs needs a value}; shift 2 ;;
            --warmup) WARMUP=${2:?--warmup needs a value}; shift 2 ;;
            --untracked-cache) UNTRACKED_CACHE=${2:?--untracked-cache needs a value}; shift 2 ;;
            --home) BENCH_HOME=${2:?--home needs a value}; shift 2 ;;
            --out) OUT_DIR=${2:?--out needs a value}; shift 2 ;;
            --force) FORCE=1; shift ;;
            -h | --help) usage; exit 0 ;;
            *) usage >&2; die "unknown argument: $1" ;;
        esac
    done
    local v
    case "$UNTRACKED_CACHE" in off | on | unset) ;; *) die "--untracked-cache must be off, on or unset" ;; esac
    for v in ROUNDS RUNS WARMUP; do
        [[ "${!v}" =~ ^[0-9]+$ ]] || die "--$(printf '%s' "$v" | tr '[:upper:]' '[:lower:]') must be a non-negative integer"
    done
    [[ $ROUNDS -ge 1 && $RUNS -ge 2 ]] || die "--rounds must be >= 1 and --runs >= 2"
    local s
    IFS=, read -r -a SCENARIO_LIST <<<"$SCENARIOS"
    for s in "${SCENARIO_LIST[@]}"; do
        [[ "$s" == clean || "$s" == dirty ]] || die "unknown scenario '$s' (clean or dirty)"
    done
}

lock_get() { sed -n "s/^$1=//p" "$HERE/versions.lock"; }

detect_platform() {
    case "$(uname -s)" in
        Darwin) OS=macos; STAR_OS=apple-darwin ;;
        Linux) OS=linux; STAR_OS=unknown-linux-musl ;;
        *) die "unsupported OS $(uname -s): GPY's IPC and this harness are macOS/Linux only" ;;
    esac
    case "$(uname -m)" in
        arm64 | aarch64) ARCH=aarch64 ;;
        x86_64 | amd64) ARCH=x86_64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac
    GPY_ASSET="gpy-agent-$OS-$ARCH"
    STARSHIP_ASSET="starship-$ARCH-$STAR_OS.tar.gz"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Newest release tag of OWNER/REPO. The public redirect needs no API token and
# has no rate limit; `gh` covers a private repository.
latest_tag() {
    local repo=$1 url tag=""
    url=$(curl -fsSI -o /dev/null -w '%{redirect_url}' "https://github.com/$repo/releases/latest" 2>/dev/null) || url=""
    tag=${url##*/}
    if [[ -z "$tag" || "$tag" == latest ]] && command -v gh >/dev/null 2>&1; then
        tag=$(gh release view -R "$repo" --json tagName -q .tagName 2>/dev/null) || tag=""
    fi
    [[ -n "$tag" && "$tag" != latest ]] || die "cannot resolve the latest release of $repo (offline, or private without gh auth)"
    printf '%s' "$tag"
}

# fetch_release_asset REPO TAG ASSET DIR: ASSET and ASSET.sha256 into DIR.
fetch_release_asset() {
    local repo=$1 tag=$2 asset=$3 dest=$4 base
    base="https://github.com/$repo/releases/download/$tag"
    rm -f "$dest/$asset" "$dest/$asset.sha256"
    if curl -fsSL --retry 3 --connect-timeout 15 -o "$dest/$asset" "$base/$asset" 2>/dev/null &&
        curl -fsSL --retry 3 --connect-timeout 15 -o "$dest/$asset.sha256" "$base/$asset.sha256" 2>/dev/null; then
        return 0
    fi
    command -v gh >/dev/null 2>&1 ||
        die "cannot download $asset from $repo $tag (not public yet? install and authenticate gh)"
    rm -f "$dest/$asset" "$dest/$asset.sha256"
    gh release download "$tag" -R "$repo" -p "$asset" -p "$asset.sha256" -D "$dest" >/dev/null ||
        die "cannot download $asset from $repo $tag"
}

# verify_sha256 FILE SUMFILE: SUMFILE holds "<hash>" or "<hash>  <name>".
verify_sha256() {
    local want got
    want=$(cut -d' ' -f1 <"$2" | tr -d '[:space:]')
    got=$(sha256_of "$1")
    [[ -n "$want" && "$want" == "$got" ]] || die "sha256 mismatch for $(basename "$1"): expected '$want', got '$got'"
}

# Each resolver sets SRC_GPY / DESC_GPY (resp. SRC_STARSHIP / DESC_STARSHIP).
gpy_from_release() {
    local tag=$1 dir tmp
    dir="$BENCH_HOME/tools/gpy-$tag-$OS-$ARCH"
    if [[ ! -x "$dir/gpy-agent" ]]; then
        mkdir -p "$BENCH_HOME/tools"
        tmp=$(mktemp -d "$BENCH_HOME/tools/.dl.XXXXXX")
        log "downloading GPY $tag ($GPY_ASSET)"
        fetch_release_asset "$GPY_REPO" "$tag" "$GPY_ASSET" "$tmp"
        verify_sha256 "$tmp/$GPY_ASSET" "$tmp/$GPY_ASSET.sha256"
        mkdir -p "$dir"
        mv "$tmp/$GPY_ASSET" "$dir/gpy-agent"
        chmod 755 "$dir/gpy-agent"
        rm -rf "$tmp"
    fi
    SRC_GPY="$dir/gpy-agent"
    DESC_GPY="release $tag"
}

gpy_build() { # SRC_DIR TARGET_DIR(optional)
    log "building gpy-agent (cargo build --release --locked) in $1"
    if [[ -n "${2:-}" ]]; then
        (cd "$1/gpy-agent" && CARGO_TARGET_DIR="$2" cargo build --release --locked --quiet)
    else
        (cd "$1/gpy-agent" && cargo build --release --locked --quiet)
    fi
}

resolve_gpy() {
    local spec=$1 tag rev sha src target
    case "$spec" in
        pinned)
            tag=$(lock_get GPY_PINNED)
            [[ -n "$tag" ]] || die "GPY_PINNED missing from versions.lock"
            gpy_from_release "$tag" ;;
        latest) gpy_from_release "$(latest_tag "$GPY_REPO")" ;;
        v[0-9]*) gpy_from_release "$spec" ;;
        local)
            need cargo "needed to build --gpy local"
            gpy_build "$ROOT"
            SRC_GPY="${CARGO_TARGET_DIR:-$ROOT/gpy-agent/target}/release/gpy-agent"
            DESC_GPY="local build of $(git -C "$ROOT" describe --always --dirty --abbrev=12 2>/dev/null || echo unknown)" ;;
        ref:*)
            need cargo "needed to build --gpy ref:REV"
            rev=${spec#ref:}
            sha=$(git -C "$ROOT" rev-parse --verify --quiet "$rev^{commit}") || die "not a commit in this repository: $rev"
            src="$BENCH_HOME/build/src-$sha"
            target="$BENCH_HOME/build/target"
            if [[ ! -d "$src" ]]; then
                mkdir -p "$src.tmp"
                git -C "$ROOT" archive "$sha" | tar -x -C "$src.tmp"
                mv "$src.tmp" "$src"
            fi
            gpy_build "$src" "$target"
            SRC_GPY="$target/release/gpy-agent"
            DESC_GPY="build of commit $sha ($rev)" ;;
        */* | .*)
            [[ -x "$spec" ]] || die "not an executable file: $spec"
            SRC_GPY=$spec
            DESC_GPY="binary $spec" ;;
        *) die "unrecognised --gpy value '$spec' (see --help)" ;;
    esac
    [[ -x "$SRC_GPY" ]] || die "expected gpy-agent at $SRC_GPY"
}

starship_from_release() {
    local ver=${1#v} dir tmp
    dir="$BENCH_HOME/tools/starship-$ver-$STAR_OS-$ARCH"
    if [[ ! -x "$dir/starship" ]]; then
        mkdir -p "$BENCH_HOME/tools"
        tmp=$(mktemp -d "$BENCH_HOME/tools/.dl.XXXXXX")
        log "downloading Starship v$ver ($STARSHIP_ASSET)"
        fetch_release_asset "$STARSHIP_REPO" "v$ver" "$STARSHIP_ASSET" "$tmp"
        verify_sha256 "$tmp/$STARSHIP_ASSET" "$tmp/$STARSHIP_ASSET.sha256"
        mkdir -p "$dir"
        tar -xzf "$tmp/$STARSHIP_ASSET" -C "$dir" starship
        rm -rf "$tmp"
    fi
    SRC_STARSHIP="$dir/starship"
    DESC_STARSHIP="release v$ver"
}

resolve_starship() {
    local spec=$1 ver
    case "$spec" in
        pinned)
            ver=$(lock_get STARSHIP_PINNED)
            [[ -n "$ver" ]] || die "STARSHIP_PINNED missing from versions.lock"
            starship_from_release "$ver" ;;
        latest) starship_from_release "$(latest_tag "$STARSHIP_REPO")" ;;
        [0-9]* | v[0-9]*) starship_from_release "$spec" ;;
        system)
            SRC_STARSHIP=$(command -v starship) || die "no starship on PATH"
            DESC_STARSHIP="system $SRC_STARSHIP" ;;
        */* | .*)
            [[ -x "$spec" ]] || die "not an executable file: $spec"
            SRC_STARSHIP=$spec
            DESC_STARSHIP="binary $spec" ;;
        *) die "unrecognised --starship value '$spec' (see --help)" ;;
    esac
    [[ -x "$SRC_STARSHIP" ]] || die "expected starship at $SRC_STARSHIP"
}

# Copy both binaries into one directory on the cache volume. A working tree
# on an external or network volume would otherwise load one tool from slower
# storage than the other.
stage_binaries() {
    RUN_BIN="$BENCH_HOME/run-bin"
    mkdir -p "$RUN_BIN"
    install -m 755 "$SRC_GPY" "$RUN_BIN/gpy-agent"
    install -m 755 "$SRC_STARSHIP" "$RUN_BIN/starship"
    GPY_BIN="$RUN_BIN/gpy-agent"
    STARSHIP_BIN="$RUN_BIN/starship"
}

first_line() { local l; read -r l < <("$@" 2>&1) || true; printf '%s' "$l"; }

REPO_NAMES=()
REPO_URLS=()
REPO_SHAS=()
REPO_REFS=()

load_repos() {
    local name url sha ref filter_hit f
    while read -r name url sha ref _; do
        [[ -z "$name" || "$name" == \#* ]] && continue
        [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || die "repos.lock: '$name' commit is not a 40-hex SHA: $sha"
        if [[ -n "$REPO_FILTER" ]]; then
            filter_hit=0
            IFS=, read -r -a wanted <<<"$REPO_FILTER"
            for f in "${wanted[@]}"; do [[ "$f" == "$name" ]] && filter_hit=1; done
            [[ $filter_hit -eq 1 ]] || continue
        fi
        REPO_NAMES+=("$name"); REPO_URLS+=("$url"); REPO_SHAS+=("$sha"); REPO_REFS+=("${ref:-}")
    done <"$HERE/repos.lock"
    [[ ${#REPO_NAMES[@]} -gt 0 ]] || die "no fixtures selected (check --repos against repos.lock)"
    if [[ -n "$REPO_FILTER" ]]; then
        IFS=, read -r -a wanted <<<"$REPO_FILTER"
        for f in "${wanted[@]}"; do
            filter_hit=0
            for name in "${REPO_NAMES[@]}"; do [[ "$f" == "$name" ]] && filter_hit=1; done
            [[ $filter_hit -eq 1 ]] || die "--repos: '$f' is not in repos.lock"
        done
    fi
}

repo_dir() { printf '%s/repos/%s' "$BENCH_HOME" "$1"; }

# Fetch exactly the pinned commit (depth 1) and leave the work tree on a local
# branch named "bench", clean, with the index refreshed.
prepare_fixture() {
    local name=$1 url=$2 sha=$3 dir head
    dir=$(repo_dir "$name")
    mkdir -p "$dir"
    if [[ ! -d "$dir/.git" ]]; then
        git -C "$dir" init -q
        git -C "$dir" remote add origin "$url"
    fi
    head=$(git -C "$dir" rev-parse --verify --quiet HEAD 2>/dev/null || true)
    if [[ "$head" != "$sha" ]]; then
        log "fetching $name @ ${sha:0:12} (this can take a while for the large repositories)"
        git -C "$dir" fetch -q --depth 1 origin "$sha" ||
            die "cannot fetch $sha from $url"
    fi
    git -C "$dir" checkout -q -f -B bench "$sha"
    head=$(git -C "$dir" rev-parse HEAD)
    [[ "$head" == "$sha" ]] || die "$name: HEAD is $head, expected $sha"
    git -C "$dir" reset -q --hard
    git -C "$dir" clean -q -fdx
    # Pin settings that change `git status` behaviour; the reader's global config must not matter.
    git -C "$dir" config core.fsmonitor false
    case "$UNTRACKED_CACHE" in
        off) git -C "$dir" config core.untrackedCache false; git -C "$dir" update-index --no-untracked-cache ;;
        on | unset)
            # Prime the cache with an index-writing `git status`, as a user's
            # would (GPY runs with GIT_OPTIONAL_LOCKS=0). Must run isolated: git
            # stamps the cache with the exclude files it saw.
            git -C "$dir" config core.untrackedCache true
            git -C "$dir" update-index --untracked-cache
            sleep 2
            git -C "$dir" status --porcelain >/dev/null
            [[ "$UNTRACKED_CACHE" == on ]] || git -C "$dir" config --unset core.untrackedCache
            ;;
    esac
    git -C "$dir" config gc.auto 0
    git -C "$dir" config maintenance.auto false
    git -C "$dir" status --porcelain >/dev/null
    [[ -z "$(git -C "$dir" status --porcelain)" ]] ||
        die "$name is not clean straight after checkout; on a case-insensitive filesystem this means two tracked paths differ only by case. Use --home on a case-sensitive volume."
}

prepare_all_fixtures() {
    local i
    for i in "${!REPO_NAMES[@]}"; do
        prepare_fixture "${REPO_NAMES[$i]}" "${REPO_URLS[$i]}" "${REPO_SHAS[$i]}"
    done
}

DIRTY_DIR=""

# Deterministic dirty state: one unstaged edit, one staged edit, one untracked
# file. Only regular, non-symlink tracked files are touched.
apply_dirty() {
    local dir=$1 f unstaged="" staged=""
    while IFS= read -r f; do
        [[ -f "$dir/$f" && ! -L "$dir/$f" ]] || continue
        if [[ -z "$unstaged" ]]; then unstaged=$f; else staged=$f; break; fi
    done < <(git -C "$dir" ls-files)
    [[ -n "$unstaged" && -n "$staged" ]] || die "$dir has fewer than two regular tracked files"
    DIRTY_DIR=$dir
    printf '\n# gpy-bench\n' >>"$dir/$unstaged"
    printf '\n# gpy-bench\n' >>"$dir/$staged"
    git -C "$dir" add -- "$staged"
    printf 'gpy-bench\n' >"$dir/gpy-bench-untracked.txt"
    settle_index "$dir"
}

revert_dirty() {
    [[ -n "$DIRTY_DIR" ]] || return 0
    git -C "$DIRTY_DIR" reset -q --hard
    git -C "$DIRTY_DIR" clean -q -fd
    settle_index "$DIRTY_DIR"
    DIRTY_DIR=""
}

# Git re-hashes any file whose mtime is not older than the index ("racily
# clean"), on every status, until the index is rewritten. Wait out the mtime
# granularity, then rewrite the index so neither tool pays for that.
settle_index() {
    sleep 2
    git -C "$1" status --porcelain >/dev/null
}

strip_ansi() {
    local esc
    esc=$(printf '\033')
    sed "s/${esc}\\[[0-9;]*[A-Za-z]//g"
}

# Fail before timing if either tool's output is wrong: a tool that errors out
# instantly is the classic way a benchmark reports a fantastic number.
preflight() { # DIR SCENARIO
    local dir=$1 scenario=$2 out
    out=$("$GPY_BIN" oneshot git --cwd "$dir" --format json) || die "gpy-agent failed on $dir"
    if [[ "$scenario" == clean ]]; then
        printf '%s' "$out" | jq -e '.branch == "bench" and .staged == 0 and .unstaged == 0 and .untracked == 0 and .conflicts == 0' >/dev/null ||
            die "gpy-agent: unexpected clean-state output on $dir: $out"
    else
        printf '%s' "$out" | jq -e '.branch == "bench" and .staged >= 1 and .unstaged >= 1 and .untracked >= 1' >/dev/null ||
            die "gpy-agent: unexpected dirty-state output on $dir: $out"
    fi
    out=$("$STARSHIP_BIN" prompt --path "$dir" | strip_ansi) || die "starship failed on $dir"
    [[ "$out" == *bench* ]] || die "starship: branch missing from output on $dir: $out"
    if [[ "$scenario" == clean ]]; then
        [[ "$out" != *[+!?]* ]] || die "starship: reports changes on a clean tree $dir: $out"
    else
        [[ "$out" == *+* && "$out" == *!* && "$out" == *\?* ]] ||
            die "starship: missing staged/modified/untracked markers on $dir: $out"
    fi
}

# Sanitise everything that could make the same command behave differently on
# two machines. Call after tools are built: cargo needs the real HOME.
isolate_env() {
    local v
    for v in $(compgen -e); do
        case "$v" in
            GPY_* | STARSHIP_* | GIT_DIR | GIT_WORK_TREE | GIT_INDEX_FILE | NO_COLOR | CLICOLOR_FORCE) unset "$v" ;;
        esac
    done
    mkdir -p "$BENCH_HOME/home"
    export HOME="$BENCH_HOME/home"
    export XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache" XDG_DATA_HOME="$HOME/.local/share"
    export STARSHIP_CONFIG="$HERE/starship.toml"
    export STARSHIP_CACHE="$HOME/starship-cache"
    # Starship disables itself under TERM=dumb.
    export TERM=xterm-256color
    export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_TERMINAL_PROMPT=0
}

ncpu() { getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1; }

load1() {
    local l
    l=$(uptime | sed 's/.*load averages\{0,1\}: *//' | cut -d' ' -f1 | tr -d ',')
    printf '%s' "${l:-0}"
}

cpu_model() {
    if [[ "$OS" == macos ]]; then
        sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m
    else
        sed -n '/^model name/{s/.*: *//p;q;}' /proc/cpuinfo 2>/dev/null || uname -m
    fi
}

power_source() {
    local f
    if [[ "$OS" == macos ]]; then
        if pmset -g batt 2>/dev/null | grep -q 'Battery Power'; then echo battery; else echo ac; fi
    else
        for f in /sys/class/power_supply/BAT*/status; do
            [[ -r "$f" ]] || continue
            if [[ "$(cat "$f")" == Discharging ]]; then echo battery; return; fi
        done
        echo ac
    fi
}

SAMPLER_PID=""
SAMPLES=""
SAMPLER_DESC=""
BLOCKS_FILE=""
BUSY_FLOOR=""

# Background sampler: one "<epoch-seconds> <CPU busy %>" line per second in
# $SAMPLES. A single long-lived process (top used ~1/5 core), so its cost is the
# same during both tools' blocks.
start_sampler() {
    SAMPLES="$OUT_DIR/cpu-samples.txt"
    : >"$SAMPLES"
    if [[ "$OS" == macos ]]; then
        SAMPLER_DESC="iostat -n 0 -w 1, once per second"
        (
            skip_first=1
            iostat -n 0 -w 1 2>/dev/null | while read -r us sy id _; do
                [[ "$us" =~ ^[0-9]+$ && "$sy" =~ ^[0-9]+$ && "$id" =~ ^[0-9]+$ ]] || continue
                # The first iostat sample is the average since boot, not "now".
                if ((skip_first)); then skip_first=0; continue; fi
                printf '%s %d\n' "$(date +%s)" $((100 - id)) >>"$SAMPLES"
            done
        ) &
    else
        SAMPLER_DESC="/proc/stat deltas, once per second"
        (
            prev_total=0
            prev_idle=0
            while :; do
                read -r _ u n s i w irq sirq st _ </proc/stat
                total=$((u + n + s + i + w + irq + sirq + st))
                idle=$((i + w))
                if ((prev_total > 0 && total > prev_total)); then
                    busy_c=$((10000 * ((total - prev_total) - (idle - prev_idle)) / (total - prev_total)))
                    printf '%s %d.%02d\n' "$(date +%s)" $((busy_c / 100)) $((busy_c % 100)) >>"$SAMPLES"
                fi
                prev_total=$total
                prev_idle=$idle
                sleep 1
            done
        ) &
    fi
    SAMPLER_PID=$!
}

stop_sampler() {
    [[ -n "$SAMPLER_PID" ]] || return 0
    pkill -P "$SAMPLER_PID" 2>/dev/null || true
    kill "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
}

# window_busy FROM TO: "<mean> <max> <samples>" of the busy samples stamped in
# [FROM, TO] (epoch seconds).
window_busy() {
    awk -v s="$1" -v e="$2" '
        $1 >= s && $1 <= e { sum += $2; n++; if ($2 > max) max = $2 }
        END { if (n) printf "%.2f %.2f %d", sum / n, max, n; else print "0 0 0" }' "$SAMPLES"
}

gt() { awk -v a="$1" -v b="$2" 'BEGIN { exit !(a > b) }'; }

swap_used_mb() {
    if [[ "$OS" == macos ]]; then
        sysctl -n vm.swapusage 2>/dev/null | sed -n 's/.*used = \([0-9.]*\)M.*/\1/p' | grep . || echo 0
    else
        awk '/^SwapTotal:/ { t = $2 } /^SwapFree:/ { f = $2 } END { printf "%.2f", (t - f) / 1024 }' /proc/meminfo 2>/dev/null || echo 0
    fi
}

# macOS reports a CPU_Speed_Limit below 100 while the CPU is thermally
# throttled; nothing is printed otherwise. Empty on Linux.
thermal_limit() {
    [[ "$OS" == macos ]] || return 0
    pmset -g therm 2>/dev/null | sed -n 's/.*CPU_Speed_Limit *= *//p' | grep . || true
}

heaviest_processes() {
    ps -A -o pcpu=,comm= 2>/dev/null | sort -rn | sed -n '1,3p' |
        sed -E 's|^ *([0-9.]+) .*/([^/]+)$|\1% \2|; s|^ *([0-9.]+) |\1% |' |
        tr '\n' ',' | cut -c1-120 || true
}

# own_busy FILE: "MEAN PEAK" percent of all CPUs the timed tools used, from
# hyperfine's accounting (children, i.e. GPY's git, included). MEAN covers both
# tools, warmup runs added since hyperfine doesn't export them; PEAK is the
# busier tool, since they run sequentially. Reads v1 and v2 export schemas.
own_busy() {
    jq -r --argjson cpus "$(ncpu)" --argjson warm "$WARMUP" '
        [.results[]
         | if has("summary")
           then {n: (.summary.time_cpu.count + $warm), cpu: .summary.time_cpu.mean, wall: .summary.time_wall_clock.mean}
           else {n: ((.times | length) + $warm), cpu: (.user + .system), wall: .mean } end
         | {cpu_s: (.cpu * .n), wall_s: (.wall * .n), rate: (if .wall > 0 then 100 * .cpu / ($cpus * .wall) else 0 end)}] as $r
        | (($r | map(.wall_s) | add)) as $wall_s
        | (if $wall_s > 0 then 100 * (($r | map(.cpu_s) | add)) / ($cpus * $wall_s) else 0 end) as $mean
        | "\($mean * 100 | round / 100) \(($r | map(.rate) | max) * 100 | round / 100)"' "$1" 2>/dev/null || echo "0 0"
}

# contention_reason MEAN MAX SAMPLES SWAP_BEFORE SWAP_AFTER THERMAL LIMIT PEAK_LIMIT:
# empty if the machine was quiet enough, else why not.
contention_reason() {
    local reason=""
    if (($3 >= 2)); then
        if gt "$1" "$7"; then reason="mean CPU busy $1% exceeds $7%; "; fi
        if gt "$2" "$8"; then reason="${reason}peak CPU busy $2% exceeds $8%; "; fi
    fi
    if gt "$(awk -v a="$5" -v b="$4" 'BEGIN { print a - b }')" "$MAX_SWAP_GROWTH_MB"; then reason="${reason}swap use grew by more than ${MAX_SWAP_GROWTH_MB} MB; "; fi
    if [[ -n "$6" && "$6" -lt 100 ]]; then reason="${reason}CPU thermally throttled to $6%; "; fi
    printf '%s' "$reason"
}

# Wait for a few idle seconds; die unless --force. Not the 1-minute load
# average: it still reflects the fixture clone or cargo build just before.
quiet_machine_gate() {
    local problems="" deadline=$((SECONDS + 60)) now mean max n
    log "checking that the machine is quiet (up to 60 s)"
    sleep 6
    while :; do
        now=$(date +%s)
        read -r mean max n <<<"$(window_busy $((now - 5)) "$now")"
        if ((n >= 4)) && ! gt "$mean" "$BUSY_MARGIN" && ! gt "$max" "$((BUSY_MARGIN + 20))"; then
            break
        fi
        if ((SECONDS >= deadline)); then
            problems="${problems}  - system CPU busy: mean ${mean}%, peak ${max}% over the last 5 s (allowed: mean ${BUSY_MARGIN}%). Heaviest: $(heaviest_processes)\n"
            break
        fi
        sleep 3
    done
    if [[ "$(power_source)" == battery ]]; then
        problems="${problems}  - running on battery power; CPU frequency policy differs from mains\n"
    fi
    if [[ -n "$problems" ]]; then
        printf 'The machine does not look quiet:\n%b' "$problems" >&2
        if [[ $FORCE -eq 1 ]]; then
            log "continuing because of --force; blocks that run under load are re-run, then marked in the report"
        else
            die "close other work (or pass --force), then rerun"
        fi
    fi
}

q() { printf '%q' "$1"; }

cmd_gpy() { printf '%s oneshot git --cwd %s --format json' "$(q "$GPY_BIN")" "$(q "$1")"; }
cmd_starship() { printf '%s prompt --path %s' "$(q "$STARSHIP_BIN")" "$(q "$1")"; }

# One timed block per round. A block run under load is re-run up to
# MAX_BLOCK_RETRIES times, then kept but flagged (the cell becomes inconclusive).
run_cell() { # REPO SCENARIO DIR RAW_DIR
    local repo=$1 scenario=$2 dir=$3 raw=$4
    local r first second out attempt t t0 t1 mean max n own own_peak limit peak_limit swap0 swap1 therm reason procs accepted contended
    local args
    for ((r = 1; r <= ROUNDS; r++)); do
        # Alternate order so drift cannot systematically favour one side.
        if ((r % 2 == 1)); then first=gpy second=starship; else first=starship second=gpy; fi
        out=$(printf '%s/%s.%s.r%02d.json' "$raw" "$repo" "$scenario" "$r")
        attempt=0
        while :; do
            attempt=$((attempt + 1))
            log "  round $r/$ROUNDS ($first first), attempt $attempt"
            args=(-N --style none --warmup "$WARMUP" --runs "$RUNS" --export-json "$out")
            for t in "$first" "$second"; do
                if [[ "$t" == gpy ]]; then
                    args+=(--command-name gpy "$(cmd_gpy "$dir")")
                else
                    args+=(--command-name starship "$(cmd_starship "$dir")")
                fi
            done
            swap0=$(swap_used_mb)
            t0=$(date +%s)
            # Warnings go to a log; a failure shows its tail.
            hyperfine "${args[@]}" >/dev/null 2>>"$OUT_DIR/hyperfine.log" ||
                { tail -n 20 "$OUT_DIR/hyperfine.log" >&2; die "hyperfine failed on $repo ($scenario)"; }
            t1=$(date +%s)
            swap1=$(swap_used_mb)
            therm=$(thermal_limit)
            read -r mean max n <<<"$(window_busy $((t0 + 1)) "$t1")"
            read -r own own_peak <<<"$(own_busy "$out")"
            limit=$(awk -v o="$own" -v f="$BUSY_FLOOR" -v m="$BUSY_MARGIN" 'BEGIN { if (o < f) o = f; printf "%.1f", o + m }')
            peak_limit=$(awk -v o="$own_peak" -v f="$BUSY_FLOOR" -v m="$BUSY_MARGIN" 'BEGIN { if (o < f) o = f; printf "%.1f", o + m + 20 }')
            reason=$(contention_reason "$mean" "$max" "$n" "$swap0" "$swap1" "${therm:-}" "$limit" "$peak_limit")
            procs=""
            accepted=true
            contended=false
            if [[ -n "$reason" ]]; then
                procs=$(heaviest_processes)
                if ((attempt <= MAX_BLOCK_RETRIES)); then
                    accepted=false
                else
                    contended=true
                fi
            fi
            jq -n -c --arg repo "$repo" --arg scenario "$scenario" --argjson round "$r" --argjson attempt "$attempt" \
                --argjson start "$t0" --argjson end "$t1" --argjson samples "$n" \
                --argjson mean_busy "$mean" --argjson max_busy "$max" --argjson own_busy "$own" --argjson limit "$limit" --argjson peak_limit "$peak_limit" \
                --argjson swap_growth_mb "$(awk -v a="$swap1" -v b="$swap0" 'BEGIN { print a - b }')" \
                --arg thermal_limit "${therm:-}" --arg reason "$reason" --arg heaviest "$procs" \
                --argjson accepted "$accepted" --argjson contended "$contended" \
                '{repo: $repo, scenario: $scenario, round: $round, attempt: $attempt, start: $start, end: $end,
                  samples: $samples, mean_busy: $mean_busy, max_busy: $max_busy, own_busy: $own_busy, limit: $limit, peak_limit: $peak_limit, swap_growth_mb: $swap_growth_mb,
                  thermal_limit: $thermal_limit, reason: $reason, heaviest: $heaviest,
                  accepted: $accepted, contended: $contended}' >>"$BLOCKS_FILE"
            [[ "$accepted" == true ]] && break
            log "    discarded, machine not quiet: $reason"
            log "    heaviest processes: $procs"
            mv "$out" "$raw/contended/$(basename "$out" .json).attempt$attempt.json"
            sleep 5
        done
    done
}

CAFFEINATE_PID=""
LOCK_DIR=""


cleanup() {
    revert_dirty || true
    [[ -z "$CAFFEINATE_PID" ]] || kill "$CAFFEINATE_PID" 2>/dev/null || true
    stop_sampler
    [[ -z "$LOCK_DIR" ]] || rmdir "$LOCK_DIR" 2>/dev/null || true
}

acquire_lock() {
    mkdir -p "$BENCH_HOME"
    LOCK_DIR="$BENCH_HOME/run.lock"
    if ! mkdir "$LOCK_DIR" 2>/dev/null; then
        LOCK_DIR=""
        die "another run is using $BENCH_HOME (if none is, remove $BENCH_HOME/run.lock)"
    fi
}

main() {
    parse_args "$@"
    need git "fixtures"
    need curl "downloads"
    detect_platform
    [[ "$BENCH_HOME" == /* ]] || BENCH_HOME="$PWD/$BENCH_HOME"
    export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_TERMINAL_PROMPT=0

    trap cleanup EXIT
    trap 'exit 130' INT TERM
    acquire_lock
    load_repos

    if [[ "$COMMAND" == fixtures ]]; then
        prepare_all_fixtures
        log "fixtures ready in $BENCH_HOME/repos"
        return 0
    fi

    need jq "result processing"
    need tar "unpacking Starship"
    resolve_gpy "$GPY_SPEC"
    resolve_starship "$STARSHIP_SPEC"
    stage_binaries
    if [[ "$COMMAND" == tools ]]; then
        printf 'gpy-agent: %s\n  %s\n  %s\n' "$DESC_GPY" "$(first_line "$GPY_BIN" --version)" "$(sha256_of "$GPY_BIN")"
        printf 'starship:  %s\n  %s\n  %s\n' "$DESC_STARSHIP" "$(first_line "$STARSHIP_BIN" --version)" "$(sha256_of "$STARSHIP_BIN")"
        return 0
    fi

    need hyperfine "https://github.com/sharkdp/hyperfine"
    # Before preparing fixtures: git stamps the untracked cache with the exclude
    # files it saw, so priming under a different HOME invalidates it.
    isolate_env
    prepare_all_fixtures

    [[ -n "$OUT_DIR" ]] || OUT_DIR="$ROOT/bench-results/compare-starship/$(date -u +%Y%m%dT%H%M%SZ)"
    local raw="$OUT_DIR/raw"
    mkdir -p "$raw/contended"
    cp "$HERE/repos.lock" "$HERE/versions.lock" "$HERE/starship.toml" "$OUT_DIR/"
    : >"$OUT_DIR/cells.ndjson"
    : >"$OUT_DIR/fixtures.ndjson"
    BLOCKS_FILE="$OUT_DIR/blocks.ndjson"
    : >"$BLOCKS_FILE"
    BUSY_FLOOR=$(awk -v c="$(ncpu)" 'BEGIN { printf "%.1f", 150 / c }')

    if [[ "$OS" == macos ]] && command -v caffeinate >/dev/null 2>&1; then
        caffeinate -i -w $$ &
        CAFFEINATE_PID=$!
    fi
    start_sampler
    quiet_machine_gate

    local started load_before
    started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    load_before=$(load1)

    local i repo dir scenario files activity total=0 done_cells=0
    total=$((${#REPO_NAMES[@]} * ${#SCENARIO_LIST[@]}))
    for i in "${!REPO_NAMES[@]}"; do
        repo=${REPO_NAMES[$i]}
        dir=$(repo_dir "$repo")
        files=$(git -C "$dir" ls-files | wc -l | tr -d '[:space:]')
        jq -n -c --arg name "$repo" --arg commit "${REPO_SHAS[$i]}" --arg ref "${REPO_REFS[$i]}" --argjson files "$files" \
            '{name: $name, commit: $commit, ref: $ref, files: $files}' >>"$OUT_DIR/fixtures.ndjson"
        for scenario in "${SCENARIO_LIST[@]}"; do
            done_cells=$((done_cells + 1))
            log "[$done_cells/$total] $repo ($files files), $scenario"
            if [[ "$scenario" == dirty ]]; then apply_dirty "$dir"; fi
            preflight "$dir" "$scenario"
            run_cell "$repo" "$scenario" "$dir" "$raw"
            if [[ "$scenario" == dirty ]]; then revert_dirty; fi
            activity=$(jq -s -c --arg repo "$repo" --arg scenario "$scenario" '
                [.[] | select(.repo == $repo and .scenario == $scenario)] as $b
                | ($b | map(select(.accepted))) as $a
                | {blocks: ($a | length),
                   retries: (($b | length) - ($a | length)),
                   contended_blocks: ($a | map(select(.contended)) | length),
                   mean_busy: ($a | map(select(.samples > 0) | .mean_busy) | if length > 0 then add / length else null end),
                   own_busy: ($a | map(select(.samples > 0) | .own_busy) | if length > 0 then add / length else null end),
                   max_busy: ($a | map(select(.samples > 0) | .max_busy) | if length > 0 then max else null end)}' "$BLOCKS_FILE")
            jq -s -c --argjson min_rounds "$MIN_ROUNDS" -f "$HERE/summarize.jq" "$raw/$repo.$scenario".r*.json |
                jq -c --arg repo "$repo" --arg scenario "$scenario" --argjson files "$files" --argjson activity "$activity" \
                    '{repo: $repo, scenario: $scenario, files: $files, activity: $activity,
                      summary: (if $activity.contended_blocks > 0
                                then . + {verdict: "inconclusive", contaminated: true}
                                else . + {contaminated: false} end)}' >>"$OUT_DIR/cells.ndjson"
        done
    done

    local finished load_after
    finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    load_after=$(load1)
    jq -n \
        --slurpfile cells "$OUT_DIR/cells.ndjson" \
        --slurpfile fixtures "$OUT_DIR/fixtures.ndjson" \
        --slurpfile blocks "$BLOCKS_FILE" \
        --arg gpy_desc "$DESC_GPY" --arg gpy_ver "$(first_line "$GPY_BIN" --version)" --arg gpy_sha "$(sha256_of "$GPY_BIN")" \
        --arg sh_desc "$DESC_STARSHIP" --arg sh_ver "$(first_line "$STARSHIP_BIN" --version)" --arg sh_sha "$(sha256_of "$STARSHIP_BIN")" \
        --arg started "$started" --arg finished "$finished" \
        --arg lb "$load_before" --arg la "$load_after" \
        --arg sampler "$SAMPLER_DESC" --arg busy_floor "$BUSY_FLOOR" \
        --argjson margin "$BUSY_MARGIN" --argjson max_retries "$MAX_BLOCK_RETRIES" \
        --arg cpu "$(cpu_model)" --argjson cpus "$(ncpu)" --arg os "$(uname -sr)" --arg arch "$(uname -m)" --arg power "$(power_source)" \
        --arg git_ver "$(git --version)" --arg hf_ver "$(hyperfine --version)" \
        --arg cmd_gpy "$(cmd_gpy REPO_DIR)" --arg cmd_sh "$(cmd_starship REPO_DIR)" \
        --argjson rounds "$ROUNDS" --argjson runs "$RUNS" --argjson warmup "$WARMUP" --argjson min_rounds "$MIN_ROUNDS" --arg untracked_cache "$UNTRACKED_CACHE" \
        '{
          env: {
            started_utc: $started, finished_utc: $finished,
            gpy: {description: $gpy_desc, version: $gpy_ver, sha256: $gpy_sha},
            starship: {description: $sh_desc, version: $sh_ver, sha256: $sh_sha},
            machine: {cpu: $cpu, cpus: $cpus, os: $os, arch: $arch, power: $power},
            load: {before: $lb, after: $la},
            monitor: {sampler: $sampler, busy_floor_percent: $busy_floor, margin_points: $margin, max_retries: $max_retries},
            git: {version: $git_ver},
            hyperfine: {version: $hf_ver}
          },
          params: {rounds: $rounds, runs: $runs, warmup: $warmup, min_rounds: $min_rounds, untracked_cache: $untracked_cache},
          commands: {gpy: $cmd_gpy, starship: $cmd_sh},
          fixtures: $fixtures,
          blocks: $blocks,
          cells: $cells
        }' >"$OUT_DIR/results.json"
    jq -r -f "$HERE/report.jq" "$OUT_DIR/results.json" >"$OUT_DIR/report.md"

    cat "$OUT_DIR/report.md"
    log ""
    log "results: $OUT_DIR"
}

# Sourcing (tests/bash/bench_compare_starship.test.bash) only defines functions.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
