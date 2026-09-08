#!/usr/bin/env fish
# Test: __gpy_read_instant_cache emits cache bytes exactly (fork-free read path, #144)
# Verifies the builtin read replacing `cat` reproduces output byte-for-byte for
# non-empty, no-trailing-newline, and empty cache files across git/lang suffixes.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# --- Setup: fake cache directory and git root ---
set -l tmp_dir (mktemp -d)
set -g fake_git_root "$tmp_dir/myproject" # global so assert_byte_exact (a function) can see it
mkdir -p "$fake_git_root/.git"

set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$tmp_dir/cache/gpy/instant-prompts"
mkdir -p "$cache_dir"

set -l resolved_root (path resolve -- "$fake_git_root" 2>/dev/null)
if test -z "$resolved_root"
    set resolved_root $fake_git_root
end
set -l cache_key (__gpy_path_to_cache_key "$resolved_root")

set -g __gpy_prompt_now (date +%s)

# Each case: <suffix> <description> writes bytes via printf, compares cmp.
function assert_byte_exact --argument-names suffix label cache_file
    # Capture __gpy_read_instant_cache output to a file (command substitution would
    # strip trailing newlines, so redirect instead and compare raw bytes).
    set -l out "$cache_file.out"
    __gpy_read_instant_cache $suffix "$fake_git_root" >"$out"
    if cmp -s "$out" "$cache_file"
        check "$label" pass
    else
        check "$label (expected "(wc -c <$cache_file | string trim)"B got "(wc -c <$out | string trim)"B)" fail
    end
end

# --- git suffix: must be fresh (<5s) to be served ---
set -l git_file "$cache_dir/$cache_key.git.none.ansi"
printf '\e[32mmain\e[0m ✓' >"$git_file" # no trailing newline
assert_byte_exact git "git non-empty bytes served exactly" "$git_file"

printf '' >"$git_file" # empty cache file
assert_byte_exact git "git empty cache emits nothing" "$git_file"

# --- lang suffix: always served regardless of age ---
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30
set -l lang_file "$cache_dir/$cache_key.lang.none.ansi"
printf '\e[32m ruby 3.4.0 \e[0m' >"$lang_file"
assert_byte_exact lang "lang non-empty bytes served exactly" "$lang_file"

printf 'trailing newline kept\n' >"$lang_file"
assert_byte_exact lang "lang trailing newline preserved" "$lang_file"

printf '' >"$lang_file"
assert_byte_exact lang "lang empty cache emits nothing" "$lang_file"

# Cleanup
rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count byte-exact cache tests passed"
