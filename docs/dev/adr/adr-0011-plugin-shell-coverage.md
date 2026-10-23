# ADR-0011: Plugin Segments Beyond Fish

## Status

Proposed.

## Context

A community plugin provides segments through `plugin.toml` and one Fish file per segment (`<plugin-root>/segments/<segment-id>.fish`). The file defines `segment_<id>_detect` and `segment_<id>_render` and draws with the public `gpy_section_*` helpers. That is the `api_version = "v1"` contract in [Plugin Framework](../plugins.md#what-is-stable-in-v1). Only Fish loads those files:

- `gpy theme export` writes `__gpy_plugin_segment_file_<token>` for Fish only (`gpy-agent/src/theme/export/plugin_segments.rs:200-208`); `fish/core/init.fish:126-175` sources the file when the segment is enabled. `bash/core` and `zsh/core` have no loader.
- The Bash and Zsh renderers walk `ui.enabled_segments` and call `__gpy_segment_<id> is_last prev_bg is_first` in a `$()` subshell, skipping any name without that function (`bash/core/init.bash:211-220,277-305`, `zsh/core/init.zsh:177-192,244-275`). Their names differ from the `v1` contract, and only `__gpy_section_standalone` exists there (`bash/core/renderer.bash:73`, `zsh/core/renderer.zsh:74`).
- `entry_type = "command"` is reserved and rejected (`gpy-agent/src/plugin/registry.rs:167`).
- The only user-facing statement is one line in [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands). The authoring guides ([Build a Segment Plugin](../build-segment-plugin.md), [Plugin + Theme Authoring](../plugin-theme-authoring.md), the [developer docs index](../README.md)) do not say it, and the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) opens with "Every shell renders the same segments".

Nobody has asked for plugin segments outside Fish. The problem worth fixing is the silent surprise measured below, not a missing feature.

### Measurements

Run on macOS (Darwin 27.0.0, arm64, Apple M4 Max), checkout `00e0427`, Homebrew Bash 5.3.20, Zsh 5.9.2, Fish 4.9.3. Every shell ran in the `tests/lib/shell_e2e.sh` sandbox: `shell_e2e_init` exports a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR` (the four variables `CliTestEnv` sets in `gpy-agent/tests/common/cli_harness.rs`) and `GPY_AGENT_SOCKET_PATH`, and runs a private agent. `$TMP` stands for the sandbox. `find ~/.config/gpy -newer <marker>`, touched before the first run, listed 0 files after the last.

**Observed behavior.** A plugin `k8s-tools` (manifest as in `gpy-agent/tests/plugin_extensibility_e2e_tests.rs`: `api_version = "v1"`, `entry_type = "file"`, `provided_segments = ["k8s_tools"]`) at `$TMP/config/gpy/plugins/k8s-tools/`, whose `segments/k8s_tools.fish` renders with `gpy_section_standalone blue white " PLUGIN_SEGMENT" $is_last`. `config.toml`: `theme = "text"`, `show_icons = false`, `enabled_segments = ["directory", "k8s_tools", "git"]`. `gpy plugin validate k8s-tools` printed `ok: Valid discovered plugin: k8s-tools 0.1.0 [user]`. The Bash and Zsh theme exports carry only the name (`__enabled_segments="directory k8s_tools git"`); the Fish export adds `__gpy_plugin_segment_file_k8s_tools`. Each shell then ran interactively on a pty (`shell_e2e_spawn_client`, sourcing this checkout's `bash/gpy.bash` or `zsh/gpy.zsh`); the transcripts are the pty output, stderr included, with terminal escapes stripped.

Bash:

```text
repo main❯echo "segments=[${__enabled_segments[*]}]"
segments=[directory k8s_tools git]

repo main❯type segment_k8s_tools_render __gpy_segment_k8s_tools 2>&1 | grep -c "not found"
2

