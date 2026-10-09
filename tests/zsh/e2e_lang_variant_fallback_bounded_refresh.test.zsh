#!/usr/bin/env zsh
# tests/zsh/e2e_lang_variant_fallback_bounded_refresh.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Language segment variant fallback: bounded BACKGROUND refresh (#843; the Zsh
# twin of tests/fish/e2e_lang_variant_fallback_bounded_refresh.test.fish, #454).
#
# The read serves the context-free `.none` entry (status bit 4) when no
# token-specific file exists for the render's real prev_bg. Zsh used to treat
# that as an ordinary warm hit and never cause the token file to be written, so
# the wrong-context chevron could persist indefinitely. Like Fish, language
# stays non-blocking: the FIRST render at a new context still shows the `.none`
# chevron and fires a throttled background refresh with the real prev_bg; the
# SECOND render hits the token file and shows the right colour. Two tokens (A,
# then B) prove a context switch neither reuses A's bytes nor is starved by A's
# refresh (the throttle is keyed per token).
#
# The stock theme does not vary the language cap by prev_bg, so the test
# installs a theme whose language opening separator reads `fg:prev_bg`.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

TOKEN_A=231
TOKEN_B=213
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"

mkdir -p "$XDG_CONFIG_HOME/gpy/themes"
sed 's/\[\$sep_open\](fg:\$bg bg:default))\[ \$symbol\](\$style)(\[ \$version\]/[$sep_open](fg:prev_bg bg:default))[ $symbol]($style)([ $version]/' \
    "$ROOT/config/themes/default.toml" >"$XDG_CONFIG_HOME/gpy/themes/test-theme.toml"
if cmp -s "$ROOT/config/themes/default.toml" "$XDG_CONFIG_HOME/gpy/themes/test-theme.toml"; then
    fail "the default theme no longer contains the language opening separator this test rewrites"
    exit 1
fi
printf '[ui]\ntheme = "test-theme"\nenabled_segments = ["language"]\n' >"$XDG_CONFIG_HOME/gpy/config.toml"

# A detectable Rust project with a deterministic fake `rustc`, put on PATH
# before the agent starts so the daemon inherits it.
mkdir -p "$SHELL_E2E_REPO/src" "$SHELL_E2E_ROOT/fakebin"
printf '[package]\nname = "gpy-e2e-test"\nversion = "0.1.0"\n' >"$SHELL_E2E_REPO/Cargo.toml"
printf 'fn main() {}\n' >"$SHELL_E2E_REPO/src/main.rs"
printf '#!/bin/sh\necho "rustc 1.99.0 (fake e2e binary)"\n' >"$SHELL_E2E_ROOT/fakebin/rustc"
chmod +x "$SHELL_E2E_ROOT/fakebin/rustc"
export PATH="$SHELL_E2E_ROOT/fakebin:$PATH"

cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (the theme export would re-set the
# flag from config.toml) so sourcing starts no daemon; enable it afterwards.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/zsh/gpy.zsh"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1

none_present() { [ -n "$(find "$cache_dir" -maxdepth 1 -name '*.lang_last.none.zsh' 2>/dev/null | head -n 1)" ]; }
token_present() { [ -n "$(find "$cache_dir" -maxdepth 1 -name "*.lang_last.$1.zsh" 2>/dev/null | head -n 1)" ]; }
token_absent() { ! token_present "$1"; }
token_a_present() { token_present "$TOKEN_A"; }
token_b_present() { token_present "$TOKEN_B"; }

# Prime the `.none` variants (a request without prev_bg writes only those).
__gpy_trigger_data_refresh lang "$SHELL_E2E_REPO" true ""
if shell_e2e_poll 8 none_present; then
    pass "the agent wrote the .none language cache variant"
else
    fail "no .none language cache file appeared"
    exit 1
fi

for token in "$TOKEN_A" "$TOKEN_B"; do
    label="$([ "$token" = "$TOKEN_A" ] && echo A || echo B)"
    token_absent "$token" || { fail "token $label already has a cache file; no genuine variant gap"; exit 1; }

    # First render at the new context: non-empty, but NOT synchronously corrected.
    out1="$(__gpy_segment_language "true" "$token" "")"
    if [ -z "$out1" ]; then
        fail "first render at token $label produced no output"
        exit 1
    fi
    case "$out1" in
        *"38;5;$token"*) fail "first render at token $label was synchronously corrected; language must stay non-blocking" ;;
        *) pass "first render at token $label serves the .none chevron without blocking" ;;
    esac
    case "$out1" in
        *"38;5;$TOKEN_A"*) [ "$token" = "$TOKEN_B" ] && fail "first render at token B reused token A's chevron" ;;
    esac

    # The bounded background refresh writes the token-specific file...
    if shell_e2e_poll 8 "token_$(echo "$label" | tr 'AB' 'ab')_present"; then
        pass "token $label cache file written by the background refresh"
    else
        fail "no token $label cache file appeared after the variant-fallback render"
        exit 1
    fi

    # ...and the next render shows the corrected chevron.
    out2="$(__gpy_segment_language "true" "$token" "")"
    case "$out2" in
        *"38;5;$token"*) pass "second render at token $label shows the correct chevron" ;;
        *) fail "second render at token $label lacks 38;5;$token: $out2" ;;
    esac
done

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh language segment corrects the variant fallback via a bounded background refresh"
