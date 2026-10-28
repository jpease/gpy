# ADR-0011 Addendum: Script Segment Security

## Status

Accepted (2026-10-27). This is the security design for the G script segments of [ADR-0011](adr-0011-plugin-shell-coverage.md). It supersedes ADR-0011 Decision item 5 ("Trust model and hardening") where the two differ, and details item 9 ("Approval boundaries"). The maintainer accepted it as written; the implementation landed in jpease/gpy#57 (#997).

## Context

A review of the proof of concept behind ADR-0011 found defects that break promises [SECURITY.md](../../../SECURITY.md) already makes. This addendum is not part of SECURITY.md because SECURITY.md describes shipped behaviour to users and to people reporting vulnerabilities. With the adapters (#997) the [user-facing statement](#securitymd-when-the-adapters-ship) moved into SECURITY.md, section "Script Segment Trust Model".

**Reproductions (macOS only).** Run on macOS (arm64) with `/bin/bash` 3.2.57, Homebrew Bash 5.3.20, Zsh 5.9.2 and Fish 4.9.3. Every run used a temporary `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR`, with no agent and no real GPY configuration. None of these forms was run on Linux (glibc or musl), Bash 4.x or Fish 3.x.

| Finding | How it was checked | Result |
|---|---|---|
| A directory name injects escape sequences | The proof of concept's Bash `__gpy_g_input`/`__gpy_g_run`/`__gpy_g_detect` were extracted unchanged and run in a directory named `$'x\n0\n\e]0;PWNED\a'` with `ttl_secs = 0` | Segment text was `\033]0;PWNED\a\|` and the script's `REAL-OUTPUT` was lost (Bash 3.2 and 5.3). The newline in `$PWD` shifts the line-based cache file, and the strip runs only before the write |
| Stale output across shells | Same functions, with a cache file under the new shell's own `$$` holding `SECONDS = 100000` (standing in for a reused PID) | `STALE-FROM-DEAD-SHELL` was shown with `ttl_secs` 0 and 30 (Bash 3.2 and 5.3) |
| Cache directory in `/tmp` | Read from the proof-of-concept code | Bash and Zsh use `${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/gpy-script-segments`; Fish uses `XDG_RUNTIME_DIR` or `/tmp` |
| Indirection runs code | `n='a[$(echo PWNED >&2)]'`, then `${!n}` (Bash) or `${(P)n}` (Zsh) | Ran in Bash 3.2, Bash 5.3 and Zsh. Fish's `$$n` printed the text and ran nothing |
| An unquoted list globs | `envv='*'; for n in ${!envv}` with `${!n-}` in a directory holding a file named `x[$(echo GLOB-PWNED >&2)]` | Ran in Bash 3.2 and 5.3. With the name check below, the `*` is rejected and nothing runs |
| `[[:cntrl:]]` depends on the locale | `a`, U+009B (`c2 9b`), `b` through `${x//[[:cntrl:]]/}` and Fish `string replace -ra '[[:cntrl:]]'` | Bash and Zsh removed it under `en_US.UTF-8` and kept it under `LC_ALL=C`. Fish kept it in both locales |
| A one-pass strip rejoins bytes | The previous rule's forms (one C0 and one C1 substitution in Bash and Zsh; one class in Fish) on the vectors in [step 4](#output-sanitized-once-before-any-shell-variable-holds-it) | `c2 c2 9b 9b` came out as `c2 9b` (U+009B, CSI) in every shell and locale. In Fish, `c2 1b 9b` also came out as `c2 9b` |
| Strip and escape cost at the cap | 4096 bytes through the strip and through `__gpy_prompt_escape`, once each, load average 10 to 13 | Bash 3.2 substitutions are quadratic in the number of matches: the C0 strip of 4096 ESC took 446 ms, a C1 pass over 1365 pairs 150 ms; Bash 5.3 took 1 ms. Escaping 4096 `$` took 514 ms in Bash 3.2 (`en_US.UTF-8`; 100 ms under `C`), 68 ms in Bash 5.3 and 42 ms in Zsh. Rerun with 1024 against 4096 ESC, interleaved, medians of 5 runs, load average 5 to 9: the whole-string C0 strip grew ×25 in Bash 3.2 (18 to 457 ms) and ×12 in Zsh (0.2 to 2.4 ms); over 64-byte slices it grew ×4.4 in Bash 3.2 (1.0 to 4.4 ms) and ×5.3 in Zsh (0.12 to 0.64 ms). Linear forms, 2026-10-27, Apple M4 Max, load average 5.1 to 5.3, `en_US.UTF-8`, 30 interleaved rounds of 10 calls, median (p90) ms per call at 4096 bytes: escape of `$` 2.3 (2.4) in Bash 3.2, 1.3 (1.4) in Bash 5.3, 2.1 (2.1) in Zsh; strip of ESC 4.6 (4.9) and of DEL 5.0 (5.3) in Bash 3.2, 0.8 (0.9) and 1.2 (1.3) in Bash 5.3 |
| Fish's name check accepts a newline | `string match -qr '^[A-Za-z_][A-Za-z0-9_]*$'` on `ab` plus a trailing newline | Matched: PCRE2's `$` also matches before a final newline. `\A…\z` rejected it |
| Prompt escaping | `v$(touch RAN-DOLLAR)` plus a backtick `touch` plus `%n`, put in `PS1` (Bash 5.3, `${PS1@P}`, `promptvars`) and in `PROMPT` (interactive `zsh -f`, `prompt_subst`) | Raw text: both files were created and Zsh expanded `%n`. After the existing `__gpy_prompt_escape` (`bash/core/renderer.bash`, `zsh/core/renderer.zsh`) the text was printed literally and no file was created |
| An absolute script path escapes the plugin root | Rust `Path::new("/plug/root").join(...)` | `"/etc/x.sh"` gave `/etc/x.sh`; `"bin/../../x.sh"` stayed lexically under the root but resolves outside it |
| The agent's strip set | `template/eval.rs` (`sanitize_control_chars` uses `char::is_control`); Rust check of `is_control` | U+009B and U+0085 are controls; U+00A0 is not. So the set is U+0000 to U+001F, U+007F and U+0080 to U+009F |

