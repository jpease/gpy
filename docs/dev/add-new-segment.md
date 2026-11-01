# Adding a New Segment

A short checklist for adding a shell-rendered segment to the Fish prompt (for example a Docker context). To ship a segment outside this repository, see [Build a Segment Plugin](build-segment-plugin.md).

## 1. Decide where the data comes from

| Data source | Approach |
|-------------|----------|
| Environment variable | Shell-only |
| File existence check | Shell-only |
| Command output under ~10 ms | Shell-only |
| Slower command, external API, complex parsing | Agent-assisted |

A shell-only segment runs synchronously on every prompt, so everything it does counts against the prompt budget (`prompt_render`: 20 ms target, 50 ms max in `benchmarks/perf-budgets.json`).

## 2. Write the segment file

Segments live in `fish/segments/<name>.fish` and define two functions, which `fish_prompt` calls for every name in `$__enabled_segments`:

- `segment_<name>_detect`: return 0 to show the segment, 1 to hide it. Keep it fast and fail early.
- `segment_<name>_render`: print the segment, using the `gpy_section_*` helpers from `fish/core/renderer.fish` for consistent delimiters.

```fish
# Docker context segment: shown when the active context is not "default".

function segment_docker_detect
    command -v docker >/dev/null 2>&1; or return 1
    set -l context (docker context show 2>/dev/null)
    test -n "$context" -a "$context" != default
end

function segment_docker_render
    set -l context (docker context show 2>/dev/null)
    gpy_section_start blue white "docker $context"
end
```

Common detect checks:

```fish
command -v docker >/dev/null 2>&1; or return 1   # command exists
test -f .dockerenv; or return 1                  # file exists
test -n "$DOCKER_HOST"; or return 1              # environment variable set
```

## 3. Enable it

Add the segment name to `ui.enabled_segments` in `config.toml`; segments render left to right in that order:

```toml
[ui]
enabled_segments = ["clock", "duration", "language", "directory", "docker", "git"]
```

## 4. Test

1. The segment appears when it should (`docker context use remote-prod`).
2. It disappears when it should (`docker context use default`, or `docker` missing from `PATH`).
3. Colors and icons render correctly in your theme.
4. `time fish_prompt` in an interactive shell stays within the prompt budget.

| Problem | Cause | Fix |
|---------|-------|-----|
| Segment not appearing | `detect` returns 1, or the name is not in `ui.enabled_segments` | Debug `segment_<name>_detect` directly |
| Wrong colors | Theme not loaded | `gpy-agent theme export --shell fish \| source` |
| Slow prompt | Expensive detection | Move the work into the agent |

## Common mistakes

- Not checking that a command exists before running it.
- Expensive work in `detect`, which runs on every prompt.
- Not handling empty output or command failures.
- Leaving stderr unredirected, so errors print into the prompt.

## Related

- [Troubleshooting Guide](../user/troubleshooting.md) - When a segment does not render
- [Benchmarking Methodology](performance/benchmarking.md) - When the prompt is slow
