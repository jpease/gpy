#!/usr/bin/env fish
# Test that the `lang` IPC payload forwards $VIRTUAL_ENV (and only for `lang`), or a non-base
# $CONDA_PREFIX when no VIRTUAL_ENV is set (#729).

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

function test_lang_payload_forwards_conda_prefix
    echo "Test 4: lang payload forwards CONDA_PREFIX for a non-base conda env"
    set -e VIRTUAL_ENV
    set -gx CONDA_PREFIX /opt/conda/envs/ml
    set -gx CONDA_DEFAULT_ENV ml
    set -l payload (__gpy_build_data_payload lang /proj ansi true '' '')
    set -e CONDA_PREFIX CONDA_DEFAULT_ENV
    if string match -q '*"virtual_env":"/opt/conda/envs/ml"*' -- "$payload"
        return 0
    end
    echo "❌ lang payload missing conda virtual_env: $payload"
    return 1
end

function test_virtual_env_beats_conda_prefix
    echo "Test 5: VIRTUAL_ENV takes precedence over CONDA_PREFIX"
    set -gx VIRTUAL_ENV /proj/.venv
    set -gx CONDA_PREFIX /opt/conda/envs/ml
    set -gx CONDA_DEFAULT_ENV ml
    set -l payload (__gpy_build_data_payload lang /proj ansi true '' '')
    set -e VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV
    if string match -q '*"virtual_env":"/proj/.venv"*' -- "$payload"
        and not string match -q '*conda*' -- "$payload"
        return 0
    end
    echo "❌ VIRTUAL_ENV should win over CONDA_PREFIX: $payload"
    return 1
end

function test_conda_base_not_forwarded
    echo "Test 6: conda base is never forwarded"
    set -e VIRTUAL_ENV
    set -gx CONDA_PREFIX /opt/conda
    set -gx CONDA_DEFAULT_ENV base
    set -l payload (__gpy_build_data_payload lang /proj ansi true '' '')
    set -e CONDA_PREFIX CONDA_DEFAULT_ENV
    if string match -q '*virtual_env*' -- "$payload"
        echo "❌ conda base should not be forwarded: $payload"
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

test_lang_payload_forwards_conda_prefix
or exit 1

test_virtual_env_beats_conda_prefix
or exit 1

test_conda_base_not_forwarded
or exit 1

echo ""
echo "🎉 All VIRTUAL_ENV forwarding tests passed!"
