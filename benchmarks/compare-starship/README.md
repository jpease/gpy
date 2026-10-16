# GPY vs Starship benchmark

Reproducible comparison of the time GPY and Starship each take to produce one
git-status prompt segment. Everything that affects the numbers is pinned,
spelled out, or recorded next to the results, so that you can rerun it and
compare.

```bash
just bench-compare                 # pinned GPY, pinned Starship, pinned repositories
./benchmarks/compare-starship/run.sh --help
```

Needs `git`, `curl`, `tar`, `jq`, [`hyperfine`](https://github.com/sharkdp/hyperfine)
(1.x or 2.x) and, for `--gpy local` / `--gpy ref:REV`, a Rust toolchain. Works on
macOS and Linux. Run it on an otherwise idle machine: the harness checks that
(see [Apples to apples](#apples-to-apples)) and refuses to start otherwise.

## What is measured

One process invocation, wall-clock, per tool:

| Tool | Command |
|---|---|
| GPY | `gpy-agent oneshot git --cwd <repo> --format json` |
| Starship | `starship prompt --path <repo>` with [`starship.toml`](starship.toml), whose `format` is `$git_branch$git_state$git_status` |

Both report the branch, repository state (rebase, merge, ...) and working-tree
status. Starship's config contains only those modules so it does no language
detection, directory truncation or anything else that GPY's git call does not
do. Each runs with an empty `HOME`, so neither reads a user config, and with
the git configuration of the machine switched off (`GIT_CONFIG_GLOBAL` and
`GIT_CONFIG_SYSTEM` point at `/dev/null`); the relevant settings are set
explicitly in each fixture instead. The one setting that changes what the tools
do is `core.untrackedCache`, chosen with `--untracked-cache`:

| Mode | Fixture | Why |
|---|---|---|
| `unset` (default) | key removed, cache primed | a typical machine; GPY switches it on itself, Starship never uses it |
| `on` | pinned `true`, cache primed | identical inputs for both tools |
| `off` | pinned `false` | GPY honours an explicit `false`, so it loses its fast path (a large repository takes about 2.5x longer) |

Priming is a `git status` that may write the index, run under the same
isolated environment as the timing (git stamps the cache with the exclude
files it saw, so priming under another `HOME` invalidates it on every call).
GPY's own status runs with `GIT_OPTIONAL_LOCKS=0` and never fills the cache.

Two scenarios per repository:

- `clean`: the pinned commit, nothing modified.
- `dirty`: one unstaged edit, one staged edit and one untracked file, applied
  identically by the harness and removed afterwards.

### What this does not measure

- **GPY's normal path.** In a shell, GPY answers from a long-running agent over a
  local socket. `oneshot` is its no-agent
  fallback and pays process start-up on every call, exactly as Starship does.
  This benchmark is therefore the like-for-like process comparison, not the
  latency a GPY user sees at the prompt. Do not quote it as that.
- Full prompt rendering, shell start-up, or perceived (asynchronous) latency.
- Powerlevel10k, whose `gitstatusd` daemon is a different architecture and is not
  compared here.

## Fixtures

[`repos.lock`](repos.lock) pins four public repositories, each to an exact commit
(not a branch tip):

| Name | Repository | Pinned to |
|---|---|---|
| tiny-express | expressjs/express | v5.1.0 |
| small-redis | redis/redis | 8.0.0 |
| medium-kubernetes | kubernetes/kubernetes | v1.34.0 |
| large-tensorflow | tensorflow/tensorflow | v2.20.0 |

The harness fetches exactly that commit (`git fetch --depth 1 origin <sha>`),
checks `HEAD` against the pinned SHA, checks out a local branch named `bench`,
and verifies the work tree is clean. Tracked-file counts are measured and
printed in the report. To move to a newer point in time, change the `commit`
and `ref` columns in `repos.lock`.

Run `./run.sh fixtures` to fetch them ahead of time. They are cached in
`$GPY_BENCH_HOME` (default `~/.cache/gpy-bench`).

## Tool versions

`--gpy` and `--starship` each take a spec:

| Spec | GPY | Starship |
|---|---|---|
| `pinned` (default) | the release tag in [`versions.lock`](versions.lock) | the release in `versions.lock` |
| `latest` | newest GitHub release | newest GitHub release |
| `vX.Y.Z` / `X.Y.Z` | that release | that release |
| `local` | `cargo build --release --locked` of this working tree | |
| `ref:REV` | `cargo build --release --locked` of that commit | |
| `system` | | the `starship` on `PATH` |
| a path | an existing `gpy-agent` | an existing `starship` |

Release downloads are verified against the `.sha256` file published with each
asset. The binaries actually timed are copied into one directory under the cache
so that neither is loaded from slower storage than the other, and their SHA-256
and `--version` are recorded in the results.

Release binaries and local builds can differ in compiler flags and CPU target.
For numbers that others can reproduce, compare pinned releases; use `local` /
`ref:` for regression checks on one machine.

## Apples to apples

| Concern | What the harness does |
|---|---|
| Same inputs | Same commit of the same repositories, same dirty state, same git, same machine, same moment |
| Tools running different work | Preflight before timing: GPY's JSON and Starship's output are parsed and must show the branch and the expected clean / dirty state; either tool failing or reporting the wrong state aborts the run |
| Shell overhead | `hyperfine -N`: no shell, each tool is `exec`ed directly |
| Ordering and drift | Each cell is timed in several rounds; the order of the two tools alternates every round, and results are pooled across rounds |
| Cold caches | Warmup runs before every block; the git index is refreshed after every change to the work tree (a "racily clean" index would make both tools re-hash files on every call) |
| Machine not quiet | See below |
| Config and environment | Isolated `HOME`/`XDG_*`, git config neutralised, `GPY_*`, `STARSHIP_*`, `GIT_DIR`... unset, `TERM` fixed; the Starship config is in the repository |
| Storage | Binaries and fixtures live on the cache volume, not the working tree |
| Sleep | On macOS the run holds `caffeinate` |

### Watching the machine while it runs

A single check before the run is not enough: Spotlight, a backup or a browser can
wake up at any moment. The harness samples system-wide CPU busy once a second for
the whole run, from a single background process (`top` on macOS, `/proc/stat` on
Linux) whose cost is the same for both tools.

1. **Before starting**, the machine must be quiet: mean busy at most
   `BUSY_MARGIN` (10%) and no spike over 30% in the last five seconds, waiting up
   to a minute for a recent build or clone to settle. Otherwise the run stops and
   names the heaviest processes. The 1-minute load average is recorded but not
   used, because it still contains whatever ran just before.
2. **After every timed block**, the samples taken during that block are checked.
   The mean limit is the CPU the benchmark itself used in that block (hyperfine's
   CPU time for both tools, including the git processes GPY spawns, as a
   percentage of all CPUs; never less than 1.5 CPUs) plus the margin. The peak
   limit is the busier tool's own rate, since the tools run one after the other
   and a one-second sample can hold only one of them, plus the margin and 20
   points. A fixed allowance does not work: on a large repository GPY's git
   alone uses about ten cores for ninety milliseconds. A block is also rejected
   if swap use grew by more than 64 MB or, on macOS, the CPU was thermally
   throttled.
3. A rejected block is moved to `raw/contended/` and run again after five
   seconds, up to three times, with the heaviest processes logged.
4. A block that never gets a quiet window is kept but flagged, and its cell's
   verdict is forced to `inconclusive`.

`--force` lets the run start on a busy or battery-powered machine; the per-block
checks still apply and the report says what was flagged.

## Output

Written to `bench-results/compare-starship/<UTC timestamp>/` (git-ignored) or `--out`:

| File | Contents |
|---|---|
| `report.md` | The tables, the exact commands, tool versions and hashes, machine and activity summary |
| `results.json` | All of the above, machine-readable, including every block's machine-activity record |
| `raw/` | One hyperfine export per cell and round; `raw/contended/` holds discarded blocks |
| `cpu-samples.txt` | The per-second CPU samples |
| `repos.lock`, `versions.lock`, `starship.toml` | Copies of the inputs used |

## Reading the result

`ratio` is the GPY median divided by the Starship median, so below 1.00 GPY is
faster. Medians are over all pooled runs of a cell; p95 shows the tail.

The verdict is deliberately stricter than comparing two means. A tool is called
faster only if it has the lower median in **every** round and at least five
rounds ran. With the default six rounds that is a two-sided sign test at
p = 2/2^6 = 0.03. Anything else is reported as `inconclusive`, with the count of
rounds in which GPY was ahead. A cell is also marked `noisy` when one tool's own
round medians differ by more than 10%, and `[other load during measurement]` when
a block could not be run quietly.

Differences of a millisecond or two between processes that each take tens of
milliseconds are within what a laptop can do on its own; treat a result as real
only when the verdict is not `inconclusive`, and then report the ratio and the
report's machine line together.

## Rerunning against a known state

```bash
# Same fixtures, same tool versions as a previous report: use the defaults.
just bench-compare

# Newest releases of both tools, same fixtures.
just bench-compare --gpy latest --starship latest

# Does this branch regress against a baseline commit? Same machine, one after the other.
just bench-compare --gpy ref:main
just bench-compare --gpy local

# Fast sanity check of the harness itself.
just bench-compare --repos tiny-express --scenarios clean --rounds 5 --runs 20 --force
```

Use `--rounds 5 --runs 20` only as a smoke test; the defaults (6 x 100 per tool
per cell) are what the verdicts are designed for.
