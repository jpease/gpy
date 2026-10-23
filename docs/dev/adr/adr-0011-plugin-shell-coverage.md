# ADR-0011: Plugin Segments Across Shells

## Status

Proposed.

## Context

GPY is a cross-shell prompt, but community plugin segments render only in Fish. A plugin provides segments through `plugin.toml` and one Fish file per segment (`<plugin-root>/segments/<segment-id>.fish`) that defines `segment_<id>_detect` and `segment_<id>_render` and draws with the public `gpy_section_*` helpers: the `api_version = "v1"` contract in [Plugin Framework](../plugins.md#what-is-stable-in-v1).

- `gpy theme export` writes `__gpy_plugin_segment_file_<token>` for Fish only (`gpy-agent/src/theme/export/plugin_segments.rs:200-208`); `fish/core/init.fish:126-175` sources the file. `bash/core` and `zsh/core` have no loader.
- The Bash and Zsh renderers call `__gpy_segment_<id> is_last prev_bg is_first` in a `$()` subshell and skip names without that function (`bash/core/init.bash:211-220,277-305`, `zsh/core/init.zsh:177-192,244-275`).
- `entry_type = "command"` is reserved and rejected (`gpy-agent/src/plugin/registry.rs:167`).
- The only user-facing statement is one line in [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) opens with "Every shell renders the same segments".

Only Fish users could write a plugin, so the lack of requests from Bash and Zsh users says nothing about demand. This ADR compares the cross-shell designs on five real segments before recommending one:

- **D**: port the loader; Bash, Zsh and Fish load shell-native segment files with the `v1` names.
- **E**: `entry_type = "command"`; a program prints the segment text, run on every prompt.
- **E-cached**: the agent runs the command off the request path, caches it and repaints (ADR-0009 pattern).
- **F**: declarative trigger plus template, rendered by the agent.
- **G**: one POSIX `sh` script per segment plus a small manifest, run by each shell and cached there, with no agent involvement.

**Scope.** The plugin contract for third-party segments. GPY's built-in segments were used below only as a test bench with known-correct output; whether any built-in moves onto the plugin contract is a separate, later decision.

### Prior art

- **Starship custom commands** ([docs](https://starship.rs/config/#custom-commands), read 2026-10-23): a `[custom.<name>]` table with `command`, `when` (a shell command shown on exit 0, or a boolean), `require_repo`, `detect_files`, `detect_folders` and `detect_extensions` (checked in the working directory; `!` marks a negative match), `shell`, `use_stdin`, `os`, `format` with `$output`/`$symbol`/`$style`, and a `description` shown by `starship explain`. Commands obey the global `command_timeout` (500 ms by default) unless `ignore_timeout` is set. Since v1.20 output is escaped; `unsafe_no_escape` opts out, and the docs warn that unescaped output (as before v1.20) can be interpreted by the shell, for example backticks executed. The config has a published JSON schema for editors. Starship's built-in modules are Rust in the core; custom commands are the only user extension (from the docs, not verified in the source).
- **Powerlevel10k** ([README](https://github.com/romkatv/powerlevel10k), [p10k-classic.zsh](https://github.com/romkatv/powerlevel10k/blob/master/config/p10k-classic.zsh), read 2026-10-23): Zsh only. A user segment is a Zsh function `prompt_<name>` that calls `p10k segment -f <fg> -i <icon> -t <text>` (the example near line 1680 of `p10k-classic.zsh`) and can be customized like a built-in (`POWERLEVEL9K_<NAME>_*`); the README's show-on-command example limits `kubecontext` to typed commands matching `kubectl|helm|kubens`. An optional `instant_prompt_<name>` is replayed before Zsh finishes loading and must make the same `p10k segment` calls. Further documentation is in `p10k help segment`. The README states the project has very limited support. Not verified: that user segments accept `SHOW_ON_COMMAND` (inferred from "customized the same way as built-in segments") and the recommended `my_` prefix.
- **What GPY takes from them:** Starship's declarative trigger, timeout, output escaping (with its pre-1.20 behaviour as the warning case), schema and `explain`-style description; Powerlevel10k's show-on-command, as a later option. Powerlevel10k's in-shell function API is the model for D.

### Measurements

macOS (Darwin 27.0.0, arm64, Apple M4 Max, 16 cores), Homebrew Bash 5.3.20, Zsh 5.9.2, Fish 4.9.3, macOS `/bin/bash` 3.2.57. Every run used the `tests/lib/shell_e2e.sh` sandbox: a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR` (the variables `CliTestEnv` sets in `gpy-agent/tests/common/cli_harness.rs`) and a private `GPY_AGENT_SOCKET_PATH`; `find ~/.config/gpy -newer <marker>` listed 0 files after the last run. Prototypes lived in throwaway worktrees, deleted afterwards; no prototype code is committed. The proof of concept below is kept outside the repository for the maintainer at `~/Backups/gpy-adr0011-poc/`: `poc-adapters.patch` (the adapters, against `28f2477`), `examples/` (the five G plugins), `d-examples/`, `e-examples/` and `harness/` (the parity and timing scripts and raw results).

**Baseline: a Fish plugin today.** A plugin `k8s-tools` (`api_version = "v1"`, `entry_type = "file"`) whose `segments/k8s_tools.fish` renders `gpy_section_standalone blue white " PLUGIN_SEGMENT" $is_last`, enabled as `["directory", "k8s_tools", "git"]` with `theme = "text"` (checkout `00e0427`). Each shell ran interactively on a pty; output with escapes stripped. Bash:

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

**Proof of concept: five built-ins as plugins.** `username`, `hostname`, `clock`, `duration` and `directory` were rewritten as plugins three ways and compared with the built-in (checkout `28f2477`):

- **G**: `bin/<name>.sh` plus `segments/<id>.segment.toml` (`script`, `inputs`, `ttl_secs`, `refresh`). The adapter (95 to 118 lines per shell, the D loader included) runs `sh <script>` with the theme's `[segments.<id>]` values as `GPY_OPT_<key>` and the listed shell values as `GPY_PWD`, `GPY_HOME`, `GPY_USER`, `GPY_HOSTNAME`, `GPY_IS_ROOT`, `GPY_IS_SUDO`, `GPY_IS_SSH`, `GPY_CMD_DURATION_MS`. Output: line 1 the text (empty hides the segment), line 2 an optional colour variant (`<variant>_bg_color`/`<variant>_text_color`). One cache entry per shell and segment, keyed by those inputs; a hit reads it with builtins; the theme export changed about 60 Rust lines.
- **D**: `segments/<id>.bash` (sourced unchanged by Zsh as `<id>.zsh` in all five cases) and `segments/<id>.fish`, with the `v1` names; the Bash/Zsh loader adapts the renderer's arguments to `segment_<id>_render is_last is_first`.
- **E**: the G script with `ttl_secs = 0`, run in the prompt every time.

Parity compared every rendered prompt as styled cells (each character with its foreground, background and attributes), so equivalent escape sequences compare equal: 48 cases (users, root, SSH, hidden; host trims; 12/24 h; four durations; ten directory modes including `truncate_to_repo` and `max_length`; two icon cases), in the `default` and `text` themes, three layouts (alone, first, last next to another segment), three shells. G, D and E gave the same result on every comparison: 296 of 432 matched; 117 differ only where the `text` theme renders `duration` and `directory` through a cap-less template (`[$path]($style)`) while a plugin is always drawn as a pill (the visible text is identical); 18 are the icon cases; 1 is a minute rollover between two clock renders.

Timing: release binaries, agent up; each case renders the prompt 30 times per sample (Bash `__gpy_render_prompt` plus `${PS1@P}`, Zsh `__gpy_update_prompts` plus `${(%)PROMPT}`, Fish `fish_prompt` in one process per sample), 32 cases interleaved for 7 rounds; medians in ms per render, Bash / Zsh / Fish; load average 5.4 to 9.2. A first full run (load 7.6 to 12) gave the same order in every cell. "Changing" means a new directory (`directory`) or a new duration (`duration`) before every render. Exec counts are external programs per prompt in steady state, from `PATH` shims (forks that exec nothing are not counted).

| Segment | Approach | Author writes (non-blank lines) | Hit | Changing input | Execs per prompt |
|---|---|---|---|---|---|
| username | built-in | about 202 (Rust 72, shell 80, IPC 50) | 9.27 / 5.70 / 6.53 | — | 1 / 0 / 1 |
| | G | script 10 + manifest 4 | 1.57 / 1.62 / 2.80 | once per session | 0 |
| | D | Bash/Zsh 13 + Fish 16 | 1.37 / 1.54 / 2.26 | — | 0 |
| | E | as G | 7.44 / 6.02 / 6.65 | — | 1 |
| hostname | built-in | about 198 | 9.30 / 3.52 / 6.75 | — | 1 / 0 / 1 |
| | G | 11 + 4 | 1.51 / 1.61 / 2.82 | once per session | 0 |
| | D | 9 + 12 | 1.42 / 1.49 / 2.35 | — | 0 |
| | E | as G | 7.13 / 5.83 / 6.73 | — | 1 |
| directory | built-in | about 346 (Rust 312) | 0.15 / 2.59 / 2.42 | 7.79 / 2.74 / 7.13 | 1 / 0 / 1 per new directory |
| | G | 94 + 4 | 1.60 / 1.70 / 2.76 | 7.36 / 6.15 / 6.64 | 1 per new directory |
| | D | 82 + 104 | 1.38 / 1.71 / 2.35 | 1.52 / 1.62 / 2.44 | 0 |
| | E | as G | 7.29 / 6.91 / 7.03 | 7.81 / 6.40 / 6.69 | 1 |
| duration | built-in | about 193 | 8.33 / 2.96 / 6.41 | 7.70 / 3.10 / 6.61 | 1 / 0 / 1 |
| | G | 16 + 4 | 1.51 / 1.79 / 2.71 | 8.45 / 6.73 / 7.21 | 1 per command |
| | D | 16 + 21 | 1.38 / 1.56 / 2.31 | 1.40 / 1.53 / 2.39 | 0 |
| | E | as G | 8.66 / 7.23 / 7.97 | 8.20 / 6.86 / 7.37 | 1 |
| clock | built-in | about 241 | 9.08 / 3.64 / 6.60 | — | 1 / 0 / 1 |
| | G (`ttl_secs = 0`) | 8 + 4 | 9.63 / 7.84 / 9.00 | — | 2 (`sh`, `date`) |
| | D | 17 + 14 | 1.74 / 2.03 / 3.55 | — | 0 / 0 / 1 (`date`) |
| | E | as G | 9.67 / 8.14 / 9.02 | — | 2 |
| all five | built-in / G / D / E | | 35.88 / 13.13 / 20.67; 15.92 / 11.21 / 10.48; 7.31 / 5.34 / 4.30; 40.04 / 30.76 / 27.37 | | 4 / 0 / 4; 2; 0 / 0 / 1; 6 |

The built-in's cost in the `default` theme is mostly the agent-rendered template: one IPC request per segment per prompt, a `socat` fork in Bash and Fish. The built-in `directory` in Bash hits an in-shell memo.

Behaviour beyond the numbers:

- **Freshness.** D recomputes every prompt. G recomputes when an input changes or the TTL expires; `directory` is keyed by `$PWD` and `$HOME`, so a change that does not move the shell (for example `git init` under `truncate_to_repo`) shows only after a `cd`. `duration` changes with every command, so G runs the script every prompt. A script can print only the time at which it ran, so the G clock needs `ttl_secs = 0`.
- **Live display.** The built-in clock hands the shell a live prompt token (`\D{}`, `%D{}`) and appears in the Zsh and Fish transient prompt; no plugin approach can do either. No approach, built-in included, repaints an idle prompt on a timer.
- **Without the agent.** With the agent disabled and stopped, Bash and Zsh drew all five segments under G, D and E; the built-in lost `directory`. In Fish, disabling the agent turns GPY off for every approach, and a stopped agent is restarted by the integration, so Fish was not testable that way.
- **Bash 3.2.** The G adapter rendered under `/bin/bash` 3.2.57 (non-interactive). D's clock needs `printf '%()T'` (Bash 4.2+) and forks `date` on 3.2.
- **Earlier background.** On a Kubernetes-context and an `$AWS_PROFILE` segment (load 11 to 37), E paid the `kubectl` script (26.7 ms alone) on every prompt, F cost one agent request per segment (+7.8 / +2.5 / +5.3 ms), E-cached matched D on a hit but showed a new directory's value only after a repaint (27-46 ms in Zsh and Fish, next prompt in Bash), kept serving old values with the agent down and after a restart, and needed agent process execution and a `Message` change; G in background mode repainted 30 to 40 ms after the prompt in Zsh and Fish. E-cached was not rebuilt for the five segments: its hit cost is the D/G file-read cost above (reused, not measured here).
- **Trigger cost.** A Starship-style trigger, evaluated by the adapter before the script, was timed on its own (2,000 evaluations, 11 directories deep, 3 file names and 1 folder checked in every ancestor, 2 extensions globbed in the working directory, 1 variable): a miss cost 204 µs in Bash, 175 µs in Zsh and 225-259 µs in Fish, a match 7 levels up 80-120 µs. Each ran with `PATH=/nonexistent`, so no external command was used: Bash `[[ -f ]]`/`[[ -d ]]` and a `nullglob` array, Zsh the same with `(N)` globs, Fish one `path filter` call over all ancestor/name pairs and one glob. A false trigger therefore costs no fork.
- **Escaping.** With `promptvars` (the default), a raw `$(touch …)` and a backtick command in Bash's `PS1` both ran; after GPY's existing `__gpy_prompt_escape` (which escapes `\`, `$` and backticks; Zsh's also `%`) the same text was shown literally and nothing ran, in Bash and in Zsh with `prompt_subst`. Fish printed it literally without escaping.

Not measured: Unicode cluster abbreviation in `directory` (G and D take the first character, not the grapheme cluster, so they probably differ); `read_only`; a script timeout; the trigger inside the full adapter (timed alone above); show-on-command (not built); a theme switch in Bash and Zsh under D or G; many plugin segments at once; Fish without the agent; interactive Bash 3.2 and 4.x; Linux; memory.

### Dogfooding and examples

- **Could run on G at parity:** `username` and `hostname` (static per session; about 6 times cheaper than the template-rendered built-in in Bash and 2 to 3.5 times in Zsh and Fish). Moving them is out of scope here.
- **Run at parity but should not use G:** `directory` and `duration`. Their input changes every directory or command, so G forks as often as E; as D they are fork-free and always fresh.
- **Must stay native:** `clock` (live prompt token, transient prompt).
- **Contract gaps** (what the public contract must expose before built-ins or third parties can rely on it): public names for shell state (`is_root`, `is_sudo`, `is_ssh`, the last command's duration; D read `__gpy_is_*` and `__gpy_cmd_duration`); colour variants (D read `__color_<id>_user_bg`); the glyph mode (a private-use icon over SSH was dropped by the built-in with `show_icons = false` and printed by every plugin); the theme's `format` for a plugin segment (or a stated rule that plugins are pills; the `text` theme differences and the username icon spacing, `⚡jpease` vs `⚡ jpease`, come from this); typed plugin settings (plugin theme values must be strings, `"3"`, `"true"`, and plugins cannot read `config.toml`); live prompt tokens and transient-prompt participation.
- **What the examples teach an author:** how a segment receives settings (`GPY_OPT_*`) and shell values (`GPY_*` inputs), the two-line output protocol, choosing `ttl_secs` and `refresh` (`username`/`hostname`: one run per session; `directory`: per directory; `duration`: per command; `clock`: why a script is the wrong tool), and that GPY draws colours, caps and separators. The G examples needed no GPY internal; the D files did (the gaps above).

## Decision

Proposed, from the measurements:

1. **Interim, whatever the long-term choice: Option B.** Say "plugin segments load in Fish only" in [Build a Segment Plugin](../build-segment-plugin.md), [Plugin + Theme Authoring](../plugin-theme-authoring.md) and the [developer docs index](../README.md); add a "Plugin segments" row to the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) and narrow its opening sentence to the built-in segments; print one line in `gpy plugin new`, `validate` and `list`; and at `gpy enable <segment>` for a plugin segment with a Bash or Zsh integration installed, note that those prompts skip it, with the remediation `gpy disable <segment>` or use Fish.
2. **Long term: a split by segment class, both on one public contract.**
   - **Static or rarely changing segments: G.** One POSIX script and a 4-line manifest serve all three shells, with the hit cost of D and no access to GPY internals; the script runs in a child, so it cannot change the shell's state.
   - **Segments whose input changes every prompt or directory: D.** Shell-native files (one Bash/Zsh file plus a Fish file) stay fork-free and fresh, where G degrades to one `sh` per change. D needs the public names listed under contract gaps; with them, D is the fast path, and today's `.fish` files remain its Fish half.
   - **Live segments (`clock`) stay built in.** No plugin approach reproduces the live prompt token.
   - **Preconditions** for the G and D contract, before either ships:
     - the contract gaps above, closed in the public contract;
     - a declarative trigger in the G manifest, evaluated by the adapter before the script so a false trigger costs no fork: `detect_files` and `detect_folders` from the working directory up to `/`, `detect_extensions` in the working directory only (globbing every ancestor would read every parent directory; Starship also checks only the working directory), `detect_env`, and an optional `when` shell command, which forks and runs only after the detect checks pass (unlike Starship, where any one condition shows the module);
     - a script timeout (Starship's 500 ms default is the reference);
     - an output rule: the adapter strips control characters and escapes what the shell would interpret, on every path. Bash: `\`, `$` and backticks (`\[ \]` are the `\` case); Zsh: the same plus `%`; Fish: nothing beyond control characters. Starship before v1.20 is the cautionary case;
     - discoverability: `gpy plugin validate` checks the manifest against a published JSON schema and prints each segment's `description`, as `starship explain` does.
   - **Later, not measured:** `show_on_command = "kubectl|helm"` in the manifest, showing a segment only while the typed command matches. Zsh can redraw while typing through a line-editor hook (`zle-line-pre-redraw`, `zle reset-prompt`), as Powerlevel10k does; Fish would need a key binding that repaints; Bash's readline offers no supported way to redraw the prompt while typing, so it would apply in Zsh and Fish only.

E is rejected: it costs a process every prompt (6 per prompt for the five segments). E-cached is not pursued: on these segments it would match G's hit cost but needs agent process execution, a `Message` change and cache invalidation, and goes stale without the agent. F is not a plugin format for these segments: four of the five need logic the template engine lacks.

## Alternatives Considered

### Option A

Fish-only, with the existing CLI-reference note. No cost, but it contradicts the cross-shell premise and leaves the silent drop shown in the baseline.

### Option B

Docs and messages, as in Decision item 1. No prompt-time cost and no change to shell scripts, IPC or `v1`. The `gpy enable` note reuses `setup::installed_integrations()`, which only checks that `$XDG_CONFIG_HOME/gpy/<shell>/` holds an entry point (`gpy-agent/src/commands/setup.rs:261-279`), so it can name a shell that is installed but unused. A mitigation, not cross-shell support.

### Option C

A `gpy doctor` warning per enabled plugin segment, from `enabled_plugin_segments` (`doctor.rs:280`) and `installed_integrations()` (`doctor.rs:46`). Keyed on installed integrations it has B's false positive inside a health report; keyed on the current shell (`GPY_SHELL`, then `$SHELL`) it misses a Fish user who also runs Bash. An optional add-on to B only if the `gpy enable` note proves insufficient.

### Option D

Port the loader: the theme export names a `.bash`/`.zsh`/`.fish` file per segment, and the Bash and Zsh loaders adapt the renderer's call to the `v1` `segment_<id>_render is_last is_first` (about 15 lines per shell on top of a public `gpy_section_standalone`). Measured above: the cheapest plugin approach in every cell, fork-free except Fish's `date`, always fresh, works without the agent. Privilege: plugin code is sourced into Bash and Zsh as it is into Fish today. Lost: authors maintain a Bash/Zsh file and a Fish file (the `directory` Fish file was a 104-line rewrite) and today must read GPY internals. `v1`: additive.

### Option E

Activate `entry_type = "command"`. Uncached, the shell runs the program every prompt: measured at 6 to 10 ms per render with one segment and one process per segment; rejected. E-cached (agent runs it off the request path, caches, repaints through SIGURG) was measured on the earlier Kubernetes segment only: hit cost equal to D, the value one repaint late on a new key, stale with the agent down, and approval-gated (external process execution from the agent, a `Message` variant, new path validation, a dependency from `plugin` on `ipc::registry`).

### Option F

Declarative trigger plus template rendered by the agent. Measured earlier at one agent request per segment per prompt. On the five segments: `username` needs a style chosen by root state (the built-in's `format`/`user_format` split); `hostname` needs `trim_at`, a string operation the template engine lacks; `duration` needs arithmetic formatting; `directory` needs the 312-line resolver; `clock` needs the per-shell live token. Each needs a new agent-side data source or transform, which is the built-in again.

### Option G

One portable script per segment, run and cached by each shell (measured above), behind a declarative trigger the shell evaluates without forking. Privilege: the script runs as the user from the shell, like D's code, but in a child `sh` that cannot change the shell's variables or functions; one child per miss; it inherits the shell's environment and `PATH`; the output cap, control-character strip and prompt escaping run after it exits, and the prototype had no timeout. Against the contributor rules' approval list it adds no agent process execution, no `Message`/`Response` change and no `security.rs` change; it adds a branch to the Fish integration's loader, which the maintainer may count as the Fish shell integration contract. Tooling: `gpy plugin new` would scaffold `bin/<id>.sh` and `segments/<id>.segment.toml`; `validate` would check the manifest against the published schema, print each segment's description and could run `sh -n`; the `gpy doctor` speed heuristic would read and could time the script; `gpy plugin list` would show the format. `v1`: additive.

## Consequences

### Positive

- B removes the silent surprise now.
- Plugin segments would render identically in Bash, Zsh and Fish; for static segments from one script, for per-prompt segments fork-free.
- `username` and `hostname` could themselves move to G at parity and at a lower prompt cost than today's template path.

### Negative

- Two plugin formats to document, validate and support, and a public contract to define for shell state, colours, glyph mode and settings first.
- Plugins remain pills: a theme's `format` does not apply to them, and the clock cannot be a plugin.
- B and the split are production code: if accepted, each is tracked as a separate work item.

## References

- [Plugin Framework](../plugins.md); [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) (SIGURG repaint); [ADR-0009](adr-0009-deferred-language-version-probes.md) (commands kept off the request path)
- Starship: [custom commands](https://starship.rs/config/#custom-commands), [prompt configuration](https://starship.rs/config/#prompt) (`command_timeout`, `scan_timeout`), [config schema](https://starship.rs/config-schema.json)
- Powerlevel10k: [README](https://github.com/romkatv/powerlevel10k) (Show on command, Extensible), [config/p10k-classic.zsh](https://github.com/romkatv/powerlevel10k/blob/master/config/p10k-classic.zsh) (user-defined segment example)
