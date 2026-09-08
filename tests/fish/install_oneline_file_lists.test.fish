#!/usr/bin/env fish
# tests/fish/install_oneline_file_lists.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Guards against #308: install-oneline.sh downloads shell files from a
# hardcoded per-file list per shell. Those lists silently rot when a new
# core/segment/function file is added to the repo but not to the installer,
# so fresh one-line installs quietly ship an incomplete set of files.
#
# This reads the FISH_/ZSH_/BASH_ file-list variables directly out of
# install-oneline.sh (not by parsing the `for...in` loop syntax, so
# reordering or reformatting those loops doesn't break this test) and
# compares each one, sorted, against the files actually present in the
# corresponding repo directory. `example_*.fish` files are template
# snippets for plugin authors, not installable segments, and are
# intentionally excluded from the comparison.
#
# It also guards the companion fix: the shell files are downloaded from
# the same $VERSION as the binary, not a hardcoded "main" ref (the binary's
# own development-build fallback URL, which intentionally still targets
# "main", is left alone and is not checked here).

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l installer install-oneline.sh
set -g failed 0

function __install_oneline_test_var_files --argument-names var_name installer
    set -l line (string match -r -- "^$var_name=\"[^\"]*\"" <"$installer")
    if test -z "$line"
        return 1
    end
    string replace -r -- "^$var_name=\"([^\"]*)\"\$" '$1' $line[1] | string split ' '
end

function __install_oneline_test_tree_files --argument-names dir
    for f in $dir/*
        set -l base (path basename $f)
        if string match -q 'example_*' -- $base
            continue
        end
        echo $base
    end
end

function __install_oneline_test_compare --argument-names label var_name installer dir
    set -l list_files (__install_oneline_test_var_files $var_name $installer)
    if test -z "$list_files"
        echo "❌ Could not find $var_name in $installer"
        set -g failed 1
        return
    end

    set -l tree_files (__install_oneline_test_tree_files $dir)

    set -l sorted_list (printf '%s\n' $list_files | sort)
    set -l sorted_tree (printf '%s\n' $tree_files | sort)

    if test "$sorted_list" != "$sorted_tree"
        echo "❌ $label: $var_name does not match files in $dir/"
        echo "   installer list: $sorted_list"
        echo "   actual files:   $sorted_tree"
        set -g failed 1
    else
        echo "✅ $label: $var_name matches $dir/"
    end
end

__install_oneline_test_compare "Fish core" FISH_CORE_FILES $installer fish/core
__install_oneline_test_compare "Fish segments" FISH_SEGMENT_FILES $installer fish/segments
__install_oneline_test_compare "Fish functions" FISH_FUNCTION_FILES $installer fish/functions
__install_oneline_test_compare "Zsh core" ZSH_CORE_FILES $installer zsh/core
__install_oneline_test_compare "Zsh segments" ZSH_SEGMENT_FILES $installer zsh/segments
__install_oneline_test_compare "Bash core" BASH_CORE_FILES $installer bash/core
__install_oneline_test_compare "Bash segments" BASH_SEGMENT_FILES $installer bash/segments

# The shell-files base URL and the gpy_init.fish download must use
# $VERSION, not a hardcoded "main" ref, so fresh installs pair shell files
# with the same release as the binary (#308).
if string match -q -- '*SHELL_FILES_BASE_URL=*/main/*' <$installer
    echo "❌ $installer: SHELL_FILES_BASE_URL still hardcodes /main/"
    set -g failed 1
else
    echo "✅ SHELL_FILES_BASE_URL is not hardcoded to /main/"
end

if string match -q -- '*raw.githubusercontent.com/$REPO/main/fish/conf.d/gpy_init.fish*' <$installer
    echo "❌ $installer: gpy_init.fish download still hardcodes /main/"
    set -g failed 1
else
    echo "✅ gpy_init.fish download is not hardcoded to /main/"
end

# Regression guard for #615: install-oneline.sh used to branch on
# curl-vs-wget separately at every download call site (18 copies at last
# count) instead of going through the single `fetch_to`/`fetch_stdout`
# helpers defined once near the top of the file. The only legitimate
# `command -v curl` left is the one-time $DOWNLOADER resolution those
# helpers are built on.
set -l curl_branch_count (grep -c 'command -v curl' $installer)
if test "$curl_branch_count" = 1
    echo "✅ $installer has exactly one curl-vs-wget branch (the \$DOWNLOADER resolution)"
else
    echo "❌ $installer has $curl_branch_count 'command -v curl' occurrences; expected exactly 1 (fetch_to/fetch_stdout should be used everywhere else)"
    set -g failed 1
end

if test $failed -eq 1
    exit 1
end

echo "✅ install-oneline.sh file lists match the repo tree"
