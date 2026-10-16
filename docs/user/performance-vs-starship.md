# GPY vs Starship: git status latency

How long GPY and [Starship](https://starship.rs) each take to produce the git
part of a prompt, when and how it was measured, and how to repeat it on your own
machine.

## Results

Median wall-clock time of one process invocation, in milliseconds. Lower is
better; a ratio below 1.00 means GPY is faster.

| Repository (tracked files) | Scenario | GPY | Starship | Ratio |
|---|---|---:|---:|---:|
| tiny-express (225) | clean | 10.45 | 10.83 | 0.96 |
| tiny-express (225) | dirty | 10.46 | 10.78 | 0.97 |
| small-redis (1,680) | clean | 11.33 | 11.38 | 1.00 (tie) |
| small-redis (1,680) | dirty | 11.47 | 11.38 | 1.01 (tie) |
| medium-kubernetes (27,597) | clean | 54.89 | 108.59 | 0.51 |
| medium-kubernetes (27,597) | dirty | 55.83 | 109.44 | 0.51 |
| large-tensorflow (34,570) | clean | 52.46 | 68.71 | 0.76 |
| large-tensorflow (34,570) | dirty | 53.12 | 68.93 | 0.77 |

Each cell is 6 rounds of 100 runs per tool (600 samples), with the order of the
two tools alternating every round. "GPY faster" is only claimed when GPY has the
lower median in every round. That held in every cell except the two
`small-redis` ones, where the difference is a tenth of a millisecond and the
harness does not call a winner.

In short: GPY is a little faster on small repositories and about 1.3 to 2 times
faster on large ones. On repositories of a few thousand files or fewer the two
are indistinguishable.

### What was measured

| | |
|---|---|
| Date | 2026-10-15 |
| GPY | v0.2.0 release binary (`gpy-agent 0.2.0`, sha256 `495cc9a5…e2ce5c`) |
| Starship | v1.26.0 release binary (sha256 `01532ebb…b245da`) |
| Machine | Apple M4 Max (12 performance + 4 efficiency cores, 64 GiB), macOS 27.0.1, internal SSD, on AC power |
| git | 2.56.0 |
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

GPY turns on git's untracked cache (`core.untrackedCache`) the first time it
runs in a repository where you have not set it either way. Starship never uses
it. That is most of GPY's lead on large repositories, so the results depend on
this setting:

| Fixture setting | What happens | medium-kubernetes (GPY / Starship) |
|---|---|---|
| unset (the default here, and typical) | GPY enables the cache | 54.9 / 108.6 ms |
| `true`, for both tools | same | about 55 / 114 ms |
| `false` | GPY respects it and loses its fast path | about 143 / 114 ms |

If you have set `core.untrackedCache = false` yourself, GPY honours that, as
git and other tools do, and large repositories take about 2.5 times longer. The
benchmark can measure all three; see the options below.

### What this does not show

- **It is not the latency you see at your prompt.** In a shell, GPY normally
  answers from a long-running agent over a local socket, and cached prompts draw
  without waiting. The benchmark times `gpy-agent oneshot git`, the no-agent
  fallback that, like Starship, starts a new process for every call. It is the
  like-for-like process comparison. Do not quote it as prompt latency.
- Full prompt rendering, shell start-up, or perceived (asynchronous) latency.
- Powerlevel10k and other prompts, which use a different architecture.
- Other operating systems or CPUs. One machine, one OS.
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
# Your installed GPY and Starship, or the newest releases of both
./benchmarks/compare-starship/run.sh --starship system
./benchmarks/compare-starship/run.sh --gpy latest --starship latest

# One repository, a quick check of the setup
./benchmarks/compare-starship/run.sh --repos tiny-express --scenarios clean \
    --rounds 5 --runs 20 --force

# The untracked-cache variants: unset (default), on, off
./benchmarks/compare-starship/run.sh --untracked-cache off
```

Compare release binaries, not a local `cargo build --release`: local release
builds are tuned for build speed (`opt-level = 1`), and shipped binaries are
not. Reading the report, the full method, every option and the fairness rules
are in [`benchmarks/compare-starship/README.md`](../../benchmarks/compare-starship/README.md).
If you share a result, include the report's machine and version lines with it.
