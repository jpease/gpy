# ADR-0011 Addendum: Script Segment Security

## Status

Proposed (2026-10). This is the security design for the G script segments of [ADR-0011](adr-0011-plugin-shell-coverage.md). It applies to ADR-0011 as amended in [PR #47](https://github.com/jpease/gpy/pull/47) (Decision item 5, "Trust model and hardening", and item 9, "Autonomy boundaries"). It turns that direction into rules the adapter work item implements and tests. The maintainer must approve the [approval boundaries](#approval-boundaries) before any implementing PR, and each such PR still needs its own approval.

## Context

The proof of concept behind ADR-0011 (`poc-adapters.patch`, against `28f2477`, kept outside the repository) was reviewed adversarially. The review found defects that break promises [SECURITY.md](../../../SECURITY.md) already makes. This addendum is not part of SECURITY.md because SECURITY.md describes shipped behaviour to users and to people reporting vulnerabilities. Rules for code that does not exist yet would be false there until the adapters ship. For now SECURITY.md only gets wording that stays true before and after the adapters land. The adapters PR then moves the [user-facing statement](#securitymd-when-the-adapters-ship) into it.

**Reproductions.** Run 2026-10-23 on macOS (arm64) with `/bin/bash` 3.2.57, Homebrew Bash 5.3.20, Zsh 5.9.2 and Fish 4.9.3. Every run used a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR`, with no agent and no real GPY configuration.

| Finding | How it was checked | Result |
|---|---|---|
| A directory name injects escape sequences | The PoC's Bash `__gpy_g_input`/`__gpy_g_run`/`__gpy_g_detect` were extracted unchanged and run in a directory named `$'x\n0\n\e]0;PWNED\a'` with `ttl_secs = 0` | Segment text was `\033]0;PWNED\a\|` and the script's `REAL-OUTPUT` was lost (Bash 3.2 and 5.3). The newline in `$PWD` shifts the line-based cache file, and the strip runs only before the write |
| Stale output across shells | Same functions, with a cache file under the new shell's own `$$` holding `SECONDS = 100000` (standing in for a reused PID) | `STALE-FROM-DEAD-SHELL` was shown with `ttl_secs` 0 and 30 (Bash 3.2 and 5.3) |
| Cache directory in `/tmp` | Read from the patch | Bash and Zsh use `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/gpy-script-segments`; Fish uses `XDG_RUNTIME_DIR` or `/tmp` |
| Indirection runs code | `n='a[$(echo PWNED >&2)]'`, then `${!n}` (Bash) or `${(P)n}` (Zsh) | Ran in Bash 3.2, Bash 5.3 and Zsh. Fish's `$$n` printed the text and ran nothing |
| An unquoted list globs | `envv='*'; for n in ${!envv}` with `${!n-}` in a directory holding a file named `x[$(echo GLOB-PWNED >&2)]` | Ran in Bash 3.2 and 5.3. With the name check below, the `*` is rejected and nothing runs |
| `[[:cntrl:]]` depends on the locale | `a`, U+009B (`c2 9b`), `b` through `${x//[[:cntrl:]]/}` and Fish `string replace -ra '[[:cntrl:]]'` | Bash and Zsh removed it under `en_US.UTF-8` and kept it under `LC_ALL=C`. Fish kept it in both locales |
| Prompt escaping | `v$(touch RAN-DOLLAR)` plus a backtick `touch` plus `%n`, put in `PS1` (Bash 5.3, `${PS1@P}`, `promptvars`) and in `PROMPT` (interactive `zsh -f`, `prompt_subst`) | Raw text: both files were created and Zsh expanded `%n`. After the existing `__gpy_prompt_escape` (`bash/core/renderer.bash`, `zsh/core/renderer.zsh`) the text was printed literally and no file was created |
| An absolute script path escapes the plugin root | Rust `Path::new("/plug/root").join(...)` | `"/etc/x.sh"` gave `/etc/x.sh`; `"bin/../../x.sh"` stayed lexically under the root but resolves outside it |
| The agent's strip set | `template/eval.rs` (`sanitize_control_chars` uses `char::is_control`); Rust check of `is_control` | U+009B and U+0085 are controls; U+00A0 is not. So the set is U+0000 to U+001F, U+007F and U+0080 to U+009F |

Not reproduced: the review's full PoC run under an interactive prompt (only the adapter functions were extracted). The Zsh and Fish halves of the cache defects were read from the patch, not run. Terminal behaviour for a lone `0x80`–`0x9F` byte that is not valid UTF-8 was not tested. Fish repaint counts were not checked either.

## Decision

### Trust model

- **Who installs.** The user does, by placing a plugin directory in the user plugin root (`$XDG_CONFIG_HOME/gpy/plugins/`) or using a bundled one, and enabling one of its segments. GPY downloads nothing and verifies nothing about a plugin.
- **What runs.** Each enabled script segment runs as the user, from Bash, Zsh or Fish, with the shell's environment and `PATH`. It runs once per command that passes its trigger, in every directory the user enters. That includes untrusted ones (a cloned repository, an unpacked archive), whose names and files reach the script as input. Installing a plugin means trusting its code like a shell startup file, in all three shells.
- **What GPY protects.** GPY does not protect the user from the script. It protects the interactive shell and the terminal from the script's output and from the manifest's names, and keeps directory names from reaching shell code. In order: script output never executes and never emits terminal controls; manifest names never execute; the executed file is the one inside the plugin directory; no state outside the shell can feed the prompt.
- **What GPY does not protect.** A malicious plugin, or a benign script that runs repository-local tools on an untrusted checkout. For example, `git` in a cloned repository may run repository-configured hooks or `core.fsmonitor` (inferred, not tested). Plugin documentation must say so.

### Output: strip, then escape, after every read

Script output goes through these steps every time a value is handed to the renderer, whether it comes from a run or from the cache. The cache stores the raw output. No step is skipped because an earlier one ran when the value was stored.

1. Split the output. Line 1 is the text; line 2 is the optional colour variant. Anything after line 2 is ignored. A length cap, if any, comes before the strip.
2. **Variant:** used only if it matches `^[a-z0-9_]{1,32}$` (validated under the C locale, as below); otherwise it is ignored. It is never filtered into shape, because it becomes part of a variable name.
3. **Strip** from the text every C0 and C1 control: U+0000 to U+001F, U+007F, and U+0080 to U+009F. This is the agent's `char::is_control` set. The rule does not depend on the user's locale and never uses `[[:cntrl:]]`. Each form below removed U+009B, U+0085, ESC, BEL, DEL and TAB, and kept `é`, `→`, `ü` and U+00A0, under `en_US.UTF-8`, `C` and `C.UTF-8` in every listed shell:
   - Bash (3.2 and 5.3), inside a function: `local LC_ALL=C`, then `x=${x//[$'\001'-$'\037'$'\177']/}` and `x=${x//$'\302'[$'\200'-$'\237']/}`. In the C locale the pattern matches bytes. C1 controls are the UTF-8 pairs `c2 80` to `c2 9f`, and `c2` never appears as a continuation byte, so other characters are untouched. Bash variables cannot hold NUL.
   - Zsh: the same two substitutions under `local LC_ALL=C`, with `$'\0'` as the first range bound, since Zsh variables can hold NUL (checked: `a\0b` became `ab`).
   - Fish: `string replace -ra '[\x00-\x1f\x7f-\x9f]' ''`. Fish matches code points in both locales, and its strings cannot hold NUL.

   A fast path that skips clean text must run under the same pinned locale. A lone byte `0x80`–`0x9F` that is not part of a UTF-8 sequence is not removed. A UTF-8 terminal should treat it as invalid UTF-8, not as a C1 control (not verified). The agent never sees such bytes, because it works on Rust `String`s.
4. **Escape** for the shell that draws the prompt, with the existing `__gpy_prompt_escape`. Bash: `\`, `$` and the backtick. Zsh: those plus `%`. Fish prints its prompt literally and needs nothing beyond the strip. The integrations already pin the options this relies on (`promptvars`; `prompt_subst`, `prompt_percent`, `no_prompt_bang`). Starship before v1.20, which did not escape `custom` output, is the case this avoids.

Values the shell passes to the script (`GPY_PWD` and the other `GPY_*` inputs, and `GPY_OPT_*` settings) go in as environment variables, unchanged. They are untrusted data for the script. The example scripts quote them, and the authoring docs say why.

### Manifest names

- At theme export, every name that becomes part of a shell variable or an environment variable must match `^[A-Za-z_][A-Za-z0-9_]*$`, checked as ASCII. This covers segment ids, `inputs`, forwarded environment names, setting keys (`GPY_OPT_<key>`) and the variant. A segment with any other name is not exported, and the reason is reported, as for other invalid theme content. `gpy plugin validate` applies the same check.
- The adapters never expand a name through `eval`, `${!n}`, Zsh `${(P)n}` or Fish `$$n` unless the name was checked against that pattern in the same shell. The export check is not enough, because a user can edit the exported variables. The in-shell check uses the C locale, or Fish's `string match -r`. These forms rejected `1bad`, `a[$(id)]`, `*`, `é`, `Ä`, `a-b` and the empty name under both locales (Bash 3.2 and 5.3, Zsh, Fish):
  - Bash: `[[ -n $n && $n != [0-9]* && $n != *[!A-Za-z0-9_]* ]]`
  - Zsh: `[[ -n $n && $n != [0-9]* && $n != *[^A-Za-z0-9_]* ]]`
  - Fish: `string match -qr '^[A-Za-z_][A-Za-z0-9_]*$' -- $n`
- Name lists are never split by an unquoted expansion that can glob. Bash splits with `set -f` in effect, restoring `$-` afterwards, because `local -` needs Bash 4.4. Zsh splits with `${=var}` and globs the result only under `GLOB_SUBST`, so the adapter runs `setopt local_options no_glob_subst`. Fish does not glob variable values. Arrays are preferred where the shell has them.

### Script containment at export

- The manifest's `script` must be a relative path with no `..` component. `plugin.root.join(script)` alone is not a check: an absolute path replaces the root.
- After the join, export canonicalizes the plugin root and the script and requires the script to be a regular file whose canonical path starts with the canonical root. A symlink leading out of the root is rejected.
- Export writes the canonical path, quoted for the target shell. It rejects paths containing control characters, as `is_safe_config_name` (`config/types.rs`) already does for theme and palette names.
- `gpy plugin validate` calls the same function. Containment that only `validate` checks does not count.
- Once checked, the path is trusted like the plugin's other files. Swapping the file afterwards needs write access to a directory the user owns, which is outside this design (see "No defence against a compromised account" in SECURITY.md).

### No files, no `/tmp`, no PID-keyed state

- The cache is shell variables in the interactive shell and nothing else: no cache files, no relay files, no marker or pending files.
- No path under `/tmp` or `$TMPDIR`. The runtime root is not an option either: `__gpy_runtime_root` falls back to `/tmp/gpy` (`bash/core/ipc.bash:107`, `zsh/core/ipc.zsh:112`, `fish/core/ipc.fish:28`). If a later item needs a file, it needs a new security review.
- No state keyed by `$$` or any PID. Ages come from a clock in the same shell, and nothing outlives it. The cache key is the plugin id and version, the segment id, the script path and every value passed to the script (ADR-0011 item 3). It is compared in memory, so a newline in `$PWD` cannot shift any framing.

### Approval boundaries

These are the items on the project's contributor approval list (Autonomy Boundaries) that the script-segment work crosses. Each implementing PR that touches one needs explicit maintainer approval:

- **Adding external process execution.** On the shell side, Bash, Zsh and Fish run `sh <script>` and the trigger's command condition. In the CLI, `gpy plugin validate` runs a syntax check (`dash -n` or `shellcheck -s sh`) and `gpy doctor` times the script.
- **File path validation.** Script containment at export and in `validate`.
- **The Fish shell integration contract.** The loader branch for script segments, and reuse across SIGURG repaints so the script runs once per command.
- **Configuration schema structure.** The script `entry_type`, the segment manifest, typed settings, and the per-segment `config.toml` table.

The agent gains no process execution and no `Message`/`Response` change, and `security.rs` is not touched.

### Named tests for the adapters item

Shell tests follow `tests/<shell>/<name>.test.<ext>` and exist for Bash (run under 3.2 and 5.x), Zsh and Fish. Rust tests sit next to the export code.

| Defect | Test |
|---|---|
| Newline and ESC in `$PWD`; strip after every read, on a run and on a cache hit | `script_segment_hostile_pwd` |
| C1 under `LC_ALL=C` and `en_US.UTF-8`, and in Fish | `script_segment_c1_locale` |
| Prompt expansion in script output (`$(…)`, backticks, `%`) | `script_segment_output_escaped` |
| `a[$(…)]` and `*` as manifest names, and in exported variables edited by hand | `script_segment_manifest_names` (shell); `script_names_validated_at_export` (Rust) |
| Absolute path, `..` and an outward symlink as `script` | `script_path_contained_at_export` (Rust), also from `gpy plugin validate` |
| No file is created anywhere under `HOME`, `TMPDIR`, `/tmp` or the runtime root, with caching on or off | `script_segment_no_files` |
| A planted file named after the shell's PID changes nothing | `script_segment_no_pid_state` |

### SECURITY.md when the adapters ship

The adapters PR replaces "Plugin segment files run as you" with a statement of the trust model above: script segments are executed (not sourced) by Bash, Zsh and Fish, as the user, once per command, in untrusted directories too. It lists the hardening: strip and escape after every read, name validation, containment, and no files. It keeps "trust it like a shell startup file" for all three shells. Until then, SECURITY.md says script segments are planned and links here.

## Consequences

- The PoC's cache design is not reused: its file cache, `$SECONDS`-in-file ages, `/tmp` fallback and `[[:cntrl:]]` strip are all replaced. The adapters item starts from these rules.
- The output path costs two pattern substitutions and an escape per segment per draw, even on a cache hit. The `ci-bench` G scenario (ADR-0011 item 7) measures it.
- Lone non-UTF-8 C1 bytes stay a documented gap in the shells. The agent does not have it.
- Four approval boundaries are crossed, so the work lands as several approved PRs, not one.

## References

- [ADR-0011](adr-0011-plugin-shell-coverage.md); [SECURITY.md](../../../SECURITY.md); [Plugin Framework](../plugins.md)
- Starship [custom commands](https://starship.rs/config/#custom-commands) (output escaping since v1.20)
