# GPY vs Starship: prompt latency

How long GPY and [Starship](https://starship.rs) each take to draw a prompt
and to produce its git part, when and how it was measured, and how to repeat
it on your own machine.

## Whole prompt, per shell

What you wait for at each prompt: the shell's prompt hook drawing the whole
prompt (clock, last command duration, directory, git branch, state and
status), through each tool's real integration: GPY's `bash/gpy.bash`,
`zsh/gpy.zsh` and `fish_prompt.fish` with its agent running, against
`starship init <shell>`. Language detection is off in both. Median
milliseconds per prompt (min-max of the round medians); lower is better.

`steady` draws every prompt in the repository root; `cd` changes directory
before each prompt; `edit` modifies a tracked file before every other prompt
and restores it before the next, so git status changes between prompts.

macOS (Apple M4 Max, macOS 27.2.0; Bash 5.3, Zsh 5.9, Fish 4.9):

| Repository | Shell | Scenario | GPY | Starship |
|---|---|---|---:|---:|
| tiny-express | Bash | steady | 4.16 [4.14-4.17] | 19.50 [19.35-19.67] |
| tiny-express | Bash | cd | 4.25 [4.19-4.33] | 19.48 [19.30-19.81] |
| tiny-express | Bash | edit | 4.24 [4.21-4.44] | 19.62 [19.14-19.80] |
| tiny-express | Zsh | steady | 3.73 [3.66-3.85] | 18.49 [18.17-18.82] |
| tiny-express | Zsh | cd | 3.84 [3.76-4.00] | 18.58 [18.36-18.75] |
| tiny-express | Zsh | edit | 3.79 [3.75-3.89] | 18.73 [18.63-18.90] |
| tiny-express | Fish | steady | 3.18 [3.16-3.24] | 17.87 [17.57-17.98] |
| tiny-express | Fish | cd | 3.33 [3.25-3.38] | 17.94 [17.51-18.11] |
| tiny-express | Fish | edit | 3.26 [3.22-3.36] | 17.98 [17.83-18.22] |
| medium-kubernetes | Bash | steady | 4.26 [4.20-5.09] | 129.31 [126.79-130.12] |
| medium-kubernetes | Bash | cd | 5.56 [5.26-6.23] | 129.48 [128.38-129.84] |
| medium-kubernetes | Bash | edit | 4.29 [4.24-4.47] | 130.37 [128.24-132.52] |
| medium-kubernetes | Zsh | steady | 3.80 [3.74-3.81] | 128.52 [126.62-128.95] |
| medium-kubernetes | Zsh | cd | 4.22 [3.81-5.27] | 128.65 [126.64-131.07] |
| medium-kubernetes | Zsh | edit | 3.91 [3.85-4.01] | 128.79 [127.85-129.44] |
| medium-kubernetes | Fish | steady | 3.42 [3.36-3.55] | 127.98 [126.15-130.81] |
| medium-kubernetes | Fish | cd | 3.50 [3.42-3.55] | 128.16 [127.40-129.16] |
| medium-kubernetes | Fish | edit | 3.50 [3.47-3.73] | 130.26 [127.67-143.81] |

Linux (Debian 13 in Docker Desktop's VM on the same machine, 8 CPUs; Bash
5.2, Zsh 5.9, Fish 4.0):

| Repository | Shell | Scenario | GPY | Starship |
|---|---|---|---:|---:|
| tiny-express | Bash | steady | 1.14 [1.13-1.16] | 3.91 [3.89-3.97] |
| tiny-express | Bash | cd | 1.28 [1.26-1.28] | 3.94 [3.88-3.97] |
| tiny-express | Bash | edit | 1.18 [1.18-1.22] | 3.91 [3.83-4.00] |
| tiny-express | Zsh | steady | 1.29 [1.28-1.30] | 3.70 [3.53-3.78] |
| tiny-express | Zsh | cd | 1.42 [1.38-1.45] | 3.73 [3.70-3.86] |
| tiny-express | Zsh | edit | 1.33 [1.32-1.36] | 3.76 [3.69-3.81] |
| tiny-express | Fish | steady | 1.31 [1.29-1.34] | 3.83 [3.73-3.88] |
| tiny-express | Fish | cd | 1.47 [1.45-1.50] | 3.91 [3.87-3.94] |
| tiny-express | Fish | edit | 1.37 [1.34-1.38] | 3.89 [3.84-3.94] |
| medium-kubernetes | Bash | steady | 1.18 [1.12-1.20] | 57.72 [44.72-60.23] |
| medium-kubernetes | Bash | cd | 1.28 [1.27-1.71] | 57.53 [45.00-58.25] |
| medium-kubernetes | Bash | edit | 1.19 [1.17-1.45] | 51.52 [45.05-58.37] |
| medium-kubernetes | Zsh | steady | 1.31 [1.27-1.77] | 44.97 [44.55-57.72] |
| medium-kubernetes | Zsh | cd | 1.43 [1.34-1.95] | 44.72 [44.47-57.90] |
| medium-kubernetes | Zsh | edit | 1.39 [1.25-1.56] | 44.78 [44.61-45.03] |
| medium-kubernetes | Fish | steady | 1.52 [1.49-1.56] | 57.71 [57.54-57.91] |
| medium-kubernetes | Fish | cd | 1.45 [1.41-1.47] | 57.62 [45.04-58.05] |
| medium-kubernetes | Fish | edit | 1.36 [1.34-1.37] | 58.18 [57.77-58.56] |

In short: a GPY prompt costs 3–6 ms on macOS and about 1.5 ms on Linux in
every shell, whatever the repository size and whether files changed, because
git status comes from the agent's cache; Starship's prompt costs 3–5.5 times
that on a small repository and 25–45 times on a large one, where it runs git
status on every prompt. Both are the 0.12 code (commit `58adc132`,
`release-dist`) against the Starship v1.27.0 release binary, on 2026-11-19.

### How fresh is a GPY prompt?

Starship computes git status while it draws, so its prompt is always current
and pays for it in the tables above. GPY draws from the agent's cache: after
a file is saved, the agent's watcher waits 100 ms for related changes, rescans
the changed paths and rewrites the cache, and idle Zsh and Fish prompts
repaint then (Bash shows it at its next prompt). Milliseconds from writing a
tracked file until `gpy prompt` shows the change, and from restoring it until
the clean prompt is back (median / p90 / max of 20 edits):

| Platform | Repository | Edit shown | Restore shown |
|---|---|---:|---:|
| macOS | tiny-express | 136 / 141 / 145 | 138 / 142 / 144 |
| macOS | medium-kubernetes | 140 / 144 / 146 | 143 / 149 / 150 |
| Linux | tiny-express | 107 / 108 / 109 | 107 / 108 / 109 |
| Linux | medium-kubernetes | 112 / 114 / 115 | 111 / 114 / 114 |

A prompt drawn inside that window shows the previous state, then repaints.

### Why Linux is faster

Both platforms ran on the same M4 Max. Starting a process and walking a file
tree both cost several times more on macOS:

| | macOS | Linux VM |
|---|---:|---:|
| `/usr/bin/true` | 0.72 ms | 0.16 ms |
| `gpy --version` | 1.65 ms | 0.40 ms |
| `git status`, medium-kubernetes, no untracked cache | 145 ms | 37 ms |
| Starship git-only prompt, same repository | 117 ms | 44 ms |

A GPY prompt is mostly process start-up, so it follows the first two rows;
Starship on a large repository is mostly the tree walk, so it follows the
last two.

### Method

Method (`benchmarks/compare-starship/prompts.sh`): a sample is one fresh
shell that loads the integration, draws one prompt, then 20 more; per prompt
= (median of the 21-prompt samples - median of the 1-prompt samples) / 20, so
shell start-up and loading the integration cancel out. 6 rounds (plus a
warmup) x 5 samples per configuration, in rotated order. Both tools run in a
sandbox: an empty `HOME`, the machine's git configuration off, and a private
copy of each fixture whose index is refreshed once before timing, as a
user's own git commands keep it fresh. The Linux numbers come from a VM and
carry its overhead; treat them as indicative.

## Git status, one process

Median wall-clock time of one process invocation, in milliseconds. Lower is
better; a ratio below 1.00 means GPY is faster.

| Repository (tracked files) | Scenario | GPY | Starship | Ratio |
|---|---|---:|---:|---:|
| tiny-express (225) | clean | 9.53 | 12.36 | 0.77 |
| tiny-express (225) | dirty | 9.51 | 12.35 | 0.77 |
| small-redis (1,680) | clean | 10.31 | 13.01 | 0.79 |
| small-redis (1,680) | dirty | 10.54 | 13.15 | 0.80 |
| medium-kubernetes (27,597) | clean | 54.21 | 117.53 | 0.46 |
| medium-kubernetes (27,597) | dirty | 55.16 | 117.61 | 0.47 |
| large-tensorflow (34,570) | clean | 51.13 | 69.72 | 0.73 |
| large-tensorflow (34,570) | dirty | 51.66 | 69.70 | 0.74 |

Each cell is 6 rounds of 100 runs per tool (600 samples), with the order of the
two tools alternating every round. "GPY faster" is only claimed when GPY has the
lower median in every round; that held in every cell.

In short: one GPY process is about 1.25 times faster than one Starship process
on small repositories and 1.35 to 2.2 times faster on large ones.

### What was measured

| | |
|---|---|
| Date | 2026-11-19 |
| GPY | 0.12 code (commit `58adc132`, `release-dist` build, `gpy-agent` sha256 `aa189061…a85946`) |
| Starship | v1.27.0 release binary (sha256 `be60216a…1591b1`) |
| Machine | Apple M4 Max (12 performance + 4 efficiency cores, 64 GiB), macOS 27.2.0, internal SSD, on AC power |
| git | 2.56.1 |
| hyperfine | 2.0.0 |
| Fixtures | `expressjs/express` v5.1.0, `redis/redis` 8.0.0, `kubernetes/kubernetes` v1.34.0, `tensorflow/tensorflow` v2.20.0, each pinned to an exact commit ([`repos.lock`](../../benchmarks/compare-starship/repos.lock)) |
| `core.untrackedCache` | unset in the fixtures (see below) |

The raw report, including per-round medians, p95s, the commands timed and the
machine-activity record for each block, is what produced this table. Every
run writes one to `bench-results/compare-starship/<timestamp>/report.md`.

Both tools are asked for the same thing: branch, repository state (rebase,
merge, ...) and working-tree status. Starship is restricted to
`$git_branch$git_state$git_status` so it does no language detection or directory
truncation. Each runs with an empty `HOME` and with the machine's git
configuration switched off. Before timing, the harness checks that both tools
report the expected clean or dirty state and aborts if either does not.

- **clean**: the pinned commit, nothing modified.
- **dirty**: one unstaged edit, one staged edit and one untracked file.

### Why the untracked cache matters

The GPY agent adds git's untracked cache (`core.untrackedCache`) to a
repository's index where you have not set it either way, and your own git
commands can add it too. Starship never uses it. That is most of GPY's lead on
large repositories, so the results depend on this setting:

| Fixture setting | What happens | medium-kubernetes (GPY / Starship) |
|---|---|---|
| unset (the default here, and typical) | the fixture primes the cache, as your own git commands would; GPY's one-shot path does not add it, the agent does | 54.2 / 117.5 ms |
| `true`, for both tools | same | about 55 / 114 ms (v0.4.0 run) |
| `false` | GPY respects it and loses its fast path | about 143 / 114 ms (v0.4.0 run) |

If you have set `core.untrackedCache = false` yourself, GPY honours that, as
git and other tools do, and large repositories take about 2.5 times longer. The
benchmark can measure all three; see the options below.

### What this does not show

- **It is not the latency you see at your prompt.** That is the
  [whole-prompt table](#whole-prompt-per-shell) above: in a shell, GPY answers
  from a long-running agent. This benchmark times `gpy-agent oneshot git`, the
  no-agent fallback that, like Starship, starts a new process for every call.
  It is the like-for-like process comparison.
- Powerlevel10k and other prompts, which use a different architecture.
- A fixed point in time. Both tools are pinned to the versions above, and the
  harness is the only thing that makes a re-run comparable.

Absolute numbers depend on the machine. Treat the ratios as the result and the
milliseconds as context.

## Run it yourself

You need `git`, `curl`, `tar`, `jq` and [`hyperfine`](https://github.com/sharkdp/hyperfine)
(1.x or 2.x), on macOS or Linux. From a clone of the repository:

```sh
just bench-compare
# or, without just:
./benchmarks/compare-starship/run.sh
```

That downloads the pinned GPY and Starship releases (checking each against its
published SHA-256), fetches the four fixture repositories (about 900 MB in
total, once, cached in `~/.cache/gpy-bench`), and runs everything. A
full run takes about 10 to 45 minutes depending on how busy the machine is.

Close other work first and stay on AC power. The harness watches system-wide CPU
use for the whole run, subtracts what the benchmark itself uses, re-runs any
block that overlapped with other activity, and says so in the report if it could
not get a quiet window. Do not touch the machine while it runs.

Useful variations:

```sh
# The Starship on your PATH, or the newest releases of both
./benchmarks/compare-starship/run.sh --starship system
# Your installed GPY (an existing gpy-agent binary) against the pinned Starship
./benchmarks/compare-starship/run.sh --gpy "$(command -v gpy-agent)"
./benchmarks/compare-starship/run.sh --gpy latest --starship latest

# One repository, a quick check of the setup
./benchmarks/compare-starship/run.sh --repos tiny-express --scenarios clean \
    --rounds 5 --runs 20 --force

# The untracked-cache variants: unset (default), on, off
./benchmarks/compare-starship/run.sh --untracked-cache off
```

Compare release binaries, not a local `cargo build --release`: local release
builds keep symbols, while shipped binaries use the stripped `release-dist`
profile. Reading the report, the full method, every option and the fairness rules
are in [`benchmarks/compare-starship/README.md`](../../benchmarks/compare-starship/README.md).
If you share a result, include the report's machine and version lines with it.

The whole-prompt comparison needs a `gpy` and `gpy-agent` build, the fixtures
above and the shells you name:

```sh
(cd gpy-agent && cargo build --profile release-dist --bins)
./benchmarks/compare-starship/prompts.sh --gpy-dir gpy-agent/target/release-dist
```
