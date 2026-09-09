#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds an isolated fixture $HOME, installs the current build into it, seeds
# a small tour repo, and records demo/demo.tape with VHS into assets/demo.gif.
#
# Never touches your real ~/.config/gpy, ~/.local/bin, or default tmux
# server: HOME/XDG_*/PATH are all repointed at demo/.fixture-home (wiped and
# rebuilt on every run) before anything is installed or recorded, and the
# recording's tmux session runs on its own `-L gpydemo` socket.
#
# Usage:
#   just demo                     # from the repo root
#   ./demo/setup.fish             # equivalent; builds a fresh release binary
#   ./demo/setup.fish --no-build  # reuse an existing target/release binary

set -l repo_root (path resolve (path dirname (status --current-filename))/..)
set -l fixture_home "$repo_root/demo/.fixture-home"
set -l workspace "$fixture_home/workspace/gpy-tour"

for cmd in vhs tmux fish git
    command -q $cmd; or begin
        echo "❌ $cmd not found; see demo/README.md for prerequisites." >&2
        exit 1
    end
end

cd $repo_root

# Captured before HOME is overridden below: some toolchains here (cargo,
# rustc, node -- whichever a given machine has under mise) are reached via
# `mise` shims that resolve the OS-level home directory rather than $HOME,
# so they ignore our fixture isolation and hit mise's own interactive
# config-trust prompt once HOME changes. Resolve what mise would actually
# run and prioritize those directories instead, bypassing the shims.
set -l real_toolchain_dirs
if command -q mise
    for tool in cargo rustc node python3
        set -l resolved (mise which $tool 2>/dev/null)
        test -n "$resolved"; and set -a real_toolchain_dirs (path dirname $resolved)
    end
end

echo "🎬 Preparing isolated demo fixture at $fixture_home"
rm -rf $fixture_home
mkdir -p $fixture_home

# --- Isolate the recording from the real environment ----------------------
set -gx HOME $fixture_home
set -gx XDG_CONFIG_HOME "$fixture_home/.config"
set -gx XDG_CACHE_HOME "$fixture_home/.cache"
set -gx XDG_DATA_HOME "$fixture_home/.local/share"
set -gx XDG_RUNTIME_DIR "$fixture_home/.run"
set -gx SHELL (command -v fish)
mkdir -p $XDG_RUNTIME_DIR

# Homebrew's mise install ships a fish *vendor* conf.d snippet
# (opt/homebrew/share/fish/vendor_conf.d/mise-activate.fish) that
# unconditionally runs `mise activate fish` on every fish session --
# vendor confdirs aren't under $XDG_CONFIG_HOME, so HOME isolation alone
# doesn't stop it, and its prompt/PWD hooks fight with GPY's. Opt out.
set -gx MISE_FISH_AUTO_ACTIVATE 0

# Fixture binaries take priority, then the real (non-mise-shim) toolchain
# dirs resolved above, then the rest of the real PATH as a fallback so
# fish, tmux, git, and anything else still resolve.
set -gx PATH "$fixture_home/.local/bin" $real_toolchain_dirs $PATH

# --- Install the current build into the fixture ---------------------------
# install-dev.fish already honors HOME/XDG_* for every install path, so
# reusing it here gets us a real, up-to-date install without duplicating its
# logic -- it never touches the real ~/.config/gpy or ~/.local/bin.
./install-dev.fish $argv; or exit 1

# --- Demo-only config tweaks ------------------------------------------------
# Show only the language judged primary for a repo. The default (`all`) puts up
# to three language pills on every prompt, which crowds the frame and competes
# with the beat headers for attention.
gpy config set language.filter primary >/dev/null; or exit 1

# --- Seed a small multi-language tour workspace -----------------------------
# api/service/engine are three INDEPENDENT git repos, not sub-dirs of one
# shared repo: GPY's default language detection scans a whole project (repo
# root down), so nesting them under one repo would show every sub-project's
# language at once in each dir instead of demonstrating detection actually
# changing as you move around.
echo "🌱 Seeding tour workspace at $workspace"
mkdir -p $workspace/api $workspace/service $workspace/engine/src

printf '%s\n' \
    '{' \
    '  "name": "gpy-tour-api",' \
    '  "version": "0.1.0",' \
    '  "engines": { "node": ">=18" }' \
    '}' >$workspace/api/package.json
printf '%s\n' 'console.log("gpy tour api");' >$workspace/api/index.js

printf '%s\n' \
    '[project]' \
    'name = "gpy-tour-service"' \
    'version = "0.1.0"' \
    'requires-python = ">=3.11"' >$workspace/service/pyproject.toml
printf '%s\n' 'print("gpy tour service")' >$workspace/service/main.py

printf '%s\n' \
    '[package]' \
    'name = "gpy-tour-engine"' \
    'version = "0.1.0"' \
    'edition = "2021"' >$workspace/engine/Cargo.toml
printf '%s\n' 'fn main() { println!("gpy tour engine"); }' >$workspace/engine/src/main.rs
printf '%s\n' \
    '# Notes' \
    '' \
    'Tracked so demo/demo.tape can append to it and show a live, ' \
    'no-keypress prompt update across both tmux panes.' >$workspace/engine/NOTES.md

for dir in api service engine
    git -C $workspace/$dir init -q
    git -C $workspace/$dir add -A
    git -C $workspace/$dir \
        -c user.name="GPY Tour" -c user.email="tour@example.invalid" \
        commit -q -m "Initial $dir project"
end

# --- Reuse real importer fixtures from the test suite ----------------------
cp gpy-agent/tests/fixtures/starship/preset_gruvbox_rainbow.toml \
    $workspace/starship-preset.toml
cp gpy-agent/tests/fixtures/base16/tokyo-night-dark.yaml \
    $workspace/tokyo-night.yaml

# The workspace root itself isn't a project repo -- give it its own clean
# git repo (rather than leaving it bare) so GPY's git segment stops here
# instead of walking up and picking up this real gpy checkout's own status
# (the fixture lives under demo/.fixture-home/, inside this very repo).
# api/service/engine are their own repos (gitlinks), so they're ignored
# here rather than tracked.
printf '%s\n' api/ service/ engine/ >$workspace/.gitignore
git -C $workspace init -q
git -C $workspace add -A
git -C $workspace \
    -c user.name="GPY Tour" -c user.email="tour@example.invalid" \
    commit -q -m "Tour workspace root"

echo "✅ Fixture ready"
echo "🎥 Recording demo/demo.tape → assets/demo.gif"
vhs demo/demo.tape
