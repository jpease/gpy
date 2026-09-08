#!/usr/bin/env fish

source fish/core/constants.fish
source fish/core/util.fish
source fish/core/ipc.fish

function make_test_repo
    set -l repo (mktemp -d)
    mkdir -p "$repo/.git"
    echo $repo
end

function cache_file_for --argument-names repo suffix
    # Reads pass no prev_bg, so the cache resolves the "none" token variant.
    set -l key (__gpy_path_to_cache_key (realpath "$repo"))
    echo "$XDG_CACHE_HOME/gpy/instant-prompts/$key.$suffix.none.ansi"
end

function test_fresh_language_cache_is_returned
    set -l repo (make_test_repo)
    set -l cache_file (cache_file_for "$repo" lang)
    mkdir -p (dirname "$cache_file")
    echo "fresh language cache" >"$cache_file"

    set -l result (__gpy_read_instant_cache lang "$repo")
    rm -rf "$repo"

    if test "$result" != "fresh language cache"
        echo "Expected fresh language cache to be returned, got '$result'"
        return 1
    end
end

function test_stale_language_cache_is_returned
    set -l repo (make_test_repo)
    set -l cache_file (cache_file_for "$repo" lang)
    mkdir -p (dirname "$cache_file")
    echo "stale language cache" >"$cache_file"
    touch -t 200001010000 "$cache_file"

    set -l result (__gpy_read_instant_cache lang "$repo")
    set -l read_status $status
    set -l still_exists 0
    test -f "$cache_file"; and set still_exists 1
    rm -rf "$repo"

    # #612: status now carries the freshness bit itself (2 = stale exact
    # hit) instead of a side-effect global; a plain "found" check would no
    # longer distinguish this from the fresh case.
    if not __gpy_cache_status_stale $read_status
        echo "Expected stale language cache read to report a stale status"
        return 1
    end

    if test "$result" != "stale language cache"
        echo "Expected stale language cache content, got '$result'"
        return 1
    end

    if test $still_exists -ne 1
        echo "Expected stale language cache file to be preserved"
        return 1
    end
end

# Git serve-stale (issue #160): a stale git entry must be SERVED and FLAGGED,
# never deleted. The fresh case must not set the staleness flag.
function test_fresh_git_cache_is_returned_not_stale
    set -l repo (make_test_repo)
    set -l cache_file (cache_file_for "$repo" git)
    mkdir -p (dirname "$cache_file")
    echo "fresh git cache" >"$cache_file"

    set -l result (__gpy_read_instant_cache git "$repo")
    set -l read_status $status
    rm -rf "$repo"

    if test $read_status -ne 0
        echo "Expected fresh git cache to be returned"
        return 1
    end
    if test "$result" != "fresh git cache"
        echo "Expected fresh git cache content, got '$result'"
        return 1
    end
    if __gpy_cache_status_stale $read_status
        echo "Fresh git cache must not report a stale status"
        return 1
    end
end

function test_stale_git_cache_is_served_flagged_and_preserved
    set -l repo (make_test_repo)
    set -l cache_file (cache_file_for "$repo" git)
    mkdir -p (dirname "$cache_file")
    echo "stale git cache" >"$cache_file"
    touch -t 200001010000 "$cache_file"

    set -l result (__gpy_read_instant_cache git "$repo")
    set -l read_status $status
    set -l still_exists 0
    test -f "$cache_file"; and set still_exists 1
    rm -rf "$repo"

    # #612: "found" and "flagged stale" used to be two separate checks (a
    # `read_status -ne 0` fail plus a staleness-global flag check); the stale
    # predicate alone now proves both, since only a found entry can report
    # stale (a miss is status 1, never 2 or 6).
    if not __gpy_cache_status_stale $read_status
        echo "Stale git cache must be returned (serve-stale) and report a stale status for background refresh"
        return 1
    end
    if test "$result" != "stale git cache"
        echo "Expected stale git cache content, got '$result'"
        return 1
    end
    # Regression guard for the old behavior: stale git cache used to be deleted.
    if test $still_exists -ne 1
        echo "Stale git cache file must be preserved, not deleted"
        return 1
    end
end

function test_git_cache_ttl_uses_named_constant
    set -l repo (make_test_repo)
    set -l cache_file (cache_file_for "$repo" git)
    mkdir -p (dirname "$cache_file")
    echo "old but inside configured ttl" >"$cache_file"
    touch -t 200001010000 "$cache_file"

    set -l previous_ttl "$GPY_GIT_INSTANT_CACHE_TTL_SECONDS"
    set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS 4102444800

    set -l result (__gpy_read_instant_cache git "$repo")
    set -l read_status $status

    if test -n "$previous_ttl"
        set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS "$previous_ttl"
    else
        set -e GPY_GIT_INSTANT_CACHE_TTL_SECONDS
    end
    rm -rf "$repo"

    if test $read_status -ne 0
        echo "Expected git cache to be returned when using configured TTL"
        return 1
    end
    if test "$result" != "old but inside configured ttl"
        echo "Expected configured-TTL git cache content, got '$result'"
        return 1
    end
    if __gpy_cache_status_stale $read_status
        echo "Git cache must not report stale before GPY_GIT_INSTANT_CACHE_TTL_SECONDS"
        return 1
    end
end

function test_missing_git_cache_returns_failure
    set -l repo (make_test_repo)

    set -l result (__gpy_read_instant_cache git "$repo")
    set -l read_status $status
    rm -rf "$repo"

    if test $read_status -ne 1
        echo "Expected missing git cache to report failure status 1 (cold miss), got $read_status"
        return 1
    end
    if test -n "$result"
        echo "Expected empty output for missing git cache, got '$result'"
        return 1
    end
end

set -gx XDG_CACHE_HOME (mktemp -d)
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30
set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS 5

test_fresh_language_cache_is_returned
or exit 1

test_stale_language_cache_is_returned
or exit 1

test_fresh_git_cache_is_returned_not_stale
or exit 1

test_stale_git_cache_is_served_flagged_and_preserved
or exit 1

test_git_cache_ttl_uses_named_constant
or exit 1

test_missing_git_cache_returns_failure
or exit 1

rm -rf "$XDG_CACHE_HOME"

echo "instant cache TTL tests passed"
