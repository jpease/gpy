#!/usr/bin/env fish
# tests/fish/json_escape_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Shared JSON-escape test vector file
# (tests/fixtures/json_escape_vectors.tsv), run by all three shells, so
# __gpy_json_escape (Fish), __gpy_escape_json_into (Bash), and
# __gpy_escape_json_into (Zsh) can never silently drift apart. This proves Fish's implementation
# (fish/core/ipc.fish), including its \n/\r/\t escaping, against the same
# vectors bash/zsh check.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish

# Undo the fixture's input-column mini escape scheme (see its own header
# comment): backslash first, then control chars. Uses a sentinel byte to
# protect literal backslash pairs from being re-touched by the later
# \n/\r/\t substitutions. Every step pipes through `string collect` -- a
# result containing a real embedded newline would otherwise be split into
# multiple list elements the instant it crosses a `(...)` command
# substitution boundary, silently dropping the newline when later
# reassembled as a plain string.
function unescape_input --argument-names raw
    set -l sentinel \x01
    set -l s (string replace -a -- '\\\\' "$sentinel" "$raw" | string collect)
    set s (string replace -a -- '\n' \n "$s" | string collect)
    set s (string replace -a -- '\r' \r "$s" | string collect)
    set s (string replace -a -- '\t' \t "$s" | string collect)
    set s (string replace -a -- "$sentinel" '\\' "$s" | string collect)
    printf '%s' "$s"
end

set -l fixture "$repo_root/tests/fixtures/json_escape_vectors.tsv"
set -l vector_count 0

# `-d \t` splits ONLY on tab (fish's default `read` also splits on spaces,
# which would wrongly break apart a column like "plain text no special
# chars" that contains no tab at all).
while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    set -l first_char (string sub -l 1 -- "$cols[1]")
    test "$first_char" = '#'; and continue
    test (count $cols) -lt 2; and continue

    set vector_count (math $vector_count + 1)
    set -l input_col $cols[1]
    set -l expected_col $cols[2]
    # `string collect`: unescape_input's result may contain a real embedded
    # newline (for the \n vectors) -- see its own comment for why every
    # `(...)` boundary a raw newline crosses needs this.
    set -l real_input (unescape_input "$input_col" | string collect)
    set -l actual (__gpy_json_escape "$real_input")
    check "vector $vector_count ($input_col)" "$expected_col" "$actual"
end <"$fixture"

if test $vector_count -eq 0
    fail "no test vectors read from $fixture"
end

# Control bytes the fixture's mini scheme cannot carry: every other C0 byte
# and DEL must leave as \u00XX, or the request is invalid JSON (the agent
# then refuses it and the segment falls back to oneshot).
check "ESC as \\u001b" 'a\u001bb' (__gpy_json_escape a\x1bb)
check "BEL, BS, VT, FF" '\u0007\u0008\u000b\u000c' (__gpy_json_escape \x07\x08\x0b\x0c)
check "0x01 and 0x1f" '\u0001x\u001f' (__gpy_json_escape \x01x\x1f)
check "DEL as \\u007f" 'a\u007f' (__gpy_json_escape a\x7f)
check "controls next to short escapes" '\u001b\n\"\\\\\u0001' (__gpy_json_escape \x1b\n'"\\'\x01)
if command -q python3
    set -l payload '{"cwd":"'(__gpy_json_escape /tmp/a\x1b\x01b\x7f)'"}'
    if printf '%s' $payload | python3 -c 'import json,sys; json.loads(sys.stdin.read())' 2>/dev/null
        pass "a cwd with control bytes builds valid JSON"
    else
        fail "a cwd with control bytes builds valid JSON: $payload"
    end
end

test_finish "JSON-escape vector tests"
