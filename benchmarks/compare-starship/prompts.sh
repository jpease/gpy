#!/usr/bin/env bash
# benchmarks/compare-starship/prompts.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY vs Starship: the cost of one whole prompt in Bash, Zsh and Fish, through
# each tool's real shell integration (`bash/gpy.bash`, `zsh/gpy.zsh`,
# `fish/functions/fish_prompt.fish`; `starship init <shell>`). Both draw the
# same modules: clock, last command duration, directory and git (branch,
# state, status); language detection is off in both.
#
# A sample is one fresh shell that loads the integration, draws one prompt,
# then ITERS more; per prompt = (median of the ITERS-prompt samples - median of
# the 1-prompt samples) / ITERS, so shell start-up, loading the integration
# and GPY's first-prompt agent start cancel out. Every round visits every
# configuration in rotated order; the result is the median over rounds >= 1
# (round 0 is a warmup). GPY runs against its own agent, started in the
# sandbox and warm before timing.
#
# Scenarios: `steady` draws every prompt in the repository root; `cd`
# alternates between the root and a subdirectory, as a `cd` before each prompt.
#
# Usage: prompts.sh --gpy-dir DIR [--starship BIN] [--repos LIST]
#                   [--shells LIST] [--rounds N] [--runs N] [--iters N]
#   --gpy-dir DIR   directory holding `gpy` and `gpy-agent` (a release-dist build)
#   --starship BIN  Starship binary (default: `starship` on PATH)
#   --repos LIST    fixture names under ~/.cache/gpy-bench/repos (run.sh fixtures)
#                   (default: tiny-express,medium-kubernetes)
#   --shells LIST   default bash,zsh,fish
# Results: bench-results/compare-starship/prompts-<timestamp>/{samples.tsv,report.md}

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

GPY_DIR="" STARSHIP_BIN="$(command -v starship || true)"
REPOS="tiny-express,medium-kubernetes" SHELLS="bash,zsh,fish"
ROUNDS=6 RUNS=5 ITERS=20
while (($#)); do
    case "$1" in
        --gpy-dir) GPY_DIR="$2"; shift 2 ;;
        --starship) STARSHIP_BIN="$2"; shift 2 ;;
        --repos) REPOS="$2"; shift 2 ;;
        --shells) SHELLS="$2"; shift 2 ;;
        --rounds) ROUNDS="$2"; shift 2 ;;
        --runs) RUNS="$2"; shift 2 ;;
        --iters) ITERS="$2"; shift 2 ;;
        -h | --help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
[[ -x "$GPY_DIR/gpy" && -x "$GPY_DIR/gpy-agent" ]] || { echo "--gpy-dir must hold gpy and gpy-agent" >&2; exit 2; }
[[ -x "$STARSHIP_BIN" ]] || { echo "no starship binary" >&2; exit 2; }
for tool in hyperfine jq git; do
    command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }
done

shell_bin() {
    case "$1" in
        bash)
            # ${PS1@P} needs Bash 4.4.
            local c
            for c in "${GPY_BENCH_BASH:-}" "$(command -v bash)" /opt/homebrew/bin/bash /usr/local/bin/bash; do
                [[ -n "$c" && -x "$c" ]] || continue
                # shellcheck disable=SC2016
                "$c" -c '((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4)))' 2>/dev/null &&
                    { printf '%s' "$c"; return; }
            done
            return 1
            ;;
        *) command -v "$1" ;;
    esac
}

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$ROOT/bench-results/compare-starship/prompts-$STAMP"
SB="$(mktemp -d "${TMPDIR:-/tmp}/gpy-prompts.XXXXXX")"
SB="$(cd "$SB" && pwd -P)"
mkdir -p "$OUT" "$SB/home" "$SB/config/gpy" "$SB/cache" "$SB/run" "$SB/repos" "$SB/bin"
chmod 700 "$SB/run"
ln -s "$GPY_DIR/gpy" "$SB/bin/gpy"
ln -s "$GPY_DIR/gpy-agent" "$SB/bin/gpy-agent"
ln -s "$STARSHIP_BIN" "$SB/bin/starship"

cat >"$SB/config/gpy/config.toml" <<'EOF'
[ui]
theme = "default"
enabled_segments = ["clock", "duration", "directory", "git"]
EOF
cat >"$SB/starship.toml" <<'EOF'
add_newline = true
command_timeout = 10000
format = "$time$cmd_duration$directory$git_branch$git_state$git_status$character"

