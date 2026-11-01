# shellcheck shell=bash source-path=SCRIPTDIR
# gpy.bash - Entry point for Bash integration
# Requires Bash 3.2+

__gpy_bash_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check every core file before sourcing any: a partial install should disable
# with one diagnostic, not half-load and error on every shell.
for __gpy_core_file in constants renderer ipc signals supervisor g_runner g_segments init; do
    if [[ ! -f "$__gpy_bash_root/core/$__gpy_core_file.bash" ]]; then
        echo "gpy[init]: core files not found at $__gpy_bash_root - GPY disabled" >&2
        unset __gpy_core_file __gpy_bash_root
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

source "$__gpy_bash_root/core/constants.bash"
source "$__gpy_bash_root/core/renderer.bash"
source "$__gpy_bash_root/core/ipc.bash"
source "$__gpy_bash_root/core/signals.bash"
source "$__gpy_bash_root/core/supervisor.bash"
source "$__gpy_bash_root/core/g_runner.bash"
source "$__gpy_bash_root/core/g_segments.bash"

source "$__gpy_bash_root/core/init.bash"

__gpy_load_theme &>/dev/null

# Once per shell, in this shell and with stderr visible.
__gpy_warn_flag_values

# After the theme export, which sets the __gpy_cfg_* config mirrors.
__gpy_init

for __gpy_seg_file in "$__gpy_bash_root"/segments/*.bash; do
    # shellcheck source=/dev/null
    [[ -f "$__gpy_seg_file" ]] && source "$__gpy_seg_file"
done

# Bash honours only the last `complete -F`, and the dynamic file must win (it
# delegates to `_gpy`), so the generated structural completion loads first.
# shellcheck source=/dev/null
[[ -f "$__gpy_bash_root/completions/gpy.bash" ]] && source "$__gpy_bash_root/completions/gpy.bash"
[[ -f "$__gpy_bash_root/completions/gpy-dynamic.bash" ]] && source "$__gpy_bash_root/completions/gpy-dynamic.bash"

# Nothing reads these after loading; keep the shell's variable namespace clean.
unset __gpy_bash_root __gpy_seg_file
