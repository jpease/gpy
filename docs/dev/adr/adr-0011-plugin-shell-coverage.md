# ADR-0011: Plugin Segments Across Shells

## Status

Accepted (2026-10). B now; G as GPY's cross-shell script-segment format, a core feature.

Amended (2026-10) after an adversarial review of the implementation plan: G is a core feature that may also power one or two built-ins; its cache is on by default with a short TTL and lives in shell variables; the trust model, output hardening, timeout constraints, trigger naming, performance budgets and the approval boundaries the work crosses are now explicit. The amendment supersedes the earlier text of Decision items 2 and 3 and the Option G and Consequences text that followed from it.

Amended (2026-10, after Spike 2): a timeout, placement and scan spike settled what Decision items 3, 5, 6, 7 and 8 left open. The runner is pure shell; scripts get a clean environment; output is sanitized once at the runner boundary; the cache is keyed on declared inputs; the timeout, kill rules, circuit breaker and scan bound are decided and measured. The earlier text claiming detect already runs in the parent shell was wrong for Zsh and is replaced.

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
- **G**: one POSIX `sh` script per segment plus a small manifest, run by each shell (cached there), with no agent involvement.

**Scope.** The segment-script contract, for third-party plugins and for GPY's own segments. The built-ins were first used below as a test bench with known-correct output; the amendment makes moving one or two of them onto G part of this decision (Decision item 2).

**Project status.** GPY has not been announced and has no users. Breaking changes are acceptable: a MAJOR schema change, changes to the `v1` manifest, no compatibility shims, and no concern for an older GPY loading a newer plugin or theme.

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

