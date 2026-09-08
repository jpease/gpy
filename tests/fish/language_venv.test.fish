#!/usr/bin/env fish
# Test that the `lang` IPC payload forwards $VIRTUAL_ENV (and only for `lang`).

echo "Testing VIRTUAL_ENV forwarding in lang payload..."

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

source fish/core/ipc.fish

function test_lang_payload_carries_virtual_env
    echo "Test 1: lang payload includes virtual_env when VIRTUAL_ENV is set"
    set -gx VIRTUAL_ENV /proj/.venv
    set -l payload (__gpy_build_data_payload lang /proj ansi true '' '')
    if string match -q '*"virtual_env":"/proj/.venv"*' -- "$payload"
        return 0
    end
    echo "❌ lang payload missing virtual_env: $payload"
    return 1
end

function test_git_payload_omits_virtual_env
    echo "Test 2: git payload never carries virtual_env"
    set -gx VIRTUAL_ENV /proj/.venv
    set -l payload (__gpy_build_data_payload git /proj ansi true '' '')
    if string match -q '*virtual_env*' -- "$payload"
        echo "❌ git payload should not carry virtual_env: $payload"
        return 1
    end
    return 0
end

function test_lang_payload_omits_virtual_env_when_unset
    echo "Test 3: lang payload omits virtual_env when VIRTUAL_ENV is unset"
    set -e VIRTUAL_ENV
    set -l payload (__gpy_build_data_payload lang /proj ansi true '' '')
    if string match -q '*virtual_env*' -- "$payload"
        echo "❌ lang payload should omit virtual_env when unset: $payload"
        return 1
    end
    return 0
end

test_lang_payload_carries_virtual_env
or exit 1

test_git_payload_omits_virtual_env
or exit 1

test_lang_payload_omits_virtual_env_when_unset
or exit 1

echo ""
echo "🎉 All VIRTUAL_ENV forwarding tests passed!"
