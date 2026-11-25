#!/usr/bin/env fish
# tests/fish/main.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# fish/conf.d/gpy_init.fish loads in an interactive Fish 3.6+ only: a
# non-interactive shell and an older Fish get none of its handlers, the older
# Fish with a one-line notice. Fish 4.2+ is transient-prompt capable. The
# version comes from the __gpy_test_fish_version hook ($FISH_VERSION is
# read-only).

source (dirname (status --filename))/../lib/test_helpers.fish

set -g init $__gpy_root/fish/conf.d/gpy_init.fish

# Load gpy_init.fish in a fresh shell, faking Fish $argv[1] (empty: the
# real version) and interactive mode when $argv[2] is -i; print what it
# defined, then stderr.
function load --argument-names fish_ver mode
    set -l flags --no-config
    test "$mode" = -i; and set -a flags -i
    fish $flags -c '
        test -n "$argv[2]"; and set -g __gpy_test_fish_version $argv[2]
        source $argv[1] 2>$argv[3]
        functions -q __gpy_on_prompt; and echo handlers
        echo capable=$__gpy_fish_transient_capable
        echo count=$__gpy_cmd_count' -- $init "$fish_ver" $__gpy_test_tmp_dir/stderr </dev/null 2>/dev/null
    cat $__gpy_test_tmp_dir/stderr
end

mkdir -p $__gpy_test_tmp_dir
test_track_dir $__gpy_test_tmp_dir

set -l out (load '' -i | string collect)
check "an interactive Fish loads the handlers" 1 (string match -q '*handlers*' -- $out; and echo 1)
check "the command count starts at 0" 1 (string match -q '*count=0*' -- $out; and echo 1)
check "loading prints nothing" 1 (string match -q '*gpy:*' -- $out; or echo 1)

set out (load '' '' | string collect)
check "a non-interactive Fish loads nothing" 1 (string match -q '*handlers*' -- $out; or echo 1)

set out (load 3.5.1 -i | string collect)
check "Fish 3.5 loads nothing" 1 (string match -q '*handlers*' -- $out; or echo 1)
check "Fish 3.5 is told it needs 3.6" 1 (string match -q '*gpy: GPY needs Fish 3.6 or newer*' -- $out; and echo 1)

set out (load 3.6.0 -i | string collect)
check "Fish 3.6 loads the handlers" 1 (string match -q '*handlers*' -- $out; and echo 1)
check "Fish 3.6 is not transient-prompt capable" 1 (string match -q '*capable=0*' -- $out; and echo 1)

set out (load 4.1.2 -i | string collect)
check "Fish 4.1 is not transient-prompt capable" 1 (string match -q '*capable=0*' -- $out; and echo 1)

set out (load 4.2.0 -i | string collect)
check "Fish 4.2 is transient-prompt capable" 1 (string match -q '*capable=1*' -- $out; and echo 1)

set out (load 5.0.0 -i | string collect)
check "Fish 5.0 is transient-prompt capable" 1 (string match -q '*capable=1*' -- $out; and echo 1)

# A beta or a development build: what follows the minor number is no
# integer, and loading must print nothing (no `test` error).
for ver in 4.0b1 4.2b1 3.6.1-56-gabc1234
    set out (load $ver -i | string collect)
    check "Fish $ver loads the handlers" 1 (string match -q '*handlers*' -- $out; and echo 1)
    check "Fish $ver loads silently" "" (string split \n -- $out | string match -v -r '^(handlers|capable=\d|count=\d+)$' | string collect)
end
check "Fish 4.0b1 is not transient-prompt capable" 1 (load 4.0b1 -i | string match -q 'capable=0'; and echo 1)
check "Fish 4.2b1 is transient-prompt capable" 1 (load 4.2b1 -i | string match -q 'capable=1'; and echo 1)

test_finish "gpy_init.fish load and version gate"