- **Freshness (cached G, as prototyped).** D recomputes every prompt. The prototype's G recomputes when an input changes or the TTL expires; `directory` is keyed by `$PWD` and `$HOME`, so a change that does not move the shell (for example `git init` under `truncate_to_repo`) shows only after a `cd`. `duration` changes with every command, so G runs the script every prompt. A script can print only the time at which it ran, so the G clock needs `ttl_secs = 0`. An earlier G run (a Kubernetes-context stand-in edited in place, TTL 5 s, one Enter per second) showed the new value 3.1 / 4.4 / 3.2 s after the edit in Bash / Zsh / Fish; with the script in the background, an idle Zsh or Fish prompt still showed the old value 7 s after the edit, until the next Enter.
- **Live display.** The built-in clock hands the shell a live prompt token (`\D{}`, `%D{}`) and appears in the Zsh and Fish transient prompt; no plugin approach can do either. No approach, built-in included, repaints an idle prompt on a timer.
- **Without the agent.** With the agent disabled and stopped, Bash and Zsh drew all five segments under G, D and E; the built-in lost `directory`. In Fish, disabling the agent turns GPY off for every approach, and a stopped agent is restarted by the integration, so Fish was not testable that way.
- **Bash 3.2.** The G adapter rendered under `/bin/bash` 3.2.57 (non-interactive). D's clock needs `printf '%()T'` (Bash 4.2+) and forks `date` on 3.2.
- **Earlier background.** On a Kubernetes-context and an `$AWS_PROFILE` segment (load 11 to 37), E paid the `kubectl` script (26.7 ms alone) on every prompt, F cost one agent request per segment (+7.8 / +2.5 / +5.3 ms), E-cached matched D on a hit but showed a new directory's value only after a repaint (27-46 ms in Zsh and Fish, next prompt in Bash), kept serving old values with the agent down and after a restart, and needed agent process execution and a `Message` change; G in background mode repainted 30 to 40 ms after the prompt in Zsh and Fish. E-cached was not rebuilt for the five segments: its hit cost is the D/G file-read cost above (reused, not measured here).
- **Trigger cost.** A Starship-style trigger, evaluated by the adapter before the script, was timed on its own (2,000 evaluations, 11 directories deep, 3 file names and 1 folder checked in every ancestor, 2 extensions globbed in the working directory, 1 variable): a miss cost 204 µs in Bash, 175 µs in Zsh and 225-259 µs in Fish, a match 7 levels up 80-120 µs. Each ran with `PATH=/nonexistent`, so no external command was used: Bash `[[ -f ]]`/`[[ -d ]]` and a `nullglob` array, Zsh the same with `(N)` globs, Fish one `path filter` call over all ancestor/name pairs and one glob. That in-shell scan is not safe, though: Spike 2 showed a `stat` on a hung NFS mount blocks every in-shell design for more than 12 s and ignores Ctrl-C, so `require_files`/`require_folders` run in a bounded child (item 6), and a false trigger on those conditions costs one fork (+0.6 to +0.9 ms per scan on Linux; macOS not measured). `require_extensions` and `require_env` stay fork-free.
- **Escaping.** With `promptvars` (the default), a raw `$(touch …)` and a backtick command in Bash's `PS1` both ran; after GPY's existing `__gpy_prompt_escape` (which escapes `\`, `$` and backticks; Zsh's also `%`) the same text was shown literally and nothing ran, in Bash and in Zsh with `prompt_subst`. Fish printed it literally without escaping. A pattern strip is not locale-independent: on `a`, U+009B (bytes `c2 9b`), `b`, Bash 5.3's `${x//[[:cntrl:]]/}` removed the C1 character under `en_US.UTF-8` and kept it under `LC_ALL=C`, and Fish 4.9.3's `string replace -ra '[[:cntrl:]]'` kept it (run 2026-10-23 for the amendment).
- **Timeout constraints** (run 2026-10-23 for the amendment unless marked). Bash 3.2.57 `read -t 0.5` fails with "invalid timeout specification"; Fish 4.9.3 `read` rejects `--timeout` and `-T`; macOS has no `timeout(1)` on the default `PATH`. Killing a timed-out `sh` does not bound the prompt: `$(sh -c 'sleep 2; echo late' & … kill)` killed `sh` at 0.2 s and the substitution still returned after 2.06 s, because the orphaned `sleep` held the pipe. Spike 2 built bounded runners in every shell anyway (below).
- **Spike 2 (2026-10).** Hardened pure-shell runners (one per shell: Bash 3.2+, Zsh, Fish) and a Rust helper prototype were run against 18 hostile scripts (hangs, process trees, TERM-ignoring children, terminal readers, `setsid` escapees, 50 MB output, NUL, forged framing) on macOS, Debian (dash) and Alpine (busybox) with Bash 3.2, 4.0, 4.1, 4.4, 5.2 and 5.3, Zsh 5.9 and Fish 4. Both runners were bounded: Enter to prompt p99 530-571 ms at T = 500 ms and a 50 ms grace, against a T + 50 + 30 ms limit (one of 900 macOS samples at 580.3 ms under load 20-43). Bash 3.2 is bounded sub-second (a timer job in its own process group). Ctrl-C gave a prompt within 105 ms p99 in every shell, the tree killed. The terminal's foreground group, `$!`/`$last_pid` and descriptor counts never changed, and both runners created 0 files. Per uncached run on macOS (load about 2.4-2.7), the pure-shell runner with the `/bin/sh` launcher skipped costs −1.85 to +1.16 ms against `$(sh script)`, and a clean environment adds +0.07 to +0.30 ms over the unmodified runner; on Linux the runner adds +0.4 to +0.9 ms and cleaning +0.04 to +0.18 ms. Survivors: only a `setsid`/daemonizing escapee; in a container whose PID 1 does not reap, orphans become zombies. Placement was measured in GPY's real hooks with a stub runner: one run for the first prompt and one per command; none on an empty Enter, Ctrl-C, the SIGURG repaint, SIGWINCH or the transient prompt.

Not measured: Unicode cluster abbreviation in `directory` (G and D take the first character, not the grapheme cluster, so they probably differ); `read_only`; the real runner inside GPY's hooks with several segments sharing one deadline; the macOS scan-child cost; the trigger inside the full adapter (timed alone above); show-on-command (not built); a theme switch in Bash and Zsh under D or G; many plugin segments at once; Fish repaints per prompt in a git repository; Fish without the agent; Bash 4.2, 4.3, 5.0 and 5.1; WSL; memory.

### Dogfooding and examples

- **Will run on G (dogfooding):** `username` and `hostname` (static per session; on a cache hit about 6 times cheaper than the template-rendered built-in in Bash and 2 to 3.5 times in Zsh and Fish, from the table above). Decision item 2 takes one or both of them onto G.
- **Run at parity on G at E's cost:** `directory` and `duration`. Their input changes every directory or command, so G runs `sh` on each such prompt, cached or not; as D they are fork-free and always fresh, which is the case D is kept for. They stay built in.
- **Must stay native:** `clock` (live prompt token, transient prompt).
- **Contract gaps** (what the public contract must expose before built-ins or third parties can rely on it): public names for shell state (`is_root`, `is_sudo`, `is_ssh`, the last command's duration; D read `__gpy_is_*` and `__gpy_cmd_duration`); colour variants (D read `__color_<id>_user_bg`); the glyph mode (a private-use icon over SSH was dropped by the built-in with `show_icons = false` and printed by every plugin); the theme's `format` for a plugin segment (or a stated rule that plugins are pills; the `text` theme differences and the username icon spacing, `⚡jpease` vs `⚡ jpease`, come from this); typed plugin settings (core config and built-in themes are typed, but a plugin's `[segments.<id>]` values are an undeclared string map, so `"3"` and `"true"`, and plugins cannot read `config.toml`); live prompt tokens and transient-prompt participation.
- **What the examples teach an author:** how a segment receives settings (`GPY_OPT_*`) and shell values (`GPY_*` inputs), the two-line output protocol, choosing `ttl_secs` and `refresh` (`username`/`hostname`: one run per session; `directory`: per directory; `duration`: per command; `clock`: why a script is the wrong tool), and that GPY draws colours, caps and separators. The G examples needed no GPY internal; the D files did (the gaps above).

## Decision

Accepted by the maintainer (2026-10); amended the same month (see Status). Accepting the amendment approves the direction only: every implementing PR that crosses an autonomy boundary (item 9) still gets explicit maintainer review at that point.

1. **Now: Option B.** Say "plugin segments written as Fish files (`entry_type = "file"`) load in Fish only" in [Build a Segment Plugin](../build-segment-plugin.md), [Plugin + Theme Authoring](../plugin-theme-authoring.md) and the [developer docs index](../README.md) ([Plugin Framework](../plugins.md) already says Fish-only); add a row for them to the [shell feature matrix](../../INSTALL.md#shell-feature-matrix) and narrow its opening sentence to the built-in segments; print one line in `gpy plugin new`, `validate` and `list`; and at `gpy enable <segment>` for such a segment with a Bash or Zsh integration installed, note that those prompts skip it, with the remediation `gpy disable <segment>` or use Fish. Every message names `v1` `.fish` segments, so it stays true when G lands; the G adapter work then narrows the docs by adding that script segments render in all three shells.
2. **G is a core feature of GPY.** One POSIX script and a manifest per segment serve all three shells with no GPY internal and no agent involvement. It is the cross-shell plugin format and also an engine for GPY's own segments:
   - **Dogfooding, one or two built-ins at most:** `username` and/or `hostname`, shipped with GPY as G segments. Acceptance bar: the PoC parity method (styled cells, `default` and `text` themes, three layouts, three shells) with no difference left except ones the contract-gap work has closed, and a render no slower than the built-in it replaces in each shell. `directory` and `duration` stay built in (D-class: fork-free and fresh every prompt); the clock stays native. D is not chosen: it stays a measured, documented alternative, and no D loader is committed.
   - **Plugin identity.** A G plugin has its own `entry_type` (for example `"script"`; `EntryType` has only `file` and the reserved `command` today, `plugin/manifest.rs:171-176`). With `entry_type = "file"`, `validate_plugin_root` requires `segments/<id>.fish` for every segment (`commands/plugin.rs:66-77`); the G branch checks the segment manifest and the contained script instead. `min_gpy_version` (`manifest.rs:298`) is today only a printed warning in `gpy plugin validate` (`commands/plugin.rs:167-182`); it becomes enforced: export and `validate` reject a plugin whose minimum exceeds the running GPY.
3. **Cache on by default, short TTL, in shell variables.**
   - **Once per command, not per draw.** A G segment is **due** on the first prompt, after a new command, or when `PWD` changed since its last run; then it runs if its TTL expired or its key changed. It is never due on a repaint, an empty Enter, Ctrl-C, the SIGURG doorbell (`fish/core/ipc.fish:1212-1225,1250-1252`) or SIGWINCH. A Ctrl-C during a run hides the segment for that prompt without rerunning it.
   - **The author sets `ttl_secs` in the segment manifest; default 5.** A hit cost 1.5 to 1.6 / 1.6 to 1.8 / 2.7 to 2.8 ms against 7.1 to 8.7 / 5.8 to 7.2 / 6.7 to 8.0 ms for a one-process run (PoC G "Hit" and E rows, reused, not re-run), and at one Enter per second a TTL of 5 s showed an edited value 3.1 to 4.4 s later (Measurements): staleness of about one or two commands, for a hit that costs a fifth to two fifths of a run. `ttl_secs = 0` opts out (one run per command that passes the trigger), for segments that must be current; a segment whose output depends only on its inputs (`username`, `hostname`) sets a TTL longer than a session. The user overrides `ttl_secs` per segment (item 4). A change outside the key shows only after the TTL, and only when a prompt is drawn; nothing repaints an idle prompt.
   - **Placement.** Runs happen in the interactive shell, never in a render subshell: Bash in `__gpy_precmd` before `__gpy_render_prompt`; Zsh in `__gpy_precmd` before `__gpy_update_prompts`, with `__gpy_preexec` setting the due flag and a local `TRAPINT`; Fish in an `--on-event fish_prompt` handler, with a `fish_postexec` handler that only sets the due flag (a run inside the `fish_prompt` function drew no prompt after Ctrl-C). Zsh renders inside `$(__gpy_render_prompt …)`, so a guard or cache set there is lost with the subshell; the render subshell and repaints only read the results. Results are known before the render loop, so `_detect` is a fork-free "result is non-empty" test and `is_first`/`is_last` are fixed correctly.
   - **Storage and key.** The guard and the cache are shell variables in the interactive shell: no files of any kind, nothing outlives the shell. The key holds the plugin id and version, the segment id, the script path, the resolved `GPY_OPT_*` values and the values of the **declared inputs** only. Scope follows from them: **session** when `pwd` is not declared (`username`/`hostname` run once per session), **folder** when it is. There is no device scope; it would need a file. The age comes from a clock in the same shell.
   - **Sanitize once.** The runner caps, strips and escapes the output before any shell variable holds it (item 5 and the script segment security addendum to this ADR); the cache holds the render-ready value and raw output is never stored.
   - **E stays rejected.** E runs a program on every prompt with no trigger and no cache; G's default caches, skips prompts where the trigger fails, and never runs more than once per command. `ttl_secs = 0` is E behind a trigger, chosen per segment and paid against the budget (item 7).
4. **Typed plugin settings and the user override.** Core `config.toml` and the built-in segment themes are typed (serde with validating newtypes; `[segments.duration] show_if_exceeds_ms` is a `u64`). Plugin segments are not: unknown `[segments.<id>]` tables land in `SegmentThemes.plugin` (`gpy-agent/src/theme/model/mod.rs:65-67`, a serde flatten catch-all) and their extra keys in `PluginSegmentTheme.properties: HashMap<String, String>` (`mod.rs:88-90`), so a non-string value cannot deserialize and nothing declares a setting's type, range or default.
   - The segment manifest declares each setting: `bool`, `int` or `string`, a default, a range or choices, and a description, the shape of a `ConfigKeyMeta` entry in `CONFIG_KEYS` (`config/metadata.rs:23-56`) plus a type. `ttl_secs` is a reserved declared setting. Values reach the script as strings; the gain is validation at load, naming the key, as core config does. `gpy plugin validate` checks the manifest against a published JSON schema with a drift test.
   - **The user override lives in `config.toml`, in a new per-segment table** (name fixed in implementation). Settings are behaviour, not style: in the theme they would be lost on every theme switch and could be changed only by copying a whole theme (`gpy theme new`). `Config` has no per-segment table today (`agent`, `git`, `language`, `ui`; `config/mod.rs:40`), so this alters configuration schema structure. Theme `[segments.<id>]` keeps style keys only.
   - **MAJOR** under `gpy-agent/SCHEMA_EVOLUTION.md` (field type change): settings leave the theme's string map and become typed values elsewhere. Acceptable before announcement; no shim, and older GPY versions are not considered. Open, not decided here: whether `gpy config get/set/list` reach plugin settings.
5. **Trust model and hardening.** The user installs a plugin by placing its directory in the user plugin root (`$XDG_CONFIG_HOME/gpy/plugins/`; or a bundled directory, `plugin/registry.rs:65-89`) and enabling a segment; GPY downloads nothing. A G script then runs as the user once per command that passes its trigger, including in untrusted directories (a cloned repository, an unpacked archive) whose names and files it receives as input. The script itself runs in a child `sh` and cannot change the shell's variables or functions, but the adapter around it runs in the interactive shell: in Bash 3.2 and 5.3, `${!n}` with `n='a[$(cmd)]'` ran `cmd` (run 2026-10-23), and the review showed the PoC adapter open to that and to globbing. Required:
   - **What the script sees.** A clean environment: `PATH`, `HOME`, `USER`, `LOGNAME`, `LANG`, every set `LC_*`, `TERM`, `TZ` and `TMPDIR`, plus the declared `env` names and the `GPY_*` inputs and `GPY_OPT_*` settings the runner sets (a user-exported `GPY_OPT_*` never reaches it). Credentials, `SSH_AUTH_SOCK` and exported functions are absent unless declared. stdin and stderr are `/dev/null`; cwd is the plugin root unless `pwd` is declared; the script runs in its own process group (or session) and never gets the terminal. The interpreter is `/bin/sh` by an absolute path resolved once (on macOS the shell `/private/var/select/sh` points to, run as `sh`, skipping the launcher); a bare `sh` repeats the `PATH` search on every run in Zsh.
   - Manifest-listed variable names match `^[A-Za-z_][A-Za-z0-9_]*$` (ASCII, no locale ranges), checked at theme export and before any indirect expansion; adapters never pass names or values through `eval`-like forms (`${!n}` on an unchecked name, `eval`, Zsh `${(P)…}`) or unquoted expansions that glob. Declared values reach the script byte for byte.
   - **Output, once at the runner boundary.** Cap at 4096 bytes (over the cap hides the segment; the cap comes before the strip), strip every C0 and C1 control (U+0000 to U+001F, U+007F, U+0080 to U+009F) by character ranges, not `[[:cntrl:]]` (per the Measurements finding), in Fish too, then escape for the drawing shell: Bash `\`, `$` and backticks; Zsh the same plus `%`; Fish nothing beyond the strip. Escaping is not idempotent, so it happens once; the cache stores that value. Output must not contain NUL; text after a NUL may be lost. Starship before v1.20 is the cautionary case.
   - The script path resolves under the plugin root (absolute paths, `..` and symlinks leaving it rejected), enforced at theme export, not only in `validate`.
   - SECURITY.md ("Plugin segment files run as you") changes with the adapters. "Bash and Zsh never source them" stays true of `.fish` files, but G scripts are executed, not sourced, by Bash, Zsh and Fish; the paragraph must say so, state the per-command, untrusted-directory exposure and the hardening above, and keep "trust it like a shell startup file" for all three shells.
6. **Timeout, kill rules and scan bound** (decided after Spike 2). The runner is **pure shell**, one per shell, inside a `$( )` (Bash, Zsh) or a child `sh` wrapper (Fish), with no files and no fixed descriptors; `require_command` runs under the same runner and deadline.
   - **Timeout.** `timeout_ms` per segment, default 500 (an author may lower it; the user overrides it in the item 4 table). One shared deadline per prompt, 500 ms from the first due segment's start; a segment not finished before it is deadline-skipped: hidden for this prompt, due again on the next.
   - **Kill rules.** On timeout or Ctrl-C: TERM, then CONT, to the script's process group, KILL after a 50 ms grace or as soon as the script itself exits; never wait for pipe EOF. On a normal exit, whatever the script left in its group gets TERM and CONT, then KILL after the grace.
   - **Circuit breaker.** Two consecutive timeouts or cap overflows disable the segment for this shell, with one stderr line, once. A success, the reload doorbell or a new shell resets it. Deadline-skipped runs, Ctrl-C and a non-zero exit do not count. The scan timeout counts the same way.
   - **Scan bound.** `require_files`/`require_folders` scan in a child with a 30 ms deadline, killed with SIGKILL (only a fatal signal ends a blocked NFS `stat`); a timeout hides the segment like a miss. The child uses cwd-relative paths (`Chart.yaml`, `../Chart.yaml`, …) and stops at the mount holding `$PWD` (Linux; macOS has no `/proc`, so the deadline alone bounds it); helpers run by absolute path; Fish starts it with `env -u PWD` because dash stats `$PWD` at startup.
   - **Documented gaps.** A `setsid` or daemonizing escapee survives; in a container whose PID 1 does not reap, orphans become zombies (run with `--init`); a `stat` in uninterruptible sleep can outlive the killed scan child, though the prompt stays bounded.
   - **Revisit a Rust helper** (`POSIX_SPAWN_SETSID`, a Linux subreaper) only if: (1) a pure-shell batch with one shared deadline cannot hold p99 ≤ T + G + 30 ms for 3 segments in Bash 3.2-4.4 or Fish; (2) the Fish over-cap error cannot be suppressed in the child for ≤ +1 ms; (3) a quiet-machine macOS run shows pure-shell Fish or Zsh above +3 ms, or more than 1.5 ms above the helper with the launcher skip on both; (4) the real hooks show a pure-shell failure the helper avoids, or a "no prompt until Ctrl-C" hang appears; (5) the maintainer makes containers with a non-reaping PID 1 a supported target; (6) the maintainer prefers one implementation and approves `unsafe`, run-time process execution in Rust and a third binary.
7. **Performance.** The PoC's five built-ins render in 35.88 / 13.13 / 20.67 ms (Bash / Zsh / Fish); `tests/performance-baselines.json` targets 37 (`prompt_render_bash`), 13 (`prompt_render_zsh`) and 20 ms (`prompt_render`, the warm `fish_prompt` render), with maxima 111, 39 and 50. A G hit adds 1.5 to 2.8 ms and a one-process run 5.8 to 8.7 ms, so one segment passes the Zsh target even on a hit and every target on a run; three G segments expiring on the same command add about 17 to 26 ms (arithmetic on reused PoC numbers; a variable-cache hit is not measured but should not cost more than the PoC's file read). The bounded runner adds +0.4 to +2.7 ms per uncached run on macOS (less with the launcher skip; clean environment included: +0.07 to +0.30 ms over the unmodified runner) and +0.4 to +0.9 ms on Linux. A due prompt with a `require_files`/`require_folders` condition pays one scan child, +0.6 to +0.9 ms on Linux (macOS not measured). On Bash 3.2-4.3 an enabled G segment installs a DEBUG trap, +5.94 µs per command. `scripts/ci-bench.sh` renders the configured built-in segments in every shell and no plugin segment, so the adapter work adds a G scenario (hit, expiry, and the scan child) with a budget per shell, Fish included, and states any target change for review. Moving `username`/`hostname` onto G lowers the prompt in each shell (hits of 1.5 to 2.8 ms against 3.5 to 9.3 ms built-in).
8. **Trigger semantics.** G keeps its rule: every condition must hold; files and folders are searched from the working directory up to the mount that holds it (item 6); extensions in the working directory only; the command condition (`require_command`) runs only after the others pass. `require_extensions` and `require_env` are fork-free; `require_files`/`require_folders` cost one bounded child per due prompt, so a false trigger on them is no longer free. An any-of rule would run the command on every prompt where the file checks miss, and project segments need the upward search. Starship's names mean something else (any one condition; working directory only), so G does not reuse them: `require_files`, `require_folders`, `require_extensions`, `require_env` and `require_command` (names fixed in implementation). The Starship importer therefore cannot map `[custom.*]` mechanically and keeps skipping it with its warning (`import/starship/layout.rs:63`); translating the cases where the meanings coincide is a later item.
9. **Autonomy boundaries** (the project's contributor approval list) the work crosses, each needing explicit approval in its PR: adding external process execution (the shells run scripts, `require_command` and the scan child; the CLI runs a syntax check and `doctor` timing); file path validation (script containment); the Fish shell integration contract (the loader branch, the event-handler placement and once-per-command reuse across repaints); configuration schema structure (the `config.toml` table, typed settings, the manifest entry type). The pure-shell runner adds no boundary beyond these.
10. **Implementation order**, each a separate work item: (1) B, as its own PR; (2) the contract: public shell-state names, colour variants, glyph mode, plugin `format` or the pill rule, G `entry_type`, enforced `min_gpy_version`, name validation, typed settings, the `config.toml` override and the published schema; (3) the timeout and scan spike (done; item 6); (4) G adapters in Bash, Zsh and Fish with the runner, placement, cache, trigger, scan child, output rule, containment, SECURITY.md change and `ci-bench` scenario; (5) tooling: `gpy plugin new` (scaffold), `validate` (schema, each segment's `description` as `starship explain` shows it, a syntax check with `dash -n` or `shellcheck -s sh` where installed, since macOS `/bin/sh` is Bash 3.2 and accepts bashisms), `doctor` (time the script), `list` (show the format); the examples include one segment that is not a built-in (a Kubernetes context or AWS profile); (6) dogfood `username`/`hostname`; (7) narrow B's docs.
11. **Later, not measured:** `show_on_command = "kubectl|helm"` in the manifest, showing a segment only while the typed command matches. Zsh can redraw while typing through a line-editor hook (`zle-line-pre-redraw`, `zle reset-prompt`), as Powerlevel10k does; Fish would need a key binding that repaints; Bash's readline offers no supported way to redraw the prompt while typing, so it would apply in Zsh and Fish only.

E-cached is not pursued: on these segments it would match a cached G hit but needs agent process execution, a `Message` change and cache invalidation, and goes stale without the agent. F is not a plugin format for these segments: four of the five need logic the template engine lacks.

## Alternatives Considered

### Option A

Fish-only, with the existing CLI-reference note. No cost, but it contradicts the cross-shell premise and leaves the silent drop shown in the baseline.

### Option B

Docs and messages, as in Decision item 1. No prompt-time cost and no change to shell scripts, IPC or `v1`. The `gpy enable` note reuses `setup::installed_integrations()`, which only checks that `$XDG_CONFIG_HOME/gpy/<shell>/` holds an entry point (`gpy-agent/src/commands/setup.rs:261-279`), so it can name a shell that is installed but unused. A mitigation, not cross-shell support.

### Option C

A `gpy doctor` warning per enabled plugin segment, from `enabled_plugin_segments` (`doctor.rs:280`) and `installed_integrations()` (`doctor.rs:46`). Keyed on installed integrations it has B's false positive inside a health report; keyed on the current shell (`GPY_SHELL`, then `$SHELL`) it misses a Fish user who also runs Bash. An optional add-on to B only if the `gpy enable` note proves insufficient.

### Option D

Port the loader: the theme export names a `.bash`/`.zsh`/`.fish` file per segment, and the Bash and Zsh loaders adapt the renderer's call to the `v1` `segment_<id>_render is_last is_first` (about 15 lines per shell on top of a public `gpy_section_standalone`). Measured above: the cheapest plugin approach in every cell, fork-free except Fish's `date`, always fresh, works without the agent. Privilege: plugin code is sourced into Bash and Zsh as it is into Fish today. Lost: authors maintain a Bash/Zsh file and a Fish file (the `directory` Fish file was a 104-line rewrite) and today must read GPY internals. `v1`: additive. Not chosen: deferred until a segment needs to be fresh and fork-free on every prompt.

### Option E

Activate `entry_type = "command"`. Uncached, the shell runs the program every prompt: measured at 6 to 10 ms per render with one segment and one process per segment, with no trigger and no cache; rejected (Decision item 3 says how G's default differs). E-cached (agent runs it off the request path, caches, repaints through SIGURG) was measured on the earlier Kubernetes segment only: hit cost equal to D, the value one repaint late on a new key, stale with the agent down, and approval-gated (external process execution from the agent, a `Message` variant, new path validation, a dependency from `plugin` on `ipc::registry`).

### Option F

Declarative trigger plus template rendered by the agent. Measured earlier at one agent request per segment per prompt. On the five segments: `username` needs a style chosen by root state (the built-in's `format`/`user_format` split); `hostname` needs `trim_at`, a string operation the template engine lacks; `duration` needs arithmetic formatting; `directory` needs the 312-line resolver; `clock` needs the per-shell live token. Each needs a new agent-side data source or transform, which is the built-in again.

### Option G

One portable script per segment, run by each shell behind a declarative trigger, cached in shell variables for a short TTL by default (measured above). Privilege: the script runs as the user from the shell, like D's code, in a child `sh` that cannot change the shell's variables or functions, while the adapter that feeds it manifest names and values runs in the interactive shell; it gets a clean environment plus declared names and runs in untrusted directories; the prototype had no timeout and inherited the whole environment (Decision items 5 and 6). It adds no agent process execution, no `Message`/`Response` change and no `security.rs` change, but crosses four autonomy boundaries (Decision item 9). Tooling: `gpy plugin new` scaffolds `bin/<id>.sh` and `segments/<id>.segment.toml`; `validate` checks the manifest against the published schema, prints each segment's description and syntax-checks the script; `gpy doctor` times it; `gpy plugin list` shows the format. `v1`: the manifest gains an entry type and an enforced minimum version, breaking where needed before announcement. Chosen, as a core feature.

## Consequences

### Positive

- B removes the silent surprise now.
- One script per segment renders in Bash, Zsh and Fish, with no GPY internal and no agent involvement, and can carry GPY's own static segments at a fraction of the template-rendered cost.
- Cached by default in shell variables, a G segment runs at most once per command and stays within one TTL of fresh; its trigger keeps it free on prompts where it does not apply; no cache file exists to go stale across shells or to be planted.
- Plugin settings get the declared types and load-time validation that core config already has, and the user's override survives a theme switch.

### Negative

- Only G is introduced, not two formats. D is deferred, so a segment that must be fresh and fork-free on every prompt has no plugin route until D is taken up.
- A run costs one `sh` (about 6 to 9 ms per render in the PoC) on each command after the TTL, or every command with `ttl_secs = 0`; otherwise output is stale up to the TTL. One segment can push a shell past its render target; the G scenario and budgets in `ci-bench` make that visible.
- A public contract to define first: shell state, colours, glyph mode, G `entry_type`, enforced `min_gpy_version`, and typed settings in a new `config.toml` table, a MAJOR configuration schema change.
- SECURITY.md changes: plugin code now runs from Bash and Zsh as well as Fish, on every command, in untrusted directories; the hardening in Decision item 5 is required, not optional.
- The timeout is bounded in every shell, Bash 3.2 included, at p99 within T + 50 + 30 ms, but with documented gaps: a `setsid` or daemonizing escapee survives, containers without a reaping PID 1 collect zombies (`--init` required), and an uninterruptible `stat` can outlive the scan child. The margin is thin under heavy load (worst sample 580.3 ms against 580 ms).
- Three runners, one per shell, with platform traps (dash `kill -SIG -PGID`, no `set -m` in `sh`, Bash 3.2 parse quirks) and a CI matrix with dash, busybox and Bash 4.x to keep them honest.
- G's trigger names differ from Starship's on purpose, so imported Starship configs still lose their `[custom.*]` modules.
- Plugins remain pills unless the contract adds `format`, and the clock cannot be a plugin.
- B and G are production code, each tracked as a separate work item, and four of the G items need explicit approval.

## References

- [Plugin Framework](../plugins.md); [CLI Reference: Plugin Commands](../../user/cli-reference.md#plugin-commands); [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) (SIGURG repaint); [ADR-0009](adr-0009-deferred-language-version-probes.md) (commands kept off the request path); [SECURITY.md](../../../SECURITY.md); [Schema Evolution](../../../gpy-agent/SCHEMA_EVOLUTION.md)
- Starship: [custom commands](https://starship.rs/config/#custom-commands), [prompt configuration](https://starship.rs/config/#prompt) (`command_timeout`, `scan_timeout`), [config schema](https://starship.rs/config-schema.json)
- Powerlevel10k: [README](https://github.com/romkatv/powerlevel10k) (Show on command, Extensible), [config/p10k-classic.zsh](https://github.com/romkatv/powerlevel10k/blob/master/config/p10k-classic.zsh) (user-defined segment example)
