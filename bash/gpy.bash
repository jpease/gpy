# shellcheck shell=bash source-path=SCRIPTDIR
# gpy.bash - Entry point for Bash integration
# Requires Bash 4.0+

# Get the directory where this script is located
GPY_BASH_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Validate installation before loading. A partial install (e.g. a core file
# that failed to download) must not source missing files one at a time --
# that prints a raw "no such file or directory" error on every new shell
# forever and half-loads the prompt. Check every core file this entry point
# is about to source before sourcing any of them, and disable cleanly with a
# single diagnostic if any are missing. Mirrors
# fish/conf.d/gpy_init.fish's guard.
for __gpy_core_file in constants ipc signals supervisor init; do
    if [[ ! -f "$GPY_BASH_ROOT/core/$__gpy_core_file.bash" ]]; then
        echo "gpy[init]: core files not found at $GPY_BASH_ROOT - GPY disabled" >&2
        unset __gpy_core_file
        return 0
    fi
done
unset __gpy_core_file

# Source core modules
source "$GPY_BASH_ROOT/core/constants.bash"
source "$GPY_BASH_ROOT/core/ipc.bash"
source "$GPY_BASH_ROOT/core/signals.bash"
source "$GPY_BASH_ROOT/core/supervisor.bash"

# Initialize (defines __gpy_load_theme)
source "$GPY_BASH_ROOT/core/init.bash"

# Load theme from agent (overrides defaults in constants.bash)
__gpy_load_theme &>/dev/null

# Source all segments
for segment in "$GPY_BASH_ROOT"/segments/*.bash; do
    # Segments are discovered by glob; the gate shellchecks each one directly.
    # shellcheck source=/dev/null
    [[ -f "$segment" ]] && source "$segment"
done

# Load shell completions for the `gpy` CLI, if installed. Unlike fish (which
# autoloads every file under completions/), bash has no autoload mechanism --
# completion functions must be `source`d and registered with `complete -F`
# explicitly, so the entry point does it here on every shell start.
#
# `completions/gpy.bash` is the STRUCTURAL completion, regenerated at install
# time from `gpy completions bash` (clap_complete); it may be absent if the
# `gpy` CLI wasn't installed (#327 -- the CLI is optional, the prompt itself
# does not depend on it). `completions/gpy-dynamic.bash` is the checked-in,
# hand-authored glue that fills in dynamic values (theme/palette/segment
# names) and otherwise delegates to the structural function. Both are
# best-effort: a missing completions directory must not break the prompt.
#
# Load order matters: bash allows only ONE `complete -F` registration to win
# per command, and the dynamic file's registration must be the one that
# wins (it delegates to `_gpy` for non-dynamic positions), so structural is
# sourced first here.
# completions/gpy.bash is generated at install time; it is not in the tree.
# shellcheck source=/dev/null
[[ -f "$GPY_BASH_ROOT/completions/gpy.bash" ]] && source "$GPY_BASH_ROOT/completions/gpy.bash"
[[ -f "$GPY_BASH_ROOT/completions/gpy-dynamic.bash" ]] && source "$GPY_BASH_ROOT/completions/gpy-dynamic.bash"