repo main❯
```

Zsh (`%` is Zsh's `PROMPT_SP` mark; the transient prompt redraws each command as `❯ <command>`):

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

Fish, for contrast (`fish -i` with `$TMP/config/fish/` linking this checkout's `fish/` tree):

```text
repo[ PLUGIN_SEGMENT ] main❯
```

Bash and Zsh drop the segment with no message from the shell or the agent. After `gpy setup --non-interactive` for Bash and Zsh in the same sandbox, `gpy doctor` listed `k8s_tools` under "Enabled community plugin segments", printed `Checking zsh integration... ok` and `Checking bash integration... ok`, and ended `ok: All checks passed.`

**Costs, relevant only to the rejected options D to F.** Release binaries, agent up; one shell process looping the `scripts/ci-bench.sh` render body, 200 renders per sample, cases interleaved for 7 rounds; medians, ms per render. "Shell-rendered" is a stand-in `__gpy_segment_k8s_tools` that calls `__gpy_section_standalone` with no fork of its own; "agent-rendered" is `hostname` with the default theme's `format`, one IPC request per render.

| Cost | Bash | Zsh |
|---|---|---|
| One shell-rendered segment (D) | +1.5 ms (7.88 to 9.41) | within noise |
| One agent-rendered segment (E, F) | +9 to 14 ms, a `socat` fork per request | +3.7 ms (2.88 to 6.57) |
| Process spawn, `hyperfine -N` (E) | `/usr/bin/true` 1.0 to 1.7 ms; `/bin/sh -c true` 3.8 to 5.8 ms | same |
| Template evaluation, `cargo bench --bench template_bench -- --quick` (F) | 43 ns to 0.82 µs per module format, 3.2 µs full prompt | same |

The machine was loaded (load average 11 to 24 on 16 cores) during these runs, so the Bash agent-rendered figure is a range and the Zsh shell-rendered delta is within noise. An enabled-but-dropped segment cost nothing measurable in either shell. Not measured: a real plugin command's runtime, Fish render cost, Bash 3.2/4.x, Linux.

## Decision

Proposed: **Option B**. Plugin segments stay Fish-only, and every place a user or author meets a plugin says so:

- Docs: a "Plugin segments load in Fish only" statement in [Build a Segment Plugin](../build-segment-plugin.md), [Plugin + Theme Authoring](../plugin-theme-authoring.md) and the plugin entries of the [developer docs index](../README.md); a "Plugin segments" row in the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) (Fish: loaded; Zsh and Bash: not loaded, the segment is skipped), with its opening sentence narrowed to the built-in segments.
- Messages: one line in the output of `gpy plugin new`, `gpy plugin validate` and `gpy plugin list` saying plugin segments render in Fish only; and at `gpy enable <segment>`, when the segment comes from a plugin and a Bash or Zsh integration is installed, a note that those prompts skip it, with the remediation `gpy disable <segment>` or use Fish.

This is the cheapest change that removes the silent surprise. It adds no prompt-time work and changes no shell script, IPC message or `v1` contract. Option C's doctor warning is an optional add-on, taken only if the `gpy enable` message proves insufficient (for example, users who edit `config.toml` directly still report the surprise).

Reopen trigger: a plugin author or user asks for a plugin segment in Bash or Zsh. Then try F first (no code execution, one plugin file for every shell), D second if F cannot express the segment, and E only with the maintainer approval external process execution needs.

## Alternatives Considered

Each option covers privilege model, latency, work per shell, and `api_version = "v1"` compatibility ([Versioning Policy](../plugins.md#versioning-policy)).

### Option A

Fish-only, with the existing CLI-reference note and nothing else. Privilege, latency and `v1` unchanged; no work. It is a defensible position for a feature nobody has asked for, but the note sits in one place authors do not read first, the feature matrix contradicts it, and the measurements show a user enabling a valid plugin segment in Bash or Zsh gets no signal at all.

### Option B

Docs and messages, as under Decision. Privilege and `v1` unchanged; no prompt-time latency. Work per shell: none in the shell scripts; four doc edits and messages in `commands/plugin.rs` and `commands/segments.rs`. The `gpy enable` note reuses `setup::installed_integrations()`, which checks only that `$XDG_CONFIG_HOME/gpy/<shell>/` holds an entry point (`gpy-agent/src/commands/setup.rs:261-279`), so it may fire for a shell that is installed but unused; for a one-time informational line that is acceptable.

### Option C

Option A plus a `gpy doctor` warning for each enabled community plugin segment, fed by `enabled_plugin_segments` (`doctor.rs:280`) and `setup::installed_integrations()` (`doctor.rs:46`). Privilege, latency and `v1` unchanged; one Rust check. Keyed on installed integrations, it has the false positive above, and doctor reports health, so a warning about a shell the user never starts reads as a defect; restricting it to the current shell (`GPY_SHELL`, then `$SHELL`, as `setup::detect_shell` does) removes that but misses a user who runs `gpy doctor` from Fish and also uses Bash. Doctor is also run only when something already looks wrong, which is after the surprise.

### Option D

Port the loader to Bash and Zsh: plugins ship `segments/<id>.bash` and `segments/<id>.zsh` with per-shell `segment_<id>_*` functions. Privilege: Bash and Zsh would source third-party files into the interactive shell with full user privileges, as Fish does. Latency: about 1.5 ms per segment in Bash for the subshell alone, plus whatever the plugin forks, every prompt. Work per shell: export the file variables beyond Fish, a loader and theme-switch reload path, an adapter from `segment_<id>_render` to `__gpy_segment_<id>`, public `gpy_section_*` helpers with shell-specific escaping (`\[ \]`, `%{ %}`), tooling that today reads only `.fish` (`commands/plugin.rs:39`, `doctor.rs:315`), and shell suites; authors write each segment three times. `v1`: additive if the extra files are optional; coverage becomes per plugin and per shell.

### Option E

Activate `entry_type = "command"`: the agent runs a manifest-declared command and the shells show its output. Privilege: new external process execution from the daemon, which needs maintainer approval; per-shell context (`KUBECONFIG`, working directory) must travel in the request. Latency: 1 to 6 ms to spawn before the command does any work, so it must run off the request path with a cache and repaint (ADR-0009); each render still pays one request per segment. Work per shell: one generic request-and-draw function in each of Fish, Bash and Zsh; an agent runner, a cache and a new IPC operation (MINOR in `gpy-agent/SCHEMA_EVOLUTION.md`, approval-gated for `Message`/`Response`). `v1`: `command` is reserved in `v1`, so activating it is additive; older gpy rejects such plugins with a diagnostic.

### Option F

Declarative plugins: a trigger (files present upward from the working directory, environment variables set) plus a Starship-compatible template evaluated by the pure `template` engine. No plugin code runs. Privilege: none beyond reading the files and variables the manifest names. Latency: evaluation under 1 µs; the cost is the per-segment request, as for E. Work per shell: one generic request-and-draw function per shell; an agent trigger schema, a resolver like `formatter/hostname_resolver.rs`, the existing per-dialect encoders, and a new IPC operation (approval-gated as for E); one plugin file serves every shell. `v1`: a new `entry_type` is additive under the Versioning Policy. It cannot show state that needs parsing a tool's files (the Kubernetes context lives in `~/.kube/config`).

## Consequences

### Positive

- Bash and Zsh users learn at `gpy enable`, in the feature matrix and in the plugin docs that plugin segments do not render there, and how to undo it; authors learn it before they write a plugin.
- No change to prompt latency, shell scripts, IPC, the `v1` contract or the privilege model.

### Negative

- Bash and Zsh users still cannot use plugin segments.
- A user who edits `config.toml` directly never sees the `gpy enable` message.
- The message and output changes are production code: if accepted, they are tracked as a separate work item.

## References

- [Plugin Framework](../plugins.md); [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); [ADR-0009](adr-0009-deferred-language-version-probes.md) (commands kept off the request path)
