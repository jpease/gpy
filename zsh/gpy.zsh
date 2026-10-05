# gpy.zsh - Entry point for Zsh integration

# Get the directory where this script is located
0=${(%):-%N}
GPY_ROOT=${0%/*}

# Validate installation before loading. A partial install (e.g. a core file
# that failed to download) must not source missing files one at a time --
# that prints a raw "no such file or directory" error on every new shell
# forever and half-loads the prompt. Check every core file this entry point
# is about to source before sourcing any of them, and disable cleanly with a
# single diagnostic if any are missing. Mirrors
# fish/conf.d/gpy_init.fish's guard.
for __gpy_core_file in constants ipc signals supervisor init; do
    if [[ ! -f "$GPY_ROOT/core/$__gpy_core_file.zsh" ]]; then
        print -r -- "gpy[init]: core files not found at $GPY_ROOT - GPY disabled" >&2
        unset __gpy_core_file
        return 0
    fi
done
unset __gpy_core_file

# Source core modules
source "$GPY_ROOT/core/constants.zsh"
source "$GPY_ROOT/core/ipc.zsh"
source "$GPY_ROOT/core/signals.zsh"
source "$GPY_ROOT/core/supervisor.zsh"

# Initialize (defines __gpy_load_theme)
source "$GPY_ROOT/core/init.zsh"

# Load theme from agent (overrides defaults in constants.zsh)
__gpy_load_theme &>/dev/null

# Source segments
for segment in "$GPY_ROOT"/segments/*.zsh; do
    source "$segment"
done

# Load shell completions for the `gpy` CLI, if installed. `completions/_gpy`
# is the STRUCTURAL completion, regenerated at install time from `gpy
# completions zsh` (clap_complete); it may be absent if the `gpy` CLI wasn't
# installed (#327 -- the CLI is optional, the prompt itself does not depend
# on it). `completions/_gpy-dynamic` is the checked-in, hand-authored glue
# that fills in dynamic values (theme/palette/segment names, #328) and
# otherwise delegates to the structural function. Both are best-effort: a
# missing completions directory must not break the prompt.
#
# Unlike fish (autoloads every file under completions/) and bash (registers
# via `complete -F` at source time), zsh normally associates a completer
# with a command by scanning fpath for `#compdef`-headed files *during
# `compinit`*. That scan already ran in the user's shell by the time this
# entry point is sourced -- the gpy-init rc block is appended to ~/.zshrc
# AFTER whatever `compinit` call the user's own config makes -- so adding to
# fpath alone here would never take effect. Instead this explicitly
# `autoload`s and `compdef`s the completer function, which works regardless
# of compinit ordering as long as compinit ran at some point (i.e.
# `compdef` exists); if it never did (e.g. a minimal non-interactive
# shell), this no-ops rather than erroring.
if [[ -d "$GPY_ROOT/completions" ]]; then
    # `-U` keeps fpath deduplicated so re-sourcing (new shells, upgrades)
    # never grows it with repeated entries.
    typeset -gU fpath
    fpath=("$GPY_ROOT/completions" $fpath)

    if [[ -f "$GPY_ROOT/completions/_gpy-dynamic" ]]; then
        autoload -Uz _gpy-dynamic

        if [[ -f "$GPY_ROOT/completions/_gpy" ]]; then
            autoload -Uz _gpy
        fi

        if whence compdef >/dev/null 2>&1; then
            compdef _gpy-dynamic gpy
        fi
    fi
fi