[time]
disabled = false
EOF

# One environment for every sample: nothing from the caller's shell, the
# machine's git configuration off, both tools' configs in the sandbox.
ENV_ARGS=(env -i "HOME=$SB/home" "USER=${USER:-bench}" "PATH=$SB/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    "TERM=xterm-256color" "LANG=${LANG:-en_US.UTF-8}"
    "XDG_CONFIG_HOME=$SB/config" "XDG_CACHE_HOME=$SB/cache" "XDG_RUNTIME_DIR=$SB/run"
    "GIT_CONFIG_GLOBAL=/dev/null" "GIT_CONFIG_SYSTEM=/dev/null" "GIT_OPTIONAL_LOCKS=0"
    "STARSHIP_CONFIG=$SB/starship.toml" "STARSHIP_CACHE=$SB/cache/starship")

cleanup() {
    "${ENV_ARGS[@]}" "$GPY_DIR/gpy-agent" stop >/dev/null 2>&1 || true
    rm -rf "$SB"
}
trap cleanup EXIT INT TERM

# Fixtures: a private copy of each (a clone on APFS), so neither tool's index
# writes reach the cached repositories.
IFS=, read -r -a REPO_LIST <<<"$REPOS"
for repo in "${REPO_LIST[@]}"; do
    src="$HOME/.cache/gpy-bench/repos/$repo"
    [[ -d "$src/.git" ]] || { echo "no fixture $src (run: benchmarks/compare-starship/run.sh fixtures)" >&2; exit 2; }
    cp -Rc "$src" "$SB/repos/$repo" 2>/dev/null || cp -R "$src" "$SB/repos/$repo"
    sub="$(cd "$SB/repos/$repo" && find . -mindepth 1 -maxdepth 1 -type d ! -name '.*' | sort | head -n 1)"
    printf '%s\n' "${sub#./}" >"$SB/repos/$repo.sub"
done

"${ENV_ARGS[@]}" "$GPY_DIR/gpy-agent" start >/dev/null
for _ in $(seq 1 50); do
    "${ENV_ARGS[@]}" "$GPY_DIR/gpy-agent" status >/dev/null 2>&1 && break
    sleep 0.1
done

# The body a shell runs: load TOOL's integration, draw one prompt, then N more.
body() {
    local tool="$1" shell="$2" repo="$3" scenario="$4" n="$5" root="$SB/repos/$3" sub
    sub="$root/$(cat "$SB/repos/$repo.sub")"
    local q_root q_sub
    q_root="$(printf '%q' "$root")" q_sub="$(printf '%q' "$sub")"
    case "$shell" in
        bash)
            local init draw
            if [[ $tool == gpy ]]; then
                init="source $(printf '%q' "$ROOT/bash/gpy.bash")"
                draw='__gpy_precmd; : "${PS1@P}"'
            else
                init='eval "$(starship init bash --print-full-init)"'
                draw='starship_precmd; : "${PS1@P}"'
            fi
            local step="$draw"
            [[ $scenario == cd ]] && step="if ((i % 2)); then builtin cd $q_root; else builtin cd $q_sub; fi; $draw"
            printf '%s' "builtin cd $q_root && $init && $draw && for ((i = 0; i < $n; i++)); do $step; done"
            ;;
        zsh)
            local init
            if [[ $tool == gpy ]]; then
                init="source $(printf '%q' "$ROOT/zsh/gpy.zsh")"
            else
                init='eval "$(starship init zsh)"'
            fi
            local draw='for f in $precmd_functions; do $f; done; print -rP -- "$PROMPT" >/dev/null'
            local step="$draw"
            [[ $scenario == cd ]] && step="if (( i % 2 )); then builtin cd $q_root; else builtin cd $q_sub; fi; $draw"
            printf '%s' "builtin cd $q_root && $init && { $draw }; for ((i = 0; i < $n; i++)); do $step; done"
            ;;
        fish)
            local init
            if [[ $tool == gpy ]]; then
                init="set -g fish_function_path $(printf '%q' "$ROOT/fish/functions") \$fish_function_path"
            else
                init='starship init fish | source'
            fi
            local draw='fish_prompt >/dev/null'
            local step="$draw"
            [[ $scenario == cd ]] && step="if test (math \$i % 2) -eq 1; builtin cd $q_root; else; builtin cd $q_sub; end; $draw"
            # A counted loop: BSD `seq 1 0` counts down and prints two lines.
            printf '%s' "builtin cd $q_root; and $init; and $draw; set -l i 0; while test \$i -lt $n; $step; set i (math \$i + 1); end"
            ;;
    esac
}

