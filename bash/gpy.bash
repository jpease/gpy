# shellcheck shell=bash source-path=SCRIPTDIR
# gpy.bash - Entry point for Bash integration
# Requires Bash 3.2+

GPY_BASH_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check every core file before sourcing any: a partial install should disable
# with one diagnostic, not half-load and error on every shell.
for __gpy_core_file in constants renderer ipc signals supervisor g_runner g_segments init; do
    if [[ ! -f "$GPY_BASH_ROOT/core/$__gpy_core_file.bash" ]]; then
        echo "gpy[init]: core files not found at $GPY_BASH_ROOT - GPY disabled" >&2
        unset __gpy_core_file
        return 0
    fi
done

# Script segment state and the theme export's script variables can arrive
# from the parent environment (an IDE, `sudo -E`, ssh SendEnv); none of it may
# be read, so both namespaces start empty and are assigned before any read.
for __gpy_core_file in "${!__gpy_g_@}" "${!__gpy_script_@}"; do
    unset "$__gpy_core_file"
done
unset __gpy_core_file

source "$GPY_BASH_ROOT/core/constants.bash"
source "$GPY_BASH_ROOT/core/renderer.bash"
source "$GPY_BASH_ROOT/core/ipc.bash"
source "$GPY_BASH_ROOT/core/signals.bash"
source "$GPY_BASH_ROOT/core/supervisor.bash"
source "$GPY_BASH_ROOT/core/g_runner.bash"
source "$GPY_BASH_ROOT/core/g_segments.bash"

source "$GPY_BASH_ROOT/core/init.bash"

__gpy_load_theme &>/dev/null

# After the theme export, which sets the GPY_AGENT_*ENABLED flags.
__gpy_init

for segment in "$GPY_BASH_ROOT"/segments/*.bash; do
    # shellcheck source=/dev/null
    [[ -f "$segment" ]] && source "$segment"
done

# Bash honours only the last `complete -F`, and the dynamic file must win (it
# delegates to `_gpy`), so the generated structural completion loads first.
# shellcheck source=/dev/null
[[ -f "$GPY_BASH_ROOT/completions/gpy.bash" ]] && source "$GPY_BASH_ROOT/completions/gpy.bash"
[[ -f "$GPY_BASH_ROOT/completions/gpy-dynamic.bash" ]] && source "$GPY_BASH_ROOT/completions/gpy-dynamic.bash"