Not reproduced: the review's full PoC run under an interactive prompt (only the adapter functions were extracted). The Zsh and Fish halves of the cache defects were read from the patch, not run. Terminal behaviour for a lone `0x80`–`0x9F` byte that is not valid UTF-8 was not tested. Fish repaint counts were not checked either.

## Decision

### Trust model

- **Who installs.** The user does, by placing a plugin directory in the user plugin root (`$XDG_CONFIG_HOME/gpy/plugins/`) or using a bundled one, and enabling one of its segments. GPY downloads nothing and verifies nothing about a plugin.
- **What runs.** Each enabled script segment's `command`, and its `require_command` if it has one, runs as the user, from Bash, Zsh or Fish. It runs once per command that passes its trigger, in every directory the user enters. That includes untrusted ones (a cloned repository, an unpacked archive), whose names and files reach the script as input. Installing a plugin means trusting its code like a shell startup file, in all three shells.
- **What the script sees.** A clean environment, not the shell's:
  - The base set, each only when set: `PATH` without empty, `.` or other relative entries, `HOME`, `USER`, `LOGNAME`, `LANG`, every `LC_*`, `TERM`, `TZ` and `TMPDIR`. Dropping relative `PATH` entries matters because a segment that declares `pwd` runs inside the untrusted directory, where a planted `git` would otherwise run on `cd`.
  - The names the manifest declares in `env`, with their values passed byte for byte.
  - The `GPY_*` inputs and `GPY_OPT_*` settings, which the runner sets itself, so a value the user exported under the same name never reaches the script.
  - Nothing else: cloud credentials, tokens, `SSH_AUTH_SOCK` and exported shell functions are absent unless declared.

  The clean environment keeps the cache key complete and keeps credentials out of scripts by default. It is not a confidentiality boundary: a script running as the user can still read `~/.aws` and `~/.ssh`, find the SSH agent socket, or read other processes' environments where the system allows it. stdin is `/dev/null` and stderr is discarded. The working directory is the plugin root, or `$PWD` only when the segment declares the `pwd` input. The interpreter is `/bin/sh`, run by an absolute path resolved once per shell (on macOS, the shell `/private/var/select/sh` points to, run as `sh`), never a bare `sh` looked up on `PATH`. `require_command` gets the same environment, working directory, interpreter and process rules as `command`.
