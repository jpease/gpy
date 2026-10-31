# Dynamic completion glue

`gpy completions <shell>` (clap_complete) is regenerated at install time and
covers the structural surface: subcommands and flags. It cannot know the live
themes, palettes or segments on the machine. The hand-authored files
`bash/completions/gpy-dynamic.bash`, `zsh/completions/_gpy-dynamic` and
`fish/completions/gpy-dynamic.fish` supply those value lists from the hidden
`gpy __complete <kind>` command, which prints newline-delimited names.

The dynamic positions are:

| Command | Source |
| --- | --- |
| `gpy theme use <TAB>`, `gpy theme validate <TAB>`, `gpy theme diff <TAB>`, `gpy theme reset <TAB>`, `gpy theme migrate <TAB>`, `gpy theme rebase <TAB>` | `gpy __complete theme` |
| `gpy palette use <TAB>`, `gpy palette validate <TAB>` | `gpy __complete palette` |
| `gpy enable <TAB>`, `gpy disable <TAB>` | `gpy __complete segment` |

## Coexistence with the generated file

Bash (`complete -F`) and Zsh (`compdef`) keep one completer per command, so the
dynamic function handles only the positions above and delegates everything else
to the generated `_gpy`. If `_gpy` is not defined, delegation yields no
candidates rather than an error. Bash therefore requires the dynamic file to be
sourced after `completions/gpy.bash`. Fish merges `complete -c` rules from every
sourced file, but autoloads `completions/<command>.fish` only for the command of
that name, so the installers append the glue to the generated output and write a
single `completions/gpy.fish`.

## Cache lifetime

Each kind's result is memoized in a session-global variable
(`__gpy_complete_cache_<kind>`), so repeated TAB presses on one command line
never fork `gpy`. Only a non-empty result is cached; a failed or empty call
leaves the variable unset so the next TAB retries. The cache lives until the
next prompt: Bash and Zsh `unset` the variables in `__gpy_precmd`, and Fish
does so from a `fish_preexec` handler defined in the completion file. A theme,
palette or segment created by the previous command therefore completes on the
next line.

- Bash 3.2 has no associative arrays, so the cache is one plain variable per kind
  read with `${!name}`; Zsh uses the `(P)` flag the same way.
- Call the cache function as a plain statement, never through `$(...)`: a
  command substitution forks a subshell whose variable writes do not reach the
  caller. Results come back through `__gpy_complete_result`.
- In Zsh the cache function is defined inside `_gpy-dynamic`. `autoload -Uz` only
  auto-invokes a function on first call when the file contains that one
  definition; a second top-level function makes Zsh run the file as the body, and
  the first TAB of a session would do nothing. The function is named after the
  file because `autoload` locates definitions by file name on `fpath`.
