# ADR-0011: Plugin Segments Beyond Fish

## Status

Proposed.

## Context

A community plugin provides segments through `plugin.toml` and a Fish file per segment (`<plugin-root>/segments/<segment-id>.fish`) that defines `segment_<id>_detect` and `segment_<id>_render` and draws with the public `gpy_section_*` helpers. That is the `api_version = "v1"` contract in [Plugin Framework](../plugins.md#what-is-stable-in-v1). Only Fish loads those files:

- `gpy theme export` writes the `__gpy_plugin_segment_file_<token>` variables for Fish only (`gpy-agent/src/theme/export/plugin_segments.rs:200-208`), and `fish/core/init.fish:126-175` sources the file when the segment is enabled. `bash/core` and `zsh/core` have no plugin loader; `bash/segments/` and `zsh/segments/` hold the eight built-ins.
- The Bash and Zsh renderers already walk `ui.enabled_segments` by name and call `__gpy_segment_<id>` with `is_last prev_bg is_first`, dropping any name without such a function (`__gpy_segment_would_render`, `bash/core/init.bash:211-220,277-305`, `zsh/core/init.zsh:177-192,244-248`). The names differ from the v1 contract (`segment_<id>_render`), and only `__gpy_section_standalone` exists there (`bash/core/renderer.bash:73`, `zsh/core/renderer.zsh:74`); there is no Bash or Zsh `gpy_section_start`, `_append` or `_end`.
- `entry_type = "command"` is reserved and rejected by discovery (`gpy-agent/src/plugin/registry.rs:167`, `manifest.rs:190`).
- `gpy doctor` already lists the enabled community plugin segments (`check_plugin_performance`, `gpy-agent/src/commands/doctor.rs:259-328`) and iterates the installed shell integrations (`doctor.rs:46`, `check_shell_integrations` at `doctor.rs:351-375`). `gpy plugin new`, `gpy plugin validate` and the doctor heuristic look only at `segments/<id>.fish` (`commands/plugin.rs:39`, `doctor.rs:315`).
- The user docs say plugin segment files load in Fish only ([CLI Reference](../../user/cli-reference.md#plugin-commands), from jpease/gpy-archive#990). The [shell feature matrix](../../INSTALL.md#shell-feature-matrix) does not mention plugin segments.

Nobody has asked for plugin segments in Bash or Zsh. The owner parked this ADR (decision D2 of jpease/gpy-archive#988) and started it with `go` on jpease/gpy-archive#993.

### Measurements

All runs: macOS (Darwin 27.0.0, arm64, Apple M4 Max, 16 cores), checkout `00e0427`, Homebrew Bash 5.3.20, Zsh 5.9.2, Fish 4.9.3, socat 1.8.1.3, hyperfine 2.0.0. Every shell ran inside the `tests/lib/shell_e2e.sh` sandbox: `shell_e2e_init` exports a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR` (the four variables `CliTestEnv` sets in `gpy-agent/tests/common/cli_harness.rs`) plus `GPY_AGENT_SOCKET_PATH`, starts a private agent on that socket and removes the sandbox on exit. The sandbox path is shortened to `$TMP` below and trailing spaces are trimmed. `find ~/.config/gpy -newer <marker>`, with the marker touched before the first run, listed 0 files after the last. The scripts were scratch files and are not committed; their steps are listed with each result.

**1. A plugin segment under Bash and Zsh.** Setup: the manifest of the plugin in `gpy-agent/tests/plugin_extensibility_e2e_tests.rs`, installed at `$TMP/config/gpy/plugins/k8s-tools/` (`plugin.toml` with `id = "k8s-tools"`, `api_version = "v1"`, `entry_type = "file"`, `provided_segments = ["k8s_tools"]`), with this `segments/k8s_tools.fish`:

```fish
function segment_k8s_tools_detect
    return 0
end

function segment_k8s_tools_render --argument-names is_last
    gpy_section_standalone blue white " PLUGIN_SEGMENT" $is_last
end
```

`config.toml` is `[ui] show_icons = false`, `theme = "text"`, `enabled_segments = ["directory", "k8s_tools", "git"]`, `[agent.supervisor] enabled = false`, with debug binaries first on `PATH` and the agent started by `shell_e2e_start_agent`. Discovery and export:

```text
$ gpy plugin list
...
- k8s-tools 0.1.0 [user] (ready)
  api: v1
  entry: file
  root: $TMP/config/gpy/plugins/k8s-tools
  segments: k8s_tools

$ gpy plugin validate k8s-tools
ok: Valid discovered plugin: k8s-tools 0.1.0 [user]
   root: $TMP/config/gpy/plugins/k8s-tools
   segments: k8s_tools
exit=0

$ gpy-agent theme export --format bash | grep -E 'enabled_segments|k8s'
__enabled_segments="directory k8s_tools git"

$ gpy-agent theme export --format zsh | grep -E 'enabled_segments|k8s'
typeset -g -a __enabled_segments=(directory k8s_tools git)

$ gpy-agent theme export --format fish | grep -E 'enabled_segments|k8s'
set -g __gpy_plugin_segment_file_k8s_tools "$TMP/config/gpy/plugins/k8s-tools/segments/k8s_tools.fish"
set -g __enabled_segments directory k8s_tools git
```

Each shell then ran interactively on a pseudo-terminal (`shell_e2e_spawn_client bash|zsh`: `bash --noprofile --rcfile <rc> -i` and `zsh -d -i` with an rc file that sources this checkout's `bash/gpy.bash` or `zsh/gpy.zsh`). Two commands were typed at the prompt. The transcripts below are the pty output with terminal escapes stripped (`shell_e2e_transcript`); they include stderr, so a warning would show.

Bash:

```text
GPY_E2E_PID=67862

repo main❯echo "segments=[${__enabled_segments[*]}]"
segments=[directory k8s_tools git]

repo main❯type segment_k8s_tools_render __gpy_segment_k8s_tools 2>&1 | grep -c "not found"
2

repo main❯
```

Zsh (the `%` lines are Zsh's `PROMPT_SP` mark; each command line appears twice because the transient prompt redraws it as `❯ <command>` at Enter):

```text
GPY_E2E_PID=68600
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

Fish, for contrast (`fish -i` on a pty with `$TMP/config/fish/` linking this checkout's `fish/` tree, `conf.d/gpy_init.fish` and `functions/`):

```text
Welcome to fish, the friendly interactive shell
Type help for instructions on how to use fish

repo[ PLUGIN_SEGMENT ] main❯
```

Observed: Bash and Zsh receive `k8s_tools` in `__enabled_segments`, define no function for it, and draw `repo main❯` with the segment absent. Neither shell prints a warning and the agent log has no line mentioning the plugin. Fish draws the segment between its neighbours.

**2. What `gpy doctor` says about it.** Same sandbox and plugin, then `gpy setup --shell bash --non-interactive --from <checkout>` and the same for Zsh, agent started:

```text
$ gpy doctor
...
[Configuration]
  ...
  Checking enabled segments... ok: All segments recognized

[Plugins]
  Discovered Plugins:
    ok: gpy-core v0.6.0 [first-party]
    ok: k8s-tools v0.1.0 [user]

[Plugin Performance]
  Enabled community plugin segments:
    - k8s_tools (plugin: k8s-tools, file: $TMP/config/gpy/plugins/k8s-tools/segments/k8s_tools.fish)
      heuristic: no obvious slow patterns detected

[System]
  Checking gpy-agent binary... ok: Found (<checkout>/gpy-agent/target/debug/gpy-agent)
  Checking zsh integration... ok: 0.6.0 ($TMP/config/gpy/zsh)
  Checking bash integration... ok: 0.6.0 ($TMP/config/gpy/bash)

Summary:
ok: All checks passed.
exit=0
```

Doctor knows both facts (an enabled community plugin segment, installed Bash and Zsh integrations) and reports no problem.

**3. Cost of one more segment per render.** Release binaries, agent up, one Bash (`bash --noprofile --norc`) or Zsh (`zsh -f`) process that sources the integration once, then loops the `scripts/ci-bench.sh` render body (`__gpy_precmd; : "${PS1@P}"`, or every `precmd_functions` entry then `: ${(%)PROMPT}`). Each sample is 200 renders timed with `EPOCHREALTIME`; the cases alternate for 7 rounds in the same process; ms per render. A stand-in for a ported plugin segment was defined in the shell itself: `__gpy_segment_k8s_tools() { __gpy_section_standalone blue white " PLUGIN_SEGMENT" "$1" "$3"; }`, a body with no fork of its own, run in the `$()` subshell the renderers use for every segment.

Text theme, `directory git` (A), `directory k8s_tools git` as today (B), and the same with the stand-in defined (C), which drew `repo[ PLUGIN_SEGMENT ] main❯` in both shells:

| Case | Bash min / median / max | Zsh, 5 runs of 200, not interleaved |
|---|---|---|
| A: plugin not enabled | 7.791 / 7.882 / 8.043 | 3.040 to 3.355 |
| B: plugin enabled, dropped (today) | 7.843 / 7.953 / 8.257 | 2.689 to 2.844 |
| C: shell-rendered stand-in | 9.375 / 9.411 / 10.346 | 3.722 to 3.948 |

The two interleaved Zsh runs for this table were unusable: load average 11 to 22 from other work on the machine, medians 7 to 26 ms with maxima to 59 ms. The Bash column is the second interleaved run; the Zsh column is the first run, five sequential samples per case. A second interleaved script with the default theme had a quiet Zsh run; it also timed an agent-rendered segment (H: `directory hostname git` with `__hostname_show_always=1`, so the theme's hostname `format` is rendered by the agent through one IPC request per render, the shape options D and E would take):

| Case, default theme | Bash min / median / max | Zsh min / median / max |
|---|---|---|
| A: `directory git` | 8.065 / 10.717 / 13.843 | 2.803 / 2.880 / 4.484 |
| C: shell-rendered stand-in | 9.624 / 13.437 / 17.308 | 3.794 / 3.814 / 5.338 |
| H: agent-rendered segment | 17.697 / 24.994 / 31.384 | 6.138 / 6.573 / 7.740 |

Load average was 14.7 at the start and 23.6 at the end of that run, so the Bash column is noisy. Read together: an enabled but dropped segment costs nothing (B is within noise of A); a shell-rendered segment costs about 1.5 ms in Bash and 0.9 ms in Zsh; an agent-rendered segment costs about 3.7 ms in Zsh (`zsocket`, no fork) and roughly 9 to 14 ms in Bash, which forks `socat` for every request (`bash/core/ipc.bash:242-243`). The budgets in `tests/performance-baselines.json` are `prompt_render_bash` 37 ms target, 111 ms max and `prompt_render_zsh` 13 ms target, 39 ms max.

**4. Process spawn cost (option D).** `hyperfine -N --warmup 50 --runs 500 /usr/bin/true '/bin/sh -c true'`, three runs: `/usr/bin/true` 1.1 ms ± 0.1, 0.99 ms ± 0.08, 1.7 ms ± 0.8; `/bin/sh -c true` 4.1 ms ± 0.3, 3.8 ms ± 0.2, 5.8 ms ± 1.6 (the last run under the heaviest load). That is the floor before the command does any work; the agent's `prompt_render` budget is 20 ms target, 50 ms max.

**5. Template evaluation cost (option E).** `cargo bench --bench template_bench -- --quick` (the pure `template` engine on Starship's default module formats), criterion middle estimates: `git_branch` 674.71 ns, `git_status_dirty` 820.86 ns, `git_status_clean` 43.206 ns, `character` 334.42 ns, `language_with_version` 733.89 ns, `language_no_version` 429.78 ns, `cmd_duration` 496.15 ns, `full_prompt` 3.2190 µs. Evaluation is three orders of magnitude below the IPC cost in item 3.

Not measured: the runtime of a real plugin command (no representative `command` plugin exists, and the cost belongs to whatever tool it runs); Fish render cost with the plugin; Bash 3.2 or 4.x; Linux. No option was prototyped beyond the stand-in function in item 3.

## Decision

Proposed: **Option B**. Plugin segments stay Fish-only by design, as the user docs already say, and `gpy doctor` warns when an enabled community plugin segment cannot render in an installed Bash or Zsh integration.

The mechanism is one doctor check fed by data doctor already computes: `enabled_plugin_segments(&cfg, &discovery)` (`doctor.rs:280`) and `setup::installed_integrations()` (`doctor.rs:46`). For each installed integration whose shell is Bash or Zsh and each enabled community plugin segment, doctor prints a warning such as `k8s_tools (plugin k8s-tools) loads in Fish only; your bash prompt skips it`, followed by a remediation line pointing at the Fish-only note. It is a warning, not a failure, like `check_shell_integrations`: the configuration is valid and Fish still renders the segment. Nothing changes at prompt time, in the shell scripts, in the IPC protocol or in the `v1` contract.

Rejected mechanisms for the warning: a Bash or Zsh init-time message (the export gives those shells the enabled names and plugin styles but no plugin list or file, so they cannot tell a plugin segment from any other unrendered name, and a message on every new shell is noise); a warning from `gpy enable` alone (it misses edits to `config.toml` and plugins installed after the segment was enabled).

Reopen trigger: a user asks for a plugin segment in Bash or Zsh. Option E is then the first candidate, because it needs no code execution and one plugin file serves every shell; option C if E cannot express the requested segment; option D only with the approval external process execution needs.

## Alternatives Considered

Each option lists its privilege model, its latency (from the Measurements), the work per shell, and its compatibility with `api_version = "v1"` ([Versioning Policy](../plugins.md#versioning-policy)).

### Option A

Fish-only by design, with the user-docs note from jpease/gpy-archive#990 and nothing else.

- Privilege: unchanged. Fish sources plugin files with the user's full privileges; Bash and Zsh run no plugin code.
- Latency: none. An enabled plugin segment is dropped at no measurable cost (item 3, case B).
- Work per shell: none; already shipped.
- `v1`: unchanged.
- Against: the drop stays silent. Measurement 1 shows no warning anywhere, and measurement 2 shows `gpy doctor` reporting "All checks passed" with the segment missing from two installed shells. A user has to find the note in the CLI reference.

### Option B

Option A plus the `gpy doctor` warning described under Decision.

- Privilege: unchanged; doctor reads config, plugin discovery and the integration directories it already reads.
- Latency: none at prompt time; one more loop in `gpy doctor`.
- Work per shell: none in the shell scripts. One Rust check in `commands/doctor.rs` with a unit test, and a "Plugin segments" row in the shell feature matrix.
- `v1`: unchanged.
- Against: an integration sourced from outside `$XDG_CONFIG_HOME/gpy/<shell>/` (a checkout, as in measurement 1) is not detected, so doctor stays silent for it. Users still get no segment in Bash or Zsh.

### Option C

Port the loader to Bash and Zsh. A plugin ships `segments/<id>.bash` and `segments/<id>.zsh` beside the Fish file, each defining per-shell `segment_<id>_detect`/`segment_<id>_render` functions.

- Privilege: Bash and Zsh would source plugin files into the interactive shell with the user's full privileges, as Fish does today. Two more shells run third-party code, and a sourced file can change the user's shell state.
- Latency: the renderers already call every segment in a `$()` subshell, so a segment with no fork of its own adds about 1.5 ms in Bash and 0.9 ms in Zsh (item 3, case C). Whatever the plugin forks comes on top, at every prompt.
- Work per shell, for Bash and Zsh each: export the file variables beyond Fish (`plugin_segments.rs:206`) for the shell's own file extension; a loader in `core/init.*` plus a reload path for theme switches like Fish's `__gpy_ensure_segments_loaded`; an adapter from `segment_<id>_render` to the renderer's `__gpy_segment_<id> is_last prev_bg is_first`; public `gpy_section_start`, `_append`, `_end` and `_standalone` helpers (only `__gpy_section_standalone` exists, with an extra `is_first` argument and output that must wrap SGR in `\[ \]` for Bash or `%{ %}` for Zsh); `gpy plugin new`, `gpy plugin validate` and the doctor heuristic taught the new files; shell suites under `tests/bash` and `tests/zsh`. Plugin authors write every segment three times, in three languages with three escaping rules.
- `v1`: additive if the Bash and Zsh files stay optional; the Fish contract is untouched. Existing plugins remain Fish-only, so coverage becomes per plugin and per shell and `gpy plugin list` would have to show it. The documented helper list does not name a shell, so the Bash and Zsh helper signatures would have to be specified, with `is_first` optional.

### Option D

Activate `entry_type = "command"`: the agent runs a command the manifest declares and the shells display its output, styled by the theme.

- Privilege: new external process execution from the agent daemon, which the project's contributor rules put behind owner approval. The argv comes from a user-writable `plugin.toml` and runs with the user's privileges in the daemon's environment, not the shell's, so per-shell context (`KUBECONFIG`, `AWS_PROFILE`, the working directory) must travel in the request.
- Latency: the spawn alone is about 1 ms, or about 4 ms through `/bin/sh -c` (item 4), before the command does any work; the tool's own runtime was not measured. Run inline, the command would have to finish inside what is left of the 20 ms `prompt_render` target, so it must run off the request path with a cache and repaint, as ADR-0009 does for language version probes. The shell still pays one request per segment per render: about 3.7 ms in Zsh and 9 to 14 ms in Bash (item 3, case H).
- Work per shell: one generic request-and-draw function per shell (Fish, Bash, Zsh); the plugin is written once. Agent side: the runner with timeout, a cache keyed by command, working directory and forwarded environment, a new IPC operation (MINOR per `gpy-agent/SCHEMA_EVOLUTION.md`, but touching `Message`/`Response` needs approval) and an output contract for the command.
- `v1`: `command` is already reserved by `v1` ([Manifest](../plugins.md#manifest-plugintoml)), so activating it is an addition; an older gpy rejects such a plugin with its "unsupported entry_type" diagnostic, the documented failure mode. `min_gpy_version` lets authors state the requirement.

### Option E

Declarative plugins: a manifest declares a trigger (files or directories present from the working directory upward, environment variables set) and a Starship-compatible template, which the agent evaluates with the pure `template` engine (`gpy-agent/src/template/mod.rs`: "Pure: no I/O"). No plugin code runs.

- Privilege: none beyond reading files and the variables the manifest names. A plugin is data.
- Latency: evaluation is under 1 µs per module format (item 5). The cost is the per-segment request, the same as D's: about 3.7 ms in Zsh and 9 to 14 ms in Bash per render (item 3, case H), unless plugin segments ride an existing request.
- Work per shell: one generic request-and-draw function per shell, as for D; the plugin is written once for all shells, Fish included. Agent side: a trigger schema, a resolver like `formatter/hostname_resolver.rs` that feeds the template variables, the per-dialect encoders that already exist (`formatter/style_encoder.rs`), and a new IPC operation carrying the forwarded variables (MINOR, approval-gated as for D).
- `v1`: a new `entry_type` (for example `template`) is an addition under the Versioning Policy; older gpy rejects it with a diagnostic; `file` plugins are untouched.
- Against: it can show only what a trigger and environment variables can express. A segment that needs to parse a tool's state (the current Kubernetes context lives in `~/.kube/config`) needs code, which Starship ships as a built-in module.

## Consequences

### Positive

- `gpy doctor` stops reporting "All checks passed" while an installed Bash or Zsh prompt silently skips an enabled plugin segment.
- No change to prompt latency, the shell scripts, the IPC protocol, the `v1` plugin contract or the privilege model; no external process execution.
- Options C, D and E stay open with the measurements above, and the reopen trigger says which to try first.

### Negative

- Bash and Zsh users still cannot use plugin segments.
- The warning depends on `gpy setup` having installed the integration under `$XDG_CONFIG_HOME/gpy/<shell>/`; an integration sourced from elsewhere gets no warning.
- Implementing the check is production code: if this ADR is accepted, it is filed as its own top-level epic.

## References

- [Plugin Framework](../plugins.md): the `v1` contract
- [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands): the Fish-only note
- [ADR-0006](adr-0006-async-terminal-updates.md) and [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): repaint of idle prompts
- [ADR-0009](adr-0009-deferred-language-version-probes.md): external commands kept off the request path
- `fish/core/init.fish`, `bash/core/init.bash`, `zsh/core/init.zsh`, `gpy-agent/src/theme/export/plugin_segments.rs`, `gpy-agent/src/commands/doctor.rs`, `tests/performance-baselines.json`
- jpease/gpy-archive#993 (this ADR), #988 (assumptions A3, A4), #990 (the user-docs note)