- **Declared values never in argv.** Declared values reach the script through its environment (`env -i` shape), never as arguments of its `sh`, so other local users cannot read them with `ps`. This costs +0.5 to +1.0 ms more per uncached run on macOS than the cheapest priced shape (`cxls`, which leaves each `NAME=value` in argv for the whole run); the maintainer chose it on 2026-10-25. `script_segment_clean_env` checks that no declared value appears in the script process's argv.
- **Process group.** The script runs in its own process group (a new group in the same session, started by the Bash runner with `set -m`, in all three shells) and is never given the foreground group. `require_command` runs first in the same group under the same deadline. Where it keeps a controlling terminal it can still open `/dev/tty`; text written there bypasses the strip. On timeout or Ctrl-C the group gets TERM, then CONT, then KILL after a 50 ms grace or as soon as the script exits. The runner never waits for pipe EOF. The KILL after the grace is asynchronous in every Bash: the timer group sends it in Bash 3.2 to 4.4, a detached killer in Bash 5. So a TERM-ignoring script can outlive the prompt by up to 50 ms after its deadline, on Bash 5 as on Bash 3.2.
- **Background processes.** After a normal exit, if members remain in the script's group, the runner sends TERM and CONT, polls `kill -0 -PGID` for at most the 50 ms grace, then sends KILL. It does this synchronously, before the prompt, and waits only when members remain. The spike runners sent TERM and CONT only; the adapters (#997) build the KILL step. A process that leaves the group (`setsid`, a daemonizing double fork) is outside this guarantee, on macOS and on Linux. In a container whose PID 1 does not reap, every timed-out run can leave zombies (1 to 3 per run in the spike) until the PID limit is reached; run containers with `--init`.
- **Ctrl-C.** Bash runs the runner in a subshell of itself; Fish and Zsh run it in a Bash child, the first Bash 4 or later on `PATH` found once per shell, else `/bin/bash`. With no Bash at all, script segments are disabled (a debug-level warning). Bash 3.2 ignores SIGINT inside `<( )` and runs traps only after `read -t` returns, so with Bash 3.2 (stock macOS) Ctrl-C during a slow script waits for the timeout; the run still ends and the group is killed. With Bash 4 or later Ctrl-C reaches the prompt at once (at most 58 ms on Linux in the spike). Fish leaves the terminal as the runner child's stdin so Fish makes it the foreground group and it receives the INT; the scripts still get `/dev/null`.
- **What GPY protects.** GPY does not protect the user from the script. It protects the interactive shell and the terminal from the script's output and from the manifest's names, and keeps directory names from reaching shell code. In order: script output never executes, and reaches the terminal with no C0 control, no DEL and no UTF-8-encoded C1 control (a lone byte `0x80`–`0x9F` is the residual in step 4); manifest names never execute; the executed files are the `command` and `require_command` scripts inside the plugin directory; no state outside the shell can feed the prompt.
- **What GPY does not protect.**
  - A malicious plugin, or a benign script that runs repository-local tools on an untrusted checkout. For example, `git` in a cloned repository may run repository-configured hooks or `core.fsmonitor` (inferred, not tested). Plugin documentation must say so.
  - Resources: there is no sandbox and no CPU, memory, file or process limit beyond the timeout. A script run in `$PWD` can write to the repository; `git` may refresh the index or take a lock.
  - Text a script writes to `/dev/tty`.
  - Unicode format characters (U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069, U+061C, U+FEFF, U+2028, U+2029, tag characters) are not controls and pass, so a directory or branch name echoed by a benign script can reorder or disguise neighbouring prompt text. The agent has the same gap.
  - The circuit breaker is a lever: a directory that makes a benign script time out twice hides the segment for the rest of the session, and a script that always finishes just under its timeout never counts.

### Output: sanitized once, before any shell variable holds it

The runner sanitizes script output once, at its boundary, before any shell variable holds the value. Raw output is never stored. The cache holds the render-ready value, and a cache hit hands it to the renderer unchanged. This is a performance choice: a hit or a repaint costs no substitution or escape. The PoC's alternative, storing raw output and escaping a copy on every draw, is also correct but pays the escape on every draw. Storing the escaped value is safe only with the [shell state rules](#shell-state-the-adapter-reads) below, because a shell variable can arrive pre-set from the parent environment.

The steps, in order:

