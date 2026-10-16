# Summarise one cell (repo x scenario). Input (`jq -s`): one hyperfine export
# per round, in order, each with results "gpy" and "starship"; schema v1 or v2.
#
# Verdict (README.md, "Reading the result"): "faster" only with the lower median
# in EVERY round and >= $min_rounds rounds; for six rounds that is a two-sided
# sign test at p = 2/2^6 ~ 0.03. Anything else is "inconclusive".

def walls:
  if has("times") then .times
  else [.measurements[] | .time_wall_clock.value]
  end;

def med:
  sort | length as $n
  | if $n == 0 then null
    elif $n % 2 == 1 then .[($n - 1) / 2]
    else (.[$n / 2 - 1] + .[$n / 2]) / 2
    end;

# Nearest-rank percentile, p in (0, 1].
def pct($p):
  sort | length as $n
  | .[([(($n * $p) | ceil) - 1, 0] | max)];

def mean: add / length;

def sd:
  mean as $m
  | if length < 2 then 0
    else (map((. - $m) * (. - $m)) | add / (length - 1)) | sqrt
    end;

def pooled($tool):
  [.[] | .results[] | select(.name == $tool) | walls[] | . * 1000];

def per_round($tool):
  [.[] | .results[] | select(.name == $tool) | (walls | map(. * 1000) | med)];

def stats($tool):
  pooled($tool) as $t
  | {
      n: ($t | length),
      median_ms: ($t | med),
      mean_ms: ($t | mean),
      stddev_ms: ($t | sd),
      min_ms: ($t | min),
      p95_ms: ($t | pct(0.95)),
      round_medians_ms: per_round($tool)
    };

def spread: (max / min);

# Pass --argjson min_rounds N.
stats("gpy") as $g
| stats("starship") as $s
| ($g.round_medians_ms | length) as $r
| ([range(0; $r)
    | select($g.round_medians_ms[.] < $s.round_medians_ms[.])] | length) as $gpy_faster
| {
    gpy: $g,
    starship: $s,
    ratio: ($g.median_ms / $s.median_ms),
    rounds: $r,
    rounds_gpy_faster: $gpy_faster,
    verdict:
      (if $r < $min_rounds then "inconclusive"
       elif $gpy_faster == $r then "gpy"
       elif $gpy_faster == 0 then "starship"
       else "inconclusive"
       end),
    # >10% drift in a tool's own round medians: the machine changed mid-run.
    noisy: (($g.round_medians_ms | spread) > 1.10
            or ($s.round_medians_ms | spread) > 1.10)
  }
