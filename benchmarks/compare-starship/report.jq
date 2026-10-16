# Render results.json (written by run.sh) as a Markdown report.
# Usage: jq -r -f report.jq results.json

def fmt:
  if . == null then "n/a" else
  (. * 100 | round) as $x
  | (($x / 100) | floor | tostring) + "."
    + (($x % 100) | tostring | if length < 2 then "0" + . else . end)
  end;

def verdict:
  if .verdict == "gpy" then "GPY faster (\(.rounds_gpy_faster)/\(.rounds) rounds)"
  elif .verdict == "starship" then "Starship faster (\(.rounds - .rounds_gpy_faster)/\(.rounds) rounds)"
  else "inconclusive (GPY ahead in \(.rounds_gpy_faster)/\(.rounds) rounds)"
  end
  + (if .contaminated then " [other load during measurement]" else "" end)
  + (if .noisy then " [noisy: rerun on a quieter machine]" else "" end);

def row:
  .summary as $s
  | "| \(.repo) (\(.files) files) | \(.scenario) "
    + "| \($s.gpy.median_ms | fmt) | \($s.starship.median_ms | fmt) "
    + "| \($s.ratio | fmt) "
    + "| \($s.gpy.p95_ms | fmt) | \($s.starship.p95_ms | fmt) "
    + "| \($s | verdict) |";

"# GPY vs Starship: git status prompt latency",
"",
"Per-invocation wall-clock time of one prompt render, in milliseconds.",
"Ratio is GPY median / Starship median; below 1.00 means GPY is faster.",
"",
"| Repository | Scenario | GPY median | Starship median | Ratio | GPY p95 | Starship p95 | Verdict |",
"|---|---|---:|---:|---:|---:|---:|---|",
(.cells[] | row),
"",
"## Run",
"",
"- GPY: \(.env.gpy.description) (`\(.env.gpy.version)`, sha256 `\(.env.gpy.sha256)`)",
"- Starship: \(.env.starship.description) (`\(.env.starship.version)`, sha256 `\(.env.starship.sha256)`)",
"- Rounds: \(.params.rounds) x \(.params.runs) runs per tool per cell (pooled n = \(.params.rounds * .params.runs)), \(.params.warmup) warmup runs per block",
"- Fixture `core.untrackedCache`: \(.params.untracked_cache) (off = pinned false, on = pinned true for both tools, unset = removed)",
"- Started: \(.env.started_utc); finished: \(.env.finished_utc)",
"- Machine: \(.env.machine.cpu), \(.env.machine.cpus) logical CPUs, \(.env.machine.os) \(.env.machine.arch), power: \(.env.machine.power)",
"- git: \(.env.git.version); hyperfine: \(.env.hyperfine.version)",
"",
"### Commands timed",
"",
"```",
"gpy      \(.commands.gpy)",
"starship \(.commands.starship)",
"```",
"",
"### Machine activity during the run",
"",
"System-wide CPU busy was sampled once a second (\(.env.monitor.sampler)) for the whole run, including the",
"tools' own work. The tools' own CPU use (hyperfine's CPU time for both tools and the git processes GPY spawns) is",
"measured for every block. A block was discarded and re-run when its mean busy exceeded that own use (at least",
"\(.env.monitor.busy_floor_percent)%, i.e. 1.5 CPUs) plus \(.env.monitor.margin_points) points, a single sample exceeded the busier tool's own rate by more than that margin plus 20 points, swap use grew,",
"or the CPU was thermally throttled. A block gets up to \(.env.monitor.max_retries) re-runs; if it never ran quietly it is kept, flagged, and its cell is inconclusive.",
"1-minute load average before / after: \(.env.load.before) / \(.env.load.after) (informational).",
"",
"| Repository | Scenario | Mean busy % | Own busy % | Peak busy % | Blocks | Discarded and re-run | Kept while contended |",
"|---|---|---:|---:|---:|---:|---:|---:|",
(.cells[]
 | "| \(.repo) | \(.scenario) | \(.activity.mean_busy | fmt) | \(.activity.own_busy | fmt) | \(.activity.max_busy | fmt) "
   + "| \(.activity.blocks) | \(.activity.retries) | \(.activity.contended_blocks) |"),
"",
(if ([.blocks[] | select(.reason != "")] | length) == 0
 then "No block was flagged: the machine stayed within the limit throughout."
 else
   "Flagged blocks:",
   "",
   "| Repository | Scenario | Round | Attempt | Outcome | Reason | Heaviest processes (ps %cpu, indicative) |",
   "|---|---|---:|---:|---|---|---|",
   (.blocks[] | select(.reason != "")
    | "| \(.repo) | \(.scenario) | \(.round) | \(.attempt) | \(if .accepted then "kept, flagged" else "discarded, re-run" end) | \(.reason) | \(.heaviest) |")
 end),
"",
"### Fixtures",
"",
"| Name | Commit | Ref | Tracked files |",
"|---|---|---|---:|",
(.fixtures[] | "| \(.name) | `\(.commit)` | \(.ref) | \(.files) |"),
"",
"### Per-round medians (ms)",
"",
"| Repository | Scenario | GPY | Starship |",
"|---|---|---|---|",
(.cells[]
 | "| \(.repo) | \(.scenario) "
   + "| \(.summary.gpy.round_medians_ms | map(fmt) | join(" ")) "
   + "| \(.summary.starship.round_medians_ms | map(fmt) | join(" ")) |")