command_for() {
    local tool="$1" shell="$2" repo="$3" scenario="$4" n="$5" exe out a
    exe="$(shell_bin "$shell")"
    local -a argv=("${ENV_ARGS[@]}" "$exe")
    case "$shell" in
        bash) argv+=(--noprofile --norc) ;;
        zsh) argv+=(-f) ;;
        fish) argv+=(--no-config) ;;
    esac
    argv+=(-c "$(body "$tool" "$shell" "$repo" "$scenario" "$n")")
    out=""
    for a in "${argv[@]}"; do out+="$(printf '%q' "$a") "; done
    printf '%s' "${out% }"
}

IFS=, read -r -a SHELL_LIST <<<"$SHELLS"
CONFIGS=()
for repo in "${REPO_LIST[@]}"; do
    for shell in "${SHELL_LIST[@]}"; do
        shell_bin "$shell" >/dev/null || { echo "no usable $shell" >&2; exit 2; }
        for scenario in steady cd; do
            for tool in gpy starship; do CONFIGS+=("$tool:$shell:$repo:$scenario"); done
        done
    done
done

# Preflight: every configuration draws a prompt that shows the branch, and
# warms the agent's cache for each repository.
for cfg in "${CONFIGS[@]}"; do
    IFS=: read -r tool shell repo scenario <<<"$cfg"
    eval "$(command_for "$tool" "$shell" "$repo" "$scenario" 2)" >/dev/null 2>&1 ||
        { echo "preflight failed: $cfg" >&2; exit 1; }
done
for repo in "${REPO_LIST[@]}"; do
    for tool in gpy starship; do
        for shell in "${SHELL_LIST[@]}"; do
            exe="$(shell_bin "$shell")"
            case "$shell:$tool" in
                bash:gpy) shown="$("${ENV_ARGS[@]}" "$exe" --noprofile --norc -c "builtin cd $(printf '%q' "$SB/repos/$repo") && source $(printf '%q' "$ROOT/bash/gpy.bash") && __gpy_precmd && printf %s \"\${PS1@P}\"")" ;;
                bash:starship) shown="$("${ENV_ARGS[@]}" "$exe" --noprofile --norc -c "builtin cd $(printf '%q' "$SB/repos/$repo") && eval \"\$(starship init bash --print-full-init)\" && starship_precmd && printf %s \"\${PS1@P}\"")" ;;
                zsh:gpy) shown="$("${ENV_ARGS[@]}" "$exe" -f -c "builtin cd $(printf '%q' "$SB/repos/$repo") && source $(printf '%q' "$ROOT/zsh/gpy.zsh") && __gpy_precmd && print -rP -- \"\$PROMPT\"")" ;;
                zsh:starship) shown="$("${ENV_ARGS[@]}" "$exe" -f -c "builtin cd $(printf '%q' "$SB/repos/$repo") && eval \"\$(starship init zsh)\" && for f in \$precmd_functions; do \$f; done && print -rP -- \"\$PROMPT\"")" ;;
                fish:gpy) shown="$("${ENV_ARGS[@]}" "$exe" --no-config -c "builtin cd $(printf '%q' "$SB/repos/$repo"); set -g fish_function_path $(printf '%q' "$ROOT/fish/functions") \$fish_function_path; fish_prompt")" ;;
                fish:starship) shown="$("${ENV_ARGS[@]}" "$exe" --no-config -c "builtin cd $(printf '%q' "$SB/repos/$repo"); starship init fish | source; fish_prompt")" ;;
            esac
            branch="$(git -C "$SB/repos/$repo" rev-parse --abbrev-ref HEAD)"
            [[ $shown == *"$branch"* ]] || { echo "preflight: $tool/$shell in $repo shows no branch '$branch'" >&2; exit 1; }
        done
    done
done

load1() {
    if [[ -r /proc/loadavg ]]; then cut -d' ' -f1 /proc/loadavg; else sysctl -n vm.loadavg | awk '{print $2}'; fi
}

