#!/usr/bin/env fish
# Test: the bundled username/hostname plugin commands (plugins/*/bin/*.sh) —
# visibility (root/sudo/SSH or the `show_always` setting), hostname trimming
# (`trim_at`) and the variant line (`root` / `ssh`). Replaces the deleted
# built-in fish segment tests; the commands are run with the environment the
# G runner gives them (GPY_IS_*, GPY_OPT_*).

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
set -g user_cmd $repo_root/plugins/username/bin/username.sh
set -g host_cmd $repo_root/plugins/hostname/bin/hostname.sh

# Runs a command with a clean GPY_* environment plus the given assignments.
function run_cmd --argument-names cmd
    env -u GPY_IS_ROOT -u GPY_IS_SUDO -u GPY_IS_SSH -u GPY_OPT_show_always -u GPY_OPT_trim_at \
        $argv[2..] sh $cmd | string collect
end

# --- username ---

check "username: hidden in a plain local session" "" (run_cmd $user_cmd USER=bob)
check "username: shown over SSH, no variant" bob (run_cmd $user_cmd USER=bob GPY_IS_SSH=1)
check "username: shown under sudo, no variant" bob (run_cmd $user_cmd USER=bob GPY_IS_SUDO=1)
check "username: shown with show_always" bob (run_cmd $user_cmd USER=bob GPY_OPT_show_always=true)
check "username: root prints the root variant" "root
root" (run_cmd $user_cmd USER=root GPY_IS_ROOT=1)
check "username: empty USER hides" "" (run_cmd $user_cmd USER= GPY_IS_SSH=1)

# --- hostname ---

set -l node (uname -n)
set -l short (string split -m1 . -- $node)[1]
check "hostname: hidden off SSH" "" (run_cmd $host_cmd)
check "hostname: shown with show_always, trimmed at the first '.'" $short (run_cmd $host_cmd GPY_OPT_show_always=true)
check "hostname: empty trim_at keeps the whole name" $node (run_cmd $host_cmd GPY_OPT_show_always=true GPY_OPT_trim_at=)
check "hostname: SSH prints the ssh variant" "$short
ssh" (run_cmd $host_cmd GPY_IS_SSH=1)

test_finish "identity plugin command tests"
