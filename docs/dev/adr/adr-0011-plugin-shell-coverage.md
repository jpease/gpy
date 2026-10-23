# ADR-0011: Plugin Segments Across Shells

## Status

Proposed.

## Context

GPY is a cross-shell prompt, but community plugin segments render only in Fish. A plugin provides segments through `plugin.toml` and one Fish file per segment (`<plugin-root>/segments/<segment-id>.fish`) that defines `segment_<id>_detect` and `segment_<id>_render` and draws with the public `gpy_section_*` helpers: the `api_version = "v1"` contract in [Plugin Framework](../plugins.md#what-is-stable-in-v1).

- `gpy theme export` writes `__gpy_plugin_segment_file_<token>` for Fish only (`gpy-agent/src/theme/export/plugin_segments.rs:200-208`); `fish/core/init.fish:126-175` sources the file. `bash/core` and `zsh/core` have no loader.
- The Bash and Zsh renderers call `__gpy_segment_<id> is_last prev_bg is_first` in a `$()` subshell and skip names without that function (`bash/core/init.bash:211-220,277-305`, `zsh/core/init.zsh:177-192,244-275`); only `__gpy_section_standalone` exists there.
- `entry_type = "command"` is reserved and rejected (`gpy-agent/src/plugin/registry.rs:167`).
- The only user-facing statement is one line in [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) opens with "Every shell renders the same segments".

Only Fish users could write a plugin, so the lack of requests from Bash and Zsh users says nothing about demand. This ADR therefore compares the cross-shell designs by prototype before recommending one.

### Measurements

macOS (Darwin 27.0.0, arm64, Apple M4 Max, 16 cores), checkout `00e0427`, Homebrew Bash 5.3.20, Zsh 5.9.2, Fish 4.9.3, kubectl on `PATH`. Every run used the `tests/lib/shell_e2e.sh` sandbox: a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR` (the variables `CliTestEnv` sets in `gpy-agent/tests/common/cli_harness.rs`) and a private `GPY_AGENT_SOCKET_PATH`. `$TMP` stands for the sandbox. `find ~/.config/gpy -newer <marker>` listed 0 files after the last run. The prototypes lived in a throwaway worktree that was deleted afterwards; none of their code is committed.

**Baseline: a Fish plugin today.** A plugin `k8s-tools` (`api_version = "v1"`, `entry_type = "file"`, `provided_segments = ["k8s_tools"]`) whose `segments/k8s_tools.fish` renders `gpy_section_standalone blue white " PLUGIN_SEGMENT" $is_last`, enabled as `["directory", "k8s_tools", "git"]` with `theme = "text"`. `gpy plugin validate k8s-tools` printed `ok: Valid discovered plugin`. Each shell ran interactively on a pty (`shell_e2e_spawn_client`); output with escapes stripped. Bash:

```text
repo main❯echo "segments=[${__enabled_segments[*]}]"
segments=[directory k8s_tools git]

repo main❯type segment_k8s_tools_render __gpy_segment_k8s_tools 2>&1 | grep -c "not found"
2

repo main❯
```

Zsh (`%` is the `PROMPT_SP` mark; the transient prompt redraws each command as `❯ <command>`):

```text
%
repo main❯eecho "segments=[${__enabled_segments[*]}]"
❯echo "segments=[${__enabled_segments[*]}]"
segments=[directory k8s_tools git]
%
repo main❯ttype segment_k8s_tools_render __gpy_segment_k8s_tools 2>&1 | grep -c "not found"
❯type segment_k8s_tools_render __gpy_segment_k8s_tools 2>&1 | grep -c "not found"
2
%
repo main❯
```

Fish:

```text
repo[ PLUGIN_SEGMENT ] main❯
```

Bash and Zsh drop the segment with no message from the shell or the agent, and `gpy doctor` ends `ok: All checks passed.` with both integrations installed.

**Prototypes.** Two segments, each built three ways (D, E, F below) as separate plugins: a Kubernetes context segment (the `current-context` of `$KUBECONFIG`, else `~/.kube/config`, shown only when set) and an `$AWS_PROFILE` segment (shown only when set). The sandbox had `~/.kube/config` with `current-context: dev-cluster` and `AWS_PROFILE=staging`. The author's files for the Kubernetes segment follow, with the id shown as `k8s_context` (the prototypes used `k8s_d`, `k8s_e` and `k8s_f`).

D, `segments/k8s_context.fish` (12 lines, today's contract) plus `segments/k8s_context.bash` (19 lines), which was copied byte for byte to `segments/k8s_context.zsh` and worked unchanged there:

```bash
segment_k8s_context_detect() {
    local cfg="${KUBECONFIG%%:*}" line
    [[ -n "$cfg" ]] || cfg="$HOME/.kube/config"
    __k8s_context=""
    [[ -r "$cfg" ]] || return 1
    while IFS= read -r line; do
        if [[ "$line" == current-context:* ]]; then
            line="${line#current-context:}"
            line="${line//[\"\' ]/}"
            __k8s_context="$line"
            break
        fi
    done <"$cfg"
    [[ -n "$__k8s_context" ]]
}

segment_k8s_context_render() {
    gpy_section_standalone "${__color_k8s_context_bg:-}" "${__color_k8s_context_fg:-}" " ⎈ ${__k8s_context}" "$1"
}
```

E, `bin/k8s_context` (2 lines, one file for every shell):

```sh
#!/bin/sh
ctx=$(kubectl config current-context 2>/dev/null) && [ -n "$ctx" ] && printf '⎈ %s\n' "$ctx"
```

F, `segments/k8s_context.toml` (6 lines, one file for every shell, no code):

```toml
format = "([ ⎈ $context ]($style))"
env = ["KUBECONFIG"]

[file_vars.context]
path = "${KUBECONFIG:-~/.kube/config}"
prefix = "current-context:"
```

The `$AWS_PROFILE` segment took 8 lines of Fish plus a 5-line Bash/Zsh file (D), a 2-line script (E), or `format = "([ ☁ $AWS_PROFILE ]($style))"` and `env = ["AWS_PROFILE"]` (F). Every variant rendered the same prompt in Bash, Zsh and Fish (escapes stripped): `repo  ⎈ dev-cluster  ☁ staging   main  ❯`; with `AWS_PROFILE` unset and a kubeconfig without `current-context`, every variant rendered `repo   main  ❯`. With `[agent] enabled = false` and no agent running, D and E still rendered both segments in Bash and F rendered neither (not checked in Zsh or Fish).

**Latency.** Release binaries, agent up, the `scripts/ci-bench.sh` render body (`__gpy_precmd; : "${PS1@P}"`, the Zsh `precmd_functions` plus `${(%)PROMPT}`, `fish_prompt`). Bash and Zsh: one process, 100 renders per sample, the four cases interleaved for 7 rounds. Fish: one `fish -c` process per sample of 100 renders, interleaved the same way, so Fish startup is spread equally over every case. Median ms per render (min to max), base case A = `directory git`, the others add both plugin segments:

| Case | Bash | Zsh | Fish |
|---|---|---|---|
| A, no plugin | 6.90 (6.70-8.04) | 2.50 (2.44-2.58) | 3.26 (2.80-3.62) |
| D, shell file | 9.82 (9.72-10.36) | 4.47 (4.34-4.53) | 3.75 (3.06-10.51) |
| E, command per render | 42.89 (42.35-43.65) | 37.02 (36.53-38.71) | 38.37 (35.89-54.78) |
| F, agent-rendered template | 22.58 (22.43-25.72) | 7.58 (7.25-8.08) | 13.84 (12.25-18.16) |

Load average was 23.5 at the start and 22.8 at the end (16 cores); two earlier full runs (load 11 to 34) ranked the four cases the same way in every shell. Per segment: D adds about 1.5 ms in Bash (one subshell) and 1.0 ms in Zsh; in Fish the difference is within noise (the ranges overlap). E adds the commands' own runtime, measured alone with `hyperfine -N` at 26.7 ms for the kubectl script (20.8 ms for `kubectl config current-context`) and 4.0 ms for the `sh` script. F adds one IPC request per segment: about 7.8 ms in Bash and 5.3 ms in Fish, which fork `socat` per request, and 2.5 ms in Zsh (`zsocket`). The per-render budgets in `tests/performance-baselines.json` are Bash 37 ms target, 111 ms max and Zsh 13 ms target, 39 ms max.
Not measured here: one batched request for all plugin segments under F; F's agent-free fallback; D's theme-switch reload in Bash and Zsh; Bash 3.2/4.x; Linux.

**Cached command (E-cached).** A second prototype, built at the maintainer's request, moves the command off the prompt path the way ADR-0009 moved version probes. A new IPC operation `plugin_cmd` carries the segment, the shell's `$PWD` and the variables the plugin lists. The agent runs one job per (segment, cwd, forwarded variables): it runs the command, writes the first output line to `$XDG_CACHE_HOME/gpy/plugin-cache/<segment>.<cwd key>.<variables key>`, rings the shells in that directory through the existing SIGURG repaint (`notify_repaint_force`, ADR-0007), re-runs every `ttl_secs` while a registered shell is still there, and deletes the entry when none is. The shell's detect function only reads that file with builtins; on a miss it sends one detached request and hides the segment. The command ran with a cleared environment (the agent's `PATH` and `HOME` plus the listed variables), the shell's cwd validated by the existing `SafePath`, no stdin, stderr discarded, a 2 s timeout with kill, output capped at 256 bytes, first line only, control characters stripped; no sandbox and no resource or concurrency limit. For the Kubernetes segment the author writes the 2-line E script above plus `segments/k8s_context.cmd.toml`:

```toml
command = "bin/k8s-context"
env = ["KUBECONFIG"]
ttl_secs = 5
```

A day-of-week segment is `bin/dow` (`date +%a`) plus a 2-line TOML. Under D the same segment needs a different builtin per shell to avoid a fork on every prompt (`printf '%(%a)T'` in Bash, `strftime` in Zsh); Fish has none, so its D file forks `date` every prompt. Measured with a 25 ms `sleep` stand-in and with the kubectl script; same methodology, 100 renders per sample, 7 interleaved rounds, two full runs:

| Measurement | Bash | Zsh | Fish |
|---|---|---|---|
| Cache hit, median ms per render, A / D / E-cached | 7.93 / 9.65 / 9.75; 9.14 / 11.40 / 11.55 | 3.62 / 4.18 / 4.12; 3.15 / 4.53 / 4.33 | 3.69 / 3.69 / 4.12; 4.23 / 4.38 / 4.39 |
| Miss (new directory every render), A → E-cached | 20.96 → 22.01; 23.00 → 25.35 | 10.61 → 11.61; 11.36 → 12.57 | 8.93 → 10.12; 9.90 → 11.08 |
| Value shown after the miss prompt (5 runs each) | at the next prompt | repaint after 27-29 ms (kubectl), 33-38 ms (stand-in) | 34-38 ms, 41-46 ms |
| Context edited in place, TTL 5 s | first prompt drawn after the refresh (2 s) | repaint 4.07 s after the edit | repaint 4.07 s after the edit |
| `KUBECONFIG` exported (new key) | at the next prompt | repaint after 43 ms | repaint after 34 ms |

Load average was 18 to 37 on 16 cores; on a hit, D and E-cached differ by less than the run-to-run spread in every shell, and in Fish A, D and E-cached are all within noise. The commands alone under that load (`hyperfine -N`): kubectl script 40.6 and 46.8 ms, stand-in 39.4 and 49.2 ms, `date +%a` 14.6 and 15.2 ms, all off the prompt path. With the agent stopped, Bash and Zsh kept showing the cached value in an already visited directory (not checked in Fish), and all three shells showed no segment in a new directory and printed nothing. After the agent restarted, Bash and Zsh still showed the old context 8 s after it changed: a hit never contacts the agent, and the prototype has no invalidation at agent start. Not measured for E-cached: invalidation when a watched file such as the kubeconfig changes (not built), many concurrent keys (one thread each), Linux.

## Decision

Proposed, from the measurements:

1. **Interim, whatever the long-term choice: Option B.** Say "plugin segments load in Fish only" in [Build a Segment Plugin](../build-segment-plugin.md), [Plugin + Theme Authoring](../plugin-theme-authoring.md) and the [developer docs index](../README.md); add a "Plugin segments" row to the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) and narrow its opening sentence to the built-in segments; print one line in `gpy plugin new`, `validate` and `list`; and at `gpy enable <segment>` for a plugin segment with a Bash or Zsh integration installed, note that those prompts skip it, with the remediation `gpy disable <segment>` or use Fish.
2. **Long term: Option D.** On a cache hit, E-cached costs the same as D in every shell (within noise), so latency does not separate them. The rest of the evidence favors D: it needs no approval-gated change, while E-cached adds external process execution from the daemon, a `Message` variant, new path validation (the command path, variable names and cache-file names) and a dependency from `plugin` on `ipc::registry`, plus refresh threads the agent's shutdown must own; D is never stale, while E-cached lagged an in-place context change by up to its TTL (4.07 s with 5 s) and kept serving old values with the agent down and after a restart; D draws the segment on the first prompt in a new directory, while E-cached hides it until a repaint 27-46 ms later in Zsh and Fish and until the next prompt in Bash; and D renders without the agent. Its cost is a second author file for Bash and Zsh (the prototype's one file served both), shell-specific knowledge to stay fork-free, and plugin code running in two more shells, as it already does in Fish.

E-cached's measured advantage is for authors: one executable in any language plus a 3-line TOML serves every shell, and the segment can use the tool itself (`kubectl`) instead of reimplementing it in shell. It is the candidate to add later, with maintainer approval, for segments whose state only a tool can compute, once cache invalidation at agent start (and, ideally, on watched-file changes) is designed. Uncached E stays rejected: its cost is the command's runtime on every prompt in every shell, which on its own exceeds the Zsh target. F is the alternative if plugins without code execution matter more than latency and agent-free operation. No hybrid was added: both prototype segments were expressible as D, E-cached and F.

## Alternatives Considered

| Option | Author writes | Code added (non-blank lines in the prototype) | Approval-gated | Same in Fish, Bash, Zsh | Per segment, Bash / Zsh / Fish |
|---|---|---|---|---|---|
| D | Fish file + one Bash/Zsh file | ~22 per Bash/Zsh, 0 Fish; theme export for all shells | No | Yes; agent-free in Bash | 1.5 / 1.0 / noise ms |
| E | one executable | ~13 per Bash/Zsh, ~18 Fish | Yes, external process execution | Yes; agent-free in Bash | command runtime (27 ms kubectl) |
| E-cached | one executable + 3-line TOML | ~34 per Bash/Zsh, ~48 Fish; ~250 Rust | Yes: process execution, `Message`, path validation | Yes; stale or absent without the agent | hit: same as D; first prompt in a new directory: +1 to 2.4 ms, value after a repaint |
| F | one TOML file | ~20 per Bash/Zsh, ~29 Fish; ~240 Rust | Yes, `Message`/`Response` | Yes; needs the agent | 7.8 / 2.5 / 5.3 ms |

### Option A

Fish-only, with the existing CLI-reference note. No change and no cost, but it contradicts the cross-shell premise and leaves the silent drop shown in the baseline.

### Option B

Docs and messages, as in Decision item 1. No prompt-time cost and no change to shell scripts, IPC or `v1`. The `gpy enable` note reuses `setup::installed_integrations()`, which only checks that `$XDG_CONFIG_HOME/gpy/<shell>/` holds an entry point (`gpy-agent/src/commands/setup.rs:261-279`), so it can name a shell that is installed but unused; for one informational line that is acceptable. It is a mitigation, not cross-shell support.

### Option C

A `gpy doctor` warning per enabled plugin segment, from `enabled_plugin_segments` (`doctor.rs:280`) and `installed_integrations()` (`doctor.rs:46`). Keyed on installed integrations it has B's false positive inside a health report; keyed on the current shell (`GPY_SHELL`, then `$SHELL`) it misses a Fish user who also runs Bash. Doctor runs after the surprise. An optional add-on to B only if the `gpy enable` note proves insufficient.

### Option D

Port the loader. The prototype exported the per-shell file path to every shell, sourced it, and defined `__gpy_segment_<id>` as an adapter that passes `is_last` and `is_first` to the author's `segment_<id>_render`, so the `v1` names and arguments are the same as Fish's; a public `gpy_section_standalone bg fg content is_last` wraps the escaping (`\[ \]`, `%{ %}`). The Fish file cannot be reused (different language), but one POSIX-style file served Bash and Zsh. Privilege: plugin code is sourced into Bash and Zsh interactive shells with user privileges, as in Fish today; no agent or IPC change. Lost: authors maintain two files; any fork in plugin code is paid every prompt; `gpy plugin new`, `validate` and doctor read only `.fish` today (`commands/plugin.rs:39`, `doctor.rs:315`). `v1`: additive (new optional files).

### Option E

Activate `entry_type = "command"`, in two variants. Uncached: each shell's segment function runs the plugin executable on every prompt; rejected, since the latency equals the tool's runtime in every shell (kubectl alone 20.8 ms against a 13 ms Zsh target). Cached (E-cached, measured above): the agent runs the command off the request path, caches the first line per (segment, cwd, forwarded variables), repaints through SIGURG and refreshes every TTL. Privilege: the daemon spawns plugin-chosen executables with the user's privileges on every miss and every TTL while a shell sits in that directory, with the agent's `PATH` rather than the shell's. Against the contributor rules' approval list it adds external process execution, changes the IPC `Message` enum (one variant; `Response` reused `Ack`), needs new file path validation (the prototype reused `SafePath` for the cwd without touching `security.rs`, but the command path must stay inside the plugin root, and variable and cache-file names need checks) and adds a cross-module dependency (`plugin` on `ipc::registry`); it did not touch the `tokio::select!` loop. Lost: freshness (up to the TTL; stale while the agent is down and after a restart in the prototype) and the segment on the first prompt in a new directory or environment. `v1`: `command` is already reserved, so activation is additive; the TOML fields are new.

### Option F

Declarative trigger plus template, evaluated by the pure `template` engine. The prototype added an IPC operation `plugin` (segment name plus the forwarded variables the TOML lists), a resolver that adds `style`, `bg` and the separators from the theme, and a per-shell request function; a conditional group `( … )` is the trigger, so an unset variable hides the segment. An env-var or file-presence trigger cannot read the Kubernetes context: that needed one minimal extension, `file_vars`, which takes the rest of the first line starting with `prefix` from the first readable file in a `:`-separated path with `$VAR`, `${VAR:-default}` and `~/` expansion (about 45 Rust lines, no YAML parser). Privilege: no plugin code runs; the agent reads the files the TOML names, and the shell forwards the named variables over the socket; the `Message`/`Response` change is approval-gated. Lost: the segment disappears without the agent; one request per segment per prompt; state that only a tool can produce (or YAML beyond a top-level `key: value` line, or kubeconfig merging beyond the first file that sets the key) is out of reach. `v1`: a new `entry_type` is additive.

## Consequences

### Positive

- B removes the silent surprise now; D would make plugin segments render in Bash and Zsh with the same `v1` names, at about 1 to 1.5 ms per segment, and (in Bash at least) without the agent.

### Negative

- Under D, plugin authors write a second file, need shell-specific builtins to stay fork-free, and plugin code runs in Bash and Zsh shells; E-cached's single-executable authoring is deferred.
- B and D are production code: if accepted, each is tracked as a separate work item.

## References

- [Plugin Framework](../plugins.md); [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) (SIGURG repaint); [ADR-0009](adr-0009-deferred-language-version-probes.md) (commands kept off the request path)
