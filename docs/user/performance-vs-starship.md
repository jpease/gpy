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
| tiny-express | Bash | steady | 5.72 [5.58-5.75] | 19.06 [19.00-20.09] |
| tiny-express | Bash | cd | 5.82 [5.73-6.18] | 19.08 [19.00-19.66] |
| tiny-express | Bash | edit | 5.75 [5.62-5.91] | 19.33 [19.15-19.62] |
| tiny-express | Zsh | steady | 5.24 [5.08-5.29] | 18.11 [17.99-18.27] |
| tiny-express | Zsh | cd | 5.42 [5.25-5.47] | 18.08 [17.47-18.44] |
| tiny-express | Zsh | edit | 5.37 [5.32-5.58] | 18.36 [18.09-19.03] |
| tiny-express | Fish | steady | 4.75 [4.71-4.93] | 17.57 [17.32-17.86] |
| tiny-express | Fish | cd | 4.87 [4.84-4.94] | 17.58 [17.26-17.63] |
| tiny-express | Fish | edit | 4.80 [4.76-4.84] | 17.68 [17.50-18.51] |
| medium-kubernetes | Bash | steady | 8.03 [5.58-8.17] | 132.25 [130.76-132.92] |
| medium-kubernetes | Bash | cd | 8.31 [8.22-8.51] | 132.23 [131.69-133.48] |
| medium-kubernetes | Bash | edit | 6.33 [5.47-6.83] | 131.95 [131.42-134.05] |
| medium-kubernetes | Zsh | steady | 7.53 [7.39-7.66] | 131.30 [130.54-132.67] |
| medium-kubernetes | Zsh | cd | 7.63 [7.49-7.85] | 131.33 [130.28-133.78] |
| medium-kubernetes | Zsh | edit | 5.87 [5.20-6.07] | 131.26 [130.71-132.81] |
| medium-kubernetes | Fish | steady | 5.09 [5.05-5.28] | 130.45 [130.03-132.22] |
| medium-kubernetes | Fish | cd | 4.95 [4.76-5.44] | 130.17 [128.31-133.08] |
| medium-kubernetes | Fish | edit | 4.99 [4.91-5.44] | 131.44 [130.04-132.54] |

Linux (Debian 13 in Docker Desktop's VM on the same machine, 8 CPUs; Bash
5.2, Zsh 5.9, Fish 4.0). Not re-run for this release: these figures are from
2026-11-19, with the 0.12 code at commit `58adc132` against Starship 1.27.0:

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

In short: a GPY prompt costs 5–8 ms on macOS in every shell, whatever the
repository size and whether files changed, because git status comes from the
agent's cache; Starship's prompt costs 3–4 times that on a small repository
and 16–26 times on a large one, where it runs git status on every prompt. On
Linux a GPY prompt costs about 1.5 ms (2026-11-19 run, table above). The macOS
numbers are GPY 0.12.1 (`release-dist`, with the dependency updates for the
next release) against Homebrew's Starship 1.27.0, on 2026-11-26.

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
| macOS | tiny-express | 135 / 141 / 142 | 136 / 140 / 144 |
| macOS | medium-kubernetes | 138 / 144 / 145 | 137 / 146 / 151 |
| Linux | tiny-express | 107 / 108 / 109 | 107 / 108 / 109 |
| Linux | medium-kubernetes | 112 / 114 / 115 | 111 / 114 / 114 |

A prompt drawn inside that window shows the previous state, then repaints.

### Why Linux is faster

Both platforms ran on the same M4 Max (these figures and the Linux rows above
are from 2026-11-19; not re-run). Starting a process and walking a file
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
| tiny-express (225) | clean | 9.07 | 12.01 | 0.76 |
| tiny-express (225) | dirty | 8.99 | 12.02 | 0.75 |
| small-redis (1,680) | clean | 10.21 | 12.60 | 0.81 |
| small-redis (1,680) | dirty | 10.33 | 12.47 | 0.83 |
| medium-kubernetes (27,597) | clean | 50.71 | 115.89 | 0.44 |
| medium-kubernetes (27,597) | dirty | 52.17 | 116.20 | 0.45 |
| large-tensorflow (34,570) | clean | 49.47 | 68.43 | 0.72 |
| large-tensorflow (34,570) | dirty | 50.14 | 68.62 | 0.73 |

Each cell is 6 rounds of 100 runs per tool (600 samples), with the order of the
two tools alternating every round. "GPY faster" is only claimed when GPY has the
lower median in every round; that held in every cell.

In short: one GPY process is about 1.2 to 1.3 times faster than one Starship
process on small repositories and 1.35 to 2.3 times faster on large ones.

### What was measured

| | |
|---|---|
| Date | 2026-11-26 |
| GPY | 0.12.1 plus the dependency updates for the next release (code as of commit `3b1afc66`), `release-dist` build, `gpy-agent` sha256 `0980257a…cd4844` |
| Starship | 1.27.0, Homebrew build (sha256 `c8e79626…460737`) |
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
| unset (the default here, and typical) | the fixture primes the cache, as your own git commands would; GPY's one-shot path does not add it, the agent does | 50.7 / 115.9 ms |
| `true`, for both tools | same | 51.0 / 117.8 ms |
| `false` | GPY respects it and loses its fast path | 147.8 / 117.6 ms |

If you have set `core.untrackedCache = false` yourself, GPY honours that, as
git and other tools do, and large repositories take about 2.9 times longer. The
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
./benchmarks/compare-starship/prompts.sh --gpy-dir "$PWD/gpy-agent/target/release-dist"
```