TSV="$OUT/samples.tsv"
printf 'round\tconfig\tms_per_prompt\tload1\n' >"$TSV"
n=${#CONFIGS[@]}
for ((r = 0; r <= ROUNDS; r++)); do
    k=$((r % n))
    order=("${CONFIGS[@]:k}" "${CONFIGS[@]:0:k}")
    args=()
    for cfg in "${order[@]}"; do
        IFS=: read -r tool shell repo scenario <<<"$cfg"
        args+=("$(command_for "$tool" "$shell" "$repo" "$scenario" "$ITERS")" "$(command_for "$tool" "$shell" "$repo" "$scenario" 0)")
    done
    l="$(load1)"
    hyperfine -N --warmup 1 --runs "$RUNS" --export-json "$OUT/round-$r.json" "${args[@]}" >/dev/null 2>&1 ||
        { echo "hyperfine failed in round $r" >&2; exit 1; }
    jq -r '.results[] | (.times // [.measurements[].time_wall_clock.value]) | sort
        | if length % 2 == 1 then .[length / 2 | floor] else (.[length / 2 - 1] + .[length / 2]) / 2 end' "$OUT/round-$r.json" |
        awk -v r="$r" -v it="$ITERS" -v l="$l" -v cfgs="${order[*]}" '
            BEGIN { m = split(cfgs, c, " ") }
            { t[NR] = $1 }
            END { for (i = 1; i <= m; i++) printf "%d\t%s\t%.3f\t%s\n", r, c[i], (t[2 * i - 1] - t[2 * i]) * 1000 / it, l }' >>"$TSV"
    echo "round $r/$ROUNDS done (load1 $l)" >&2
done

median_of() {
    awk -F'\t' -v c="$1" 'NR > 1 && $1 > 0 && $2 == c { print $3 }' "$TSV" | sort -g |
        awk '{ a[++n] = $1 } END { if (!n) exit; m = (n % 2) ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2; printf "%.2f %.2f %.2f", m, a[1], a[n] }'
}

{
    echo "# GPY vs Starship: one prompt, per shell"
    echo
    echo "Generated $(date -u +%Y-%m-%dT%H:%M:%SZ). Median ms per prompt over rounds 1-$ROUNDS (min-max in brackets); lower is better."
    echo
    echo "| Repository | Shell | Scenario | GPY | Starship | Ratio |"
    echo "|---|---|---|---:|---:|---:|"
    for repo in "${REPO_LIST[@]}"; do
        for shell in "${SHELL_LIST[@]}"; do
            for scenario in steady cd; do
                read -r gm glo ghi <<<"$(median_of "gpy:$shell:$repo:$scenario")"
                read -r sm slo shi <<<"$(median_of "starship:$shell:$repo:$scenario")"
                ratio="$(awk -v g="$gm" -v s="$sm" 'BEGIN { printf "%.2f", g / s }')"
                echo "| $repo | $shell | $scenario | $gm [$glo-$ghi] | $sm [$slo-$shi] | $ratio |"
            done
        done
    done
    echo
    echo "## Setup"
    echo
    echo "- GPY: $("$GPY_DIR/gpy" --version 2>&1 | head -n 1) ($GPY_DIR)"
    echo "- Starship: $("$STARSHIP_BIN" --version 2>&1 | head -n 1)"
    for shell in "${SHELL_LIST[@]}"; do
        exe="$(shell_bin "$shell")"
        echo "- $shell: $exe ($("$exe" --version 2>&1 | head -n 1))"
    done
    echo "- git: $(git --version)"
    echo "- hyperfine: $(hyperfine --version)"
    echo "- Machine: $(uname -srm); $(if command -v sysctl >/dev/null && sysctl -n machdep.cpu.brand_string >/dev/null 2>&1; then sysctl -n machdep.cpu.brand_string; else grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2-; fi)"
    echo "- Method: $ROUNDS rounds (+1 warmup) x $RUNS samples x $ITERS prompts per sample; per prompt = (t[1+$ITERS prompts] - t[1 prompt]) / $ITERS."
    echo "- Load1 per round: $(awk -F'\t' 'NR > 1 { l[$1] = $4 } END { for (r in l) printf "%s ", l[r] }' "$TSV")"
} >"$OUT/report.md"
cat "$OUT/report.md"
