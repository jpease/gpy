#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Run an unreleased gpy in an isolated fish shell. The installed release, its
# config (~/.config/gpy, ~/.config/fish/gpy) and its agent are never touched:
# `exit` returns to them. Contributor flow; end users install with install.sh.
#
#   gpy-dev.sh                      build this checkout (debug) and enter the shell
#   gpy-dev.sh --release            build with the release profile
#   gpy-dev.sh --ref REV            build REV (branch, tag, sha) from a worktree
#   gpy-dev.sh --pr N               build pull request N (fetched from origin)
#   gpy-dev.sh --bin DIR            skip the build; use DIR/gpy and DIR/gpy-agent
#   gpy-dev.sh --reset              wipe the sandbox first
#   gpy-dev.sh --sync-config        replace the sandbox config with your real one
#
# Isolation: XDG_CONFIG_HOME / XDG_CACHE_HOME hold the config, fish scripts and
# caches; XDG_RUNTIME_DIR holds the agent socket and is short because macOS caps
# unix socket paths at 104 bytes. The prompt carries a `[gpy-dev ...]` marker.
set -euo pipefail

usage() {
    sed -n '5,13p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile=debug
ref=""
pr=""
bin_dir=""
reset=0
sync_config=0

need_value() { [ "$#" -ge 2 ] || usage; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --release) profile=release ;;
        --ref) need_value "$@"; ref="$2"; shift ;;
        --pr) need_value "$@"; pr="$2"; shift ;;
        --bin) need_value "$@"; bin_dir="$2"; shift ;;
        --reset) reset=1 ;;
        --sync-config) sync_config=1 ;;
        *) usage ;;
    esac
    shift
done

[ -z "$ref" ] || [ -z "$pr" ] || { echo "gpy-dev: --ref and --pr are exclusive" >&2; exit 2; }
if [ -n "$bin_dir" ]; then
    [ -z "$ref$pr" ] || { echo "gpy-dev: --bin cannot be combined with --ref/--pr" >&2; exit 2; }
    bin_dir="$(cd "$bin_dir" && pwd)"
    [ -x "$bin_dir/gpy" ] && [ -x "$bin_dir/gpy-agent" ] \
        || { echo "gpy-dev: $bin_dir must hold executable gpy and gpy-agent" >&2; exit 2; }
fi

root="$HOME/.local/share/gpy-dev"
export XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache"
export XDG_RUNTIME_DIR="/tmp/gpy-dev-$USER"
if [ "$reset" = 1 ]; then
    rm -rf "$root" "$XDG_RUNTIME_DIR"
    # The worktrees went with $root; drop their registrations or a later --ref/--pr fails.
    git -C "$repo" worktree prune
fi
mkdir -p "$XDG_CONFIG_HOME/gpy" "$XDG_CONFIG_HOME/fish" "$XDG_CACHE_HOME"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# Seed the sandbox config from the real one: with no sandbox file gpy falls back
# to ~/.config/gpy/config.toml, and `gpy enable`/`config set` would edit it.
real_config="$HOME/.config/gpy/config.toml"
sandbox_config="$XDG_CONFIG_HOME/gpy/config.toml"
if [ -f "$real_config" ]; then
    if [ "$sync_config" = 1 ] && [ -e "$sandbox_config" ]; then
        cp "$sandbox_config" "$sandbox_config.bak"
        cp "$real_config" "$sandbox_config"
    elif [ ! -e "$sandbox_config" ]; then
        cp "$real_config" "$sandbox_config"
    fi
fi

# A ref or PR builds from its own worktree so the checkout and its target/ stay put.
src="$repo"
label="$(git -C "$repo" rev-parse --abbrev-ref HEAD 2>/dev/null || echo checkout)"
if [ -n "$pr" ]; then
    case "$pr" in *[!0-9]* | "") echo "gpy-dev: --pr takes a number" >&2; exit 2 ;; esac
    git -C "$repo" fetch -q origin "pull/$pr/head"
    ref="$(git -C "$repo" rev-parse FETCH_HEAD)"
    label="pr$pr"
elif [ -n "$ref" ]; then
    label="$ref"
fi
if [ -n "$ref" ]; then
    rev="$(git -C "$repo" rev-parse --verify "$ref^{commit}")"
    src="$root/worktrees/$(printf '%s' "$label" | tr -c 'A-Za-z0-9._-' '_')"
    if [ -d "$src" ]; then
        git -C "$src" checkout -q --detach "$rev"
    else
        mkdir -p "$root/worktrees"
        git -C "$repo" worktree add -q --detach "$src" "$rev"
    fi
fi

if [ -n "$bin_dir" ]; then
    bin="$bin_dir"
    label="bin"
    share="$repo"
    [ -d "$bin_dir/../share/gpy/fish" ] && share="$(cd "$bin_dir/../share/gpy" && pwd)"
else
    if [ "$profile" = release ]; then
        env -u CARGO_TARGET_DIR cargo build --manifest-path "$src/gpy-agent/Cargo.toml" --release
    else
        env -u CARGO_TARGET_DIR cargo build --manifest-path "$src/gpy-agent/Cargo.toml"
    fi
    bin="$src/gpy-agent/target/$profile"
    share="$src"
fi

export PATH="$bin:$PATH" GPY_SHARE_DIR="$share"
# Always refresh the fish scripts so the sandbox matches the build.
gpy setup --shell fish --non-interactive >/dev/null
version="$(gpy --version | awk '{print $2}')"
export GPY_DEV="$version $label"

# Prefix the prompt with the marker, once, after gpy's own init has defined it.
marker_cfg="$XDG_CONFIG_HOME/fish/config.fish"
if ! grep -q 'gpy-dev-marker' "$marker_cfg" 2>/dev/null; then
    cat >>"$marker_cfg" <<'EOF'

# gpy-dev-marker: show which build this sandbox runs.
if status is-interactive; and set -q GPY_DEV
    function __gpy_dev_replay_status
        return $argv[1]
    end
    functions -c fish_prompt __gpy_dev_inner_prompt
    function fish_prompt
        set -l last_status $status
        set_color yellow
        printf '[gpy-dev %s] ' $GPY_DEV
        set_color normal
        # Replay the status so the wrapped prompt still sees the last command's.
        __gpy_dev_replay_status $last_status
        __gpy_dev_inner_prompt $argv
    end
end
EOF
fi

gpy restart >/dev/null 2>&1 || gpy start >/dev/null 2>&1 || true
echo "gpy-dev: $(gpy --version) [$label] from $bin" >&2
echo "gpy-dev: config $XDG_CONFIG_HOME/gpy; 'exit' returns to your release install" >&2
exec fish