1. **Cap** at 4096 bytes, before anything else, counted in bytes (`${#x}` under `LC_ALL=C`; Fish's `string length` counts characters, so Fish counts with `head -c` in the wrapper). Output over the cap hides the segment (it is not truncated), and two consecutive overflows count toward the segment's circuit breaker. 4096 is a decision: the spike runners capped at 64 KiB, except the Fish `fdrain` variant. Output must not contain NUL; text after a NUL may be lost, and shells differ in what they keep.
2. Split the output. Line 1 is the text; line 2 is the optional colour variant. Anything after line 2 is ignored.
3. **Variant:** used only if it matches `^[a-z0-9_]{1,32}$` (validated under the C locale, as below); otherwise it is ignored. It is never filtered into shape, because it becomes part of a variable name.
4. **Strip, then check.** Strip every C0 control and DEL from the text: U+0000 to U+001F and U+007F. Then, if the text still holds a C1 control (U+0080 to U+009F), **hide the segment**; C1 is never stripped. Removing bytes can join their neighbours into a new character: one C1 pass turns `c2 c2 9b 9b` into `c2 9b` (U+009B, CSI), and in Fish removing the ESC from `c2 1b 9b` does the same. A check after the C0 strip catches every such case in one match, and a benign script has no reason to print C1. The set is still the agent's `char::is_control` set; the agent strips C1 instead, which is safe there because it filters the `char`s of a valid Rust `String`. The rule does not depend on the user's locale and never uses `[[:cntrl:]]`.
   - Bash (3.2 and 5.3), inside a function: `local LC_ALL=C`, then strip 64-byte slices, reading the length once: `n=${#x}; for ((i = 0; i < n; i += 64)); do s=${x:i:64}; y+=${s//[$'\001'-$'\037'$'\177']/}; done`, then hide if `[[ $y == *$'\302'[$'\200'-$'\237']* ]]`. In the C locale the patterns match bytes. C1 controls are the UTF-8 pairs `c2 80` to `c2 9f`, and `c2` never appears as a continuation byte, so other characters are untouched. Bash variables cannot hold NUL. A whole-string `${x//…/}` is quadratic in Bash 3.2 (see Reproductions); removing a byte does not depend on its neighbours, so the slices give the same result.
   - Zsh: the same under `local LC_ALL=C`, with `$'\0'` as the first range bound, since Zsh variables can hold NUL. The slice is written `${x:$i:64}`, because Zsh reads an offset that starts with a letter as a modifier.
   - Fish: `set x (string replace -ra '[\x00-\x1f\x7f]' '' -- $x)`, then hide if `printf '%s' $x | string match -qr '[\x80-\x9f]'`. Fish keeps invalid bytes as raw bytes and writes them back unchanged, so the check runs on the bytes printed again, not on the variable. Both are builtins; the pipe forks nothing (2,000 checks took about 30 ms).

   Results, identical in all 12 shell and locale pairs (`/bin/bash` 3.2, Bash 5.3, Zsh and Fish, each under `en_US.UTF-8`, `C` and `C.UTF-8`; macOS only):

   | Vector (bytes) | One-pass strip (previous rule) | This rule |
   |---|---|---|
   | `61 c2 9b 62` (`a`, CSI, `b`) | `61 62` | hidden |
   | `61 c2 85 1b 07 7f 09 62` (NEL, ESC, BEL, DEL, TAB) | `61 62` | hidden |
   | `c3 a9 e2 86 92 c3 bc c2 a0` (`é`, `→`, `ü`, U+00A0) | unchanged | unchanged |
   | `c2 c2 9b 9b` (`c1_adjacent`) | `c2 9b` | hidden |
   | `c2 c2 c2 9b 9b 9b` | `c2 c2 9b 9b` | hidden |
   | `c2 1b 9b` | Bash, Zsh: empty; Fish: `c2 9b` | hidden |
   | `78 c2 01 9b 79` | Bash, Zsh: `78 79`; Fish: `78 c2 9b 79` | hidden |
   | `e2 c2 9b 82` | `e2 82` | hidden |
   | `61 9b 62`, `61 85 62` (lone bytes) | unchanged | unchanged |
   | `61 00 62` (Zsh only) | `61 62` | `61 62` |

   The sliced Bash and Zsh forms gave the same results in `/bin/bash` 3.2, Bash 5.3 and Zsh under the same three locales (2026-10-25; Fish's form is unchanged), as did vectors that put `c2`, ESC and `9b` across the byte-64 slice boundary and 4096-byte runs of ESC and `c2`.

   A fast path that skips clean text must run under the same pinned locale. A lone byte `0x80`–`0x9F` that is not part of a UTF-8 sequence is kept. A UTF-8 terminal should treat it as invalid UTF-8, not as a C1 control (not verified). A terminal not in UTF-8 mode reads every byte `0x80`–`0x9F` as C1, including the continuation bytes of valid text such as `→` (`e2 86 92`), which the agent emits too, so the lone byte adds no exposure the agent's output lacks there (inferred).
5. **Escape** for the shell that draws the prompt, with the existing `__gpy_prompt_escape`. Bash: `\`, `$` and the backtick. Zsh: those plus `%`. Fish prints its prompt literally and needs nothing beyond the strip, as long as the adapter prints the text with `printf '%s'` or `echo -n --`, never `echo $text` or `printf $text`. The integrations already pin the options this relies on (`promptvars`; `prompt_subst`, `prompt_percent`, `no_prompt_bang`). Starship before v1.20, which did not escape `custom` output, is the case this avoids.

Steps 4 and 5 run in the interactive shell, after the runner's deadline. A whole-string substitution in Bash 3.2 costs the rest of the string per match, so at the cap the whole-string strip took 446 ms and the per-character `__gpy_prompt_escape` it replaced 514 ms (see Reproductions). The sliced strip forms above and `__gpy_prompt_escape` (64-byte slices in Bash, one array element per byte in Zsh) grow about linearly. Measured 2026-10-27 on an Apple M4 Max (load average 5.1 to 5.3, 30 interleaved rounds, medians per call): at 4096 bytes the escape of `$` took 2.3 ms in Bash 3.2, 1.3 ms in Bash 5.3 and 2.1 ms in Zsh, and the sliced strip of ESC or DEL 4.6 to 5.0 ms in Bash 3.2 and 0.8 to 1.2 ms in Bash 5.3; from 1024 to 4096 bytes each grew ×3.8 to ×4.3 in Bash 3.2 and Zsh, where the per-character loop had grown ×8 to ×14. The 4096-byte cap applies in every shell (see [Open decisions](#open-decisions)).

Values the shell passes to the script (`GPY_PWD` and the other `GPY_*` inputs, declared `env` names, and `GPY_OPT_*` settings) go in as environment variables, unchanged. They are untrusted data for the script. The example scripts quote them, and the authoring docs say why.

### Shell state the adapter reads

- Every exported variable of the parent (an IDE, a terminal workspace, `sudo -E`, ssh `SendEnv`) becomes a shell variable at startup in Bash, Zsh and Fish. So the integration unsets the `__gpy_g_*` namespace at load and assigns every adapter variable (cache, ages, breaker counts, the once-per-command guard) before anything reads it.
- Integer state is checked as ASCII digits (`case $v in ''|*[!0-9]*) …` under `LC_ALL=C` in Bash and Zsh, `string match -qr '\A[0-9]+\z'` in Fish) before any arithmetic use.
- Text from a script, a directory, the manifest or the environment never goes into `(( ))`, `$(( ))`, `let`, `[[ … -eq … ]]`, an array subscript, or the name operand of `printf -v`, `read`, `declare`, `typeset`, `local` or `export`. With a variable holding `a[$(touch F)]`, `(( v > 0 ))` created `F` in Bash 3.2 and 5.3, and `printf -v` with that name created it in Bash 5.3; Zsh's `(( ))` did not.

### Manifest names

- The merged contract types are the first check, at discovery, so neither export nor `gpy plugin validate` sees a plugin that fails them:
  - Segment names match `^[a-z0-9_-]+$` (`SegmentName`). The shell token is the name with `-` turned into `_`, and names whose tokens collide are rejected (`first_token_collision`: in one manifest, across plugins, in `ui.enabled_segments` and among a theme's sections). That rejection is what keeps one segment's cache variables apart from another's.
  - `env` and `require_env` names match `^[A-Za-z_][A-Za-z0-9_]*$`, ASCII (`EnvName`); setting keys match `^[a-z_][a-z0-9_]{0,63}$` (`SettingKey`); `inputs` is a fixed list (`InputName`).
  - The output variant matches `^[a-z0-9_]{1,32}$` (`VariantName`); the shell checks it in step 3.
- Every shell or environment variable name the adapter builds is a fixed prefix plus one of these tokens or names, so the complete name matches `^[A-Za-z_][A-Za-z0-9_]*$`. The adapters never expand a name through `eval`, `${!n}`, Zsh `${(P)n}` or Fish `$$n` unless the complete name was checked against that pattern in the same shell. The export check is not enough, because a user can edit the exported variables. These forms rejected `1bad`, `a[$(id)]`, `*`, `é`, `Ä`, `a-b`, the empty name and `ab` plus a trailing newline under `en_US.UTF-8` and `C` (Bash 3.2 and 5.3, Zsh, Fish):
  - Bash, inside a function: `local LC_ALL=C; [[ -n $n && $n != [0-9]* && $n != *[!A-Za-z0-9_]* ]]`
  - Zsh, inside a function: `local LC_ALL=C; [[ -n $n && $n != [0-9]* && $n != *[^A-Za-z0-9_]* ]]`
  - Fish: `string match -qr '\A[A-Za-z_][A-Za-z0-9_]*\z' -- $n`
- Variant colour variables get their own namespace (`__gpy_segment_<token>_variant_<variant>_bg_color`), so variant `open` cannot name a cap colour. Tokens and variants both allow `_`, so `_`-joined names are still ambiguous across sections (segment `a` with an `open` cap and segment `a_open` with an `icon` both name `__gpy_segment_a_open_icon`; segment `a` with variant `b_c` and segment `a_b` with variant `c` both name `__color_a_b_c_bg`). The names stay as they are; instead one function in `theme/export/plugin_segments.rs` builds every plugin segment variable name, export writes from it, and theme validation (`first_plugin_variable_collision`) rejects a theme in which two fields would name the same variable, quoting both fields. Export skips a section that collides with an earlier one, so no variable is assigned twice even for an unvalidated theme. Discovery cannot check this: the names depend on the theme's keys, not the manifest.
- Name lists are never split by an unquoted expansion that can glob. Bash splits with `set -f` in effect, restoring `$-` afterwards, because `local -` needs Bash 4.4. Zsh splits with `${=var}` and globs the result only under `GLOB_SUBST`, so the adapter runs `setopt local_options no_glob_subst`. Fish does not glob variable values. Arrays are preferred where the shell has them. Lists whose entries are not identifiers (`require_files`, `require_folders`, `require_extensions`) are exported with a control character as the delimiter, which the types reject in an entry, never with whitespace.
- Manifest text the CLI prints (plugin and segment descriptions, setting descriptions, and the rejected value in a validation error) is escaped (`{:?}`) or stripped of controls before it reaches a terminal. `gpy plugin validate` and `gpy config list` strip controls from the segment and setting descriptions (the plugin `name` and `description` are not printed), and the errors of `ScriptPath::new`, `EnvName::new`, `ScanName::new`, `Extension::new`, `SettingKey::new`, `VariantName::new`, `VariantColorKey::new` and `SettingDecl::parse_str` echo the value `{:?}`-escaped.

### Script containment

- **Purpose.** A plugin cannot be sandboxed from its own scripts. Containment makes the files GPY runs the ones the plugin ships, so a manifest cannot name `/bin/rm` or a file elsewhere in the user's home.
- The manifest's `command` and `require_command` must be relative paths with no `..` component (`ScriptPath`, which also rejects control characters and backslashes). `plugin.root.join(path)` alone is not a check: an absolute path replaces the root.
- `ScriptSegmentManifest::load` canonicalizes the plugin root and both scripts and requires each to be a regular file whose canonical path starts with the canonical root. A symlink leading out of the root is rejected. `gpy plugin validate` and discovery, which theme export uses, both call it, so containment that only `validate` checks cannot happen.
- Export writes the canonical path, quoted for the target shell.
- Export refuses a script, or any directory from it up to the plugin root, that is group- or world-writable or owned by a user other than the current user or root (`check_owned_and_not_shared` in `gpy-agent/src/plugin/script.rs`, run on every export). Another user who can write there could otherwise run code as this user on every command.
- Export also refuses a plugin whose canonical root has an ancestor, up to `/`, that another user could use to replace the root (#1102). Renaming or replacing a directory needs write permission on its parent, not on the directory itself, so the checks above do not reach it. An ancestor passes when it is owned by the user or root and is not group- or world-writable, or when it is group- or world-writable but sticky and owned by the user or root or is group-writable but not world-writable through a user-private group (a process group named like the user whose only members are the user or root, so `$HOME` at 0775 under umask 002 passes; a shared group such as macOS `staff` does not) (the sticky bit stops others renaming the root, so `/tmp` and a private macOS `$TMPDIR` stay usable). The walk uses the canonical path, so a symlinked plugin root is judged by its physical location (`gpy-agent/src/plugin/script_ancestry.rs`). Cost: one `stat` per ancestor at export, none per prompt.
- Once checked, the path is trusted like the plugin's other files, and the adapters run the exported absolute path without resolving it again (decided on #997, answering the #1012 audit comment). A symlink swap or root replacement between export and run needs write access to the script, a directory on its path, or an ancestor's directory entry, and the rules above have just shown that only the user or root has it. An attacker with that access already runs code as the user, which is outside this design (see "No defence against a compromised account" in SECURITY.md); a run-time re-check would cost a `readlink`/`stat` per run and close no gap another user can reach.

### No files, no `/tmp`, no PID-keyed state

- No files. The cache and the once-per-command guard are shell variables in the interactive shell, and the runner uses none either: no cache, relay, marker, pending or output files. Both runner designs measured in the spike created 0 files. The adapters use no here-documents, here-strings or Zsh `=(…)`, because Bash before 5.1 backs here-documents and here-strings with temporary files and Zsh uses `$TMPPREFIX` for them (inferred). Bash `<(…)` falls back to a FIFO under `$TMPDIR` when `/dev/fd` is missing (inferred).
- No descriptor stays open after a run, and none is a fixed number shared with the user's shell. The scan child on Bash before 4.1, which has no `{fd}`, reads through a descriptor the adapter closes before the prompt.
- The scan child's PID comes from the child itself (it prints its PID first), never from `$!`. Whether `$!` names a process substitution in Bash 3.2 to 4.3 was not run, and a stale `$!` would signal the user's last background job.
- No path under `/tmp` or `$TMPDIR`. The runtime root is not an option either: `__gpy_runtime_root` falls back to `/tmp/gpy` (`bash/core/ipc.bash:107`, `zsh/core/ipc.zsh:112`, `fish/core/ipc.fish:28`). If a later item needs a file, it needs a new security review.
- No state keyed by `$$` or any PID. Ages come from a clock in the same shell, and nothing outlives it. Signalling a process group the runner obtained in the same run is not state: the PGID is never stored past that run. Its one exposure is a reuse window between the script's exit and the final signal (0 to 550 ms); reuse inside it needs the PID counter to wrap (inferred, not measured).
- The cache key is the plugin id and version, the segment, the `command` path, the resolved `GPY_OPT_*` values, the values of the declared inputs and the values of the declared `env` names (ADR-0011 item 3). It is compared in memory, so a newline in `$PWD` cannot shift any framing. The comparison quotes its right side (`[[ $stored == "$key" ]]`, or `case` with a quoted pattern): unquoted, the key is a glob, and a directory named `*` would match another folder's entry.

### As built (#997)

Where this differs from the rules above, the shipped adapters (`bash/core/g_runner.bash`, with byte-identical copies in `zsh/core` and `fish/core`; `bash/core/g_segments.bash`, `zsh/core/script.zsh`, `fish/core/script.fish`) behave as follows:

- **Start.** The interpreter runs as `sh -c PROLOGUE SCRIPT` after `exec -c` (empty environment). The prologue reads `export NAME='value'` lines on descriptor 3, closes it and sources the script. No value is in any argv; for a folder-scoped segment the runner gets an empty working-directory argument and keeps the one it inherited, so `$PWD` is not in argv either.
- **Fish output.** Fish does not cap, strip or check output itself: the Bash runner does steps 1 to 4 for all three shells, so the Fish forms in step 4 and the `head -c` cap are not used.
- **Deadline.** All due segments start together and each is killed `timeout_ms` after that shared start; there is no separate skip of segments past a 500 ms prompt deadline. The runner reads at most 40 due segments per call.
- **Triggers.** `require_env`, then one scan child for files, folders and extensions (names matched literally; files and folders searched through `../` paths, so physical parents; on Linux stopping at the mount), then the cache, then `require_command` in the run.
- **Results.** A non-zero exit, a failed `require_command` and C1 output are cached as hidden for `ttl_secs`; a timeout, over-cap output and Ctrl-C are not cached; a failed trigger drops the cache entry.
- **Breaker.** Separate counters for consecutive scan timeouts and consecutive run timeouts or over-cap outputs; either reaching two disables the segment for the shell. Ctrl-C never counts.
- **Clock.** Bash ages use `EPOCHSECONDS`, `printf '%(%s)T'`, or `$SECONDS` on Bash 3.2 to 4.1 (shell-relative, enough because the cache lives in that shell); Zsh `EPOCHSECONDS`; Fish the mtime of `/` plus its age.
- **Declared `env`.** A name is forwarded when it is set as a variable in the interactive shell, exported or not.

### Approval boundaries

These are the items on the changes needing explicit maintainer approval (ADR-0011 Decision item 9) that the script-segment work crosses. Each implementing PR that touches one needs explicit maintainer approval:

- **Adding external process execution.** On the shell side, Bash, Zsh and Fish run the `command` and `require_command` scripts and the scan child. In the CLI, `gpy plugin validate` runs a syntax check and `gpy doctor` times the script:
  - `gpy plugin validate` never runs a plugin script. It runs only `dash -n` or `shellcheck -s sh` (without `-x`) on it.
  - `gpy doctor` times only enabled segments, under the runner's rules: clean environment, timeout, plugin-root working directory.
  - Theme export writes only enabled segments. An installed plugin whose segments are not enabled is discovered and validated, never run.
- **File path validation.** Script containment at discovery, for both scripts.
- **The Fish shell integration contract.** The loader branch for script segments, and reuse across SIGURG repaints so the script runs once per command.
- **Configuration schema structure.** The script `entry_type`, the segment manifest, typed settings, and the `[plugin.<segment>]` table in `config.toml` (landed with the contract).

The agent gains no process execution and no `Message`/`Response` change, and `security.rs` is not touched.

### Open decisions

Decided by the maintainer for the adapters PR (#997):

- ~~Declared values in argv~~: decided 2026-10-25, environment only (see "Declared values never in argv").
- ~~Strip and escape cost at the cap in Bash 3.2~~: decided in #997, the 4096-byte cap applies in every shell, Bash 3.2 included, using the linear strip and escape forms (#54).
- ~~Run-time re-resolution of the script path~~: decided in #997, not done. Containment and the ownership rule are enforced at export, and the adapters run the exported canonical path (see [Script containment](#script-containment)).

### Named tests for the adapters item

Shell tests follow `tests/<shell>/<name>.test.<ext>` and exist for Bash (run under 3.2 and 5.x), Zsh and Fish. Rust tests sit next to the code they test.

| Defect | Test |
|---|---|
| Newline and ESC in `$PWD`; sanitized once, shown the same on a run and on a cache hit; a directory named `*`, `?` or `[a-z]*` does not hit another folder's entry | `script_segment_hostile_pwd` |
| Every vector in the step 4 table, including `c1_adjacent` (`c2 c2 9b 9b`), `c2 1b 9b` and a lone `9b`, under `LC_ALL=C`, `C.UTF-8` and `en_US.UTF-8`, in every shell | `script_segment_c1_locale` |
| Prompt expansion in script output (`$(…)`, backticks, `%`) | `script_segment_output_escaped` |
| `a[$(…)]`, `*` and a trailing newline as names, in exported variables edited by hand | `script_segment_manifest_names` (shell); the existing `env_names_are_validated_before_use` and `variant_names_follow_the_output_protocol` (Rust, `plugin/script_tests.rs`) |
| Pre-set `__gpy_g_*` variables and a numeric state variable holding `a[$(touch F)]` in the parent environment change nothing and create no file | `script_segment_env_seeded_state` |
| Absolute path, `..` and an outward symlink as `command` or `require_command` | the existing `script_paths_stay_relative`, `load_checks_the_script_exists_inside_the_root` and `load_rejects_a_symlink_out_of_the_root` (Rust); the symlink case gains `require_command`. A group-writable script or directory is refused at export (new Rust test) |
| No file is created anywhere under `HOME`, `TMPDIR`, `/tmp` or the runtime root, with caching on or off, and once without `/dev/fd` | `script_segment_no_files` |
| A planted file named after the shell's PID changes nothing | `script_segment_no_pid_state` |
| The hostile script set (hangs, process trees, TERM-ignoring children, terminal readers, `setsid`, 50 MB output, 4096 bytes of `$` and of ESC, forged framing) in GPY's real hooks. Correctness fails on any run: a prompt is drawn, the tree is killed, only documented survivors remain, no runner text reaches the terminal. Timing: Enter to prompt p99 within the timeout + 50 + 30 ms over at least 300 Enters per shell, with the load average recorded | `script_segment_hostile_scripts` |
| A script that exits normally and leaves a TERM-ignoring child: the child is killed within the grace, and the prompt waits only when members remain | `script_segment_leftover_kill` |
| Ctrl-C during a run draws a prompt, kills the tree, hides the segment and does not rerun it | `script_segment_ctrl_c` |
| Output over 4096 bytes, multibyte included, hides the segment with no text on the terminal, Fish included, and leaves shell memory unchanged | `script_segment_output_cap` |
| NUL in output never reaches the prompt and prints no shell warning | `script_segment_nul` |
| `$!`/`$last_pid`, the job table, the terminal's foreground group and open descriptors are unchanged by a run and by a scan | `script_segment_job_state` |
| A `require_files`, `require_folders` or `require_extensions` scan on a hung NFS mount, and a `require_extensions` scan of a directory with 1,000,000 entries, stay within the 30 ms bound and the child is killed (Linux CI only; needs a privileged container) | `script_segment_scan_hang` |
| Secrets and `SSH_AUTH_SOCK` absent unless declared; relative `PATH` entries dropped; `require_command` gets the same environment; hostile declared values (newlines, `$(…)`, backticks, globs, quotes) arrive literally; names like `a[$(cmd)]` rejected before any use; no declared value appears in any process's argv during the run | `script_segment_clean_env` |

### SECURITY.md when the adapters ship

The adapters PR replaces "Plugin segment files run as you" with a statement of the trust model above: script segments are executed (not sourced) by Bash, Zsh and Fish, as the user, once per command, in untrusted directories too. It lists the hardening: the clean environment (not a confidentiality boundary), output sanitized once before storage, name validation, containment and the ownership check, killing leftover background processes (with the `setsid` exclusion), and no files. It keeps "trust it like a shell startup file" for all three shells. It also corrects statements the adapters make incomplete:

- "Repositories are read, never written": a script that declares `pwd` runs tools such as `git` in the repository, and they may write index and lock files.
- "No sandbox": scripts have no CPU, memory, file or process limits beyond the timeout.
- "Prompt data is escaped … so neither can emit escape sequences": the shells keep a lone byte `0x80`–`0x9F`, and text written to `/dev/tty` is not sanitized.
- The documented gaps: lone C1 bytes, Unicode format characters, a `setsid` or daemonizing escapee, zombies under a non-reaping PID 1, and Ctrl-C bounded by the timeout in Fish and Zsh on a stock macOS (Bash 3.2 runner child).

Done with #997: SECURITY.md "Script Segment Trust Model" holds this statement, and the four corrections are in "Not Covered by This Design", "File System Access" and the prompt-escaping paragraph.

## Consequences

- The PoC's cache design is not reused: its file cache, `$SECONDS`-in-file ages, `/tmp` fallback and `[[:cntrl:]]` strip are all replaced. The adapters item starts from these rules.
- Output is sanitized once per run, so a cache hit or a repaint costs no substitution or escape. The `ci-bench` G scenario (ADR-0011 item 7) measures the run path.
- Output holding a C1 control is hidden, not cleaned, unlike agent-rendered text.
- Scripts that relied on an inherited variable must declare it in `env`. The clean environment missed the measured cost bar: like for like it costs +0.97 to +1.24 ms per uncached run on macOS against the +0.5 ms bar, and the measurements favoured inheriting. The maintainer chose cleaning anyway. With the launcher skip the same design also takes, a clean run costs +0.07 to +0.30 ms more than today's unmodified runner. On Linux cleaning cost +0.04 to +0.18 ms in a process-level loop, not the harness.
- A `setsid` or daemonizing escapee and zombies under a non-reaping container PID 1 are documented gaps.
- Lone non-UTF-8 C1 bytes stay a documented gap in the shells. The agent does not have it.
- Four approval boundaries are crossed, so the work lands as several approved PRs, not one.

## References

- [ADR-0011](adr-0011-plugin-shell-coverage.md); [SECURITY.md](../../../SECURITY.md); [Plugin Framework](../plugins.md)
- Starship [custom commands](https://starship.rs/config/#custom-commands) (output escaping since v1.20)
