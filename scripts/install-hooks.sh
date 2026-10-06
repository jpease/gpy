#!/usr/bin/env bash
# Install the repository git hooks. prek owns the hook shims; Raven's checks
# under .raven/git-hooks/ run through .pre-commit-config.yaml entries (#666).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

HOOK_TYPES=(pre-commit pre-push commit-msg)
PREK_HOOK_FLAGS=()
for hook_type in "${HOOK_TYPES[@]}"; do
    PREK_HOOK_FLAGS+=(--hook-type "$hook_type")
done

usage() {
    cat <<'EOF'
Usage: ./scripts/install-hooks.sh [--uninstall]

Installs the repository git hooks via prek:
  - pre-commit: fast checks filtered to the file types that changed (clippy,
                rustfmt, fish syntax/format, attribution, blanket-suppression
                and secret scans)
  - pre-push:   path-aware gate: shell-only pushes run 'just check-shell';
                anything touching Rust, Cargo.*, config/ or scripts/ also
                runs 'just check-rust', 'just lint' (moon, #584) and a
                same-machine perf canary
  - commit-msg: strips AI attribution trailers (.raven/git-hooks/commit-msg)
See .pre-commit-config.yaml. Run once per clone.

Hook symlinks that Raven installed (.git/hooks/* -> .raven/git-hooks/*) are
replaced by prek shims; Raven leaves regular hook files alone afterwards.

Options:
  --uninstall    Remove the prek shims and any custom hooksPath.
EOF
}

# Drop Raven's hook symlinks before prek installs its shims. `prek install`
# would otherwise either rename them to <hook>.legacy and run Raven's whole
# `just check` gate on top of prek's (migration mode), or, with --force, write
# its shim *through* the symlink and overwrite the tracked script in
# .raven/git-hooks/. A regular, non-prek hook file is left for prek's
# migration mode so a contributor's own hook keeps running.
remove_hook_symlinks() {
    local hooks_dir type path
    hooks_dir="$(git rev-parse --git-path hooks)"
    for type in "${HOOK_TYPES[@]}"; do
        for path in "$hooks_dir/$type" "$hooks_dir/$type.legacy"; do
            if [[ -L "$path" ]]; then
                echo "[gpy] Replacing hook symlink $path -> $(readlink "$path")"
                rm -f "$path"
            fi
        done
    done
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
    --uninstall)
        if command -v prek >/dev/null 2>&1; then
            echo "[gpy] Uninstalling prek hooks..."
            prek uninstall "${PREK_HOOK_FLAGS[@]}"
        fi

        if git config --get core.hooksPath >/dev/null 2>&1; then
            git config --unset core.hooksPath
            echo "[gpy] Removed custom hooksPath; default git hooks restored."
        else
            echo "[gpy] No custom hooksPath set."
        fi
        exit 0
        ;;
    "")
        if ! command -v prek >/dev/null 2>&1; then
            echo "[gpy] 'prek' not found. Installing prek via cargo..."
            cargo install prek
        fi

        # Clean up old manual hooks configuration if it exists; prek installs
        # into the default hooks directory.
        if [[ "$(git config --get core.hooksPath || true)" == ".githooks" ]]; then
            git config --unset core.hooksPath
            echo "[gpy] Cleaned up legacy .githooks configuration."
        fi

        remove_hook_symlinks

        echo "[gpy] Installing prek hooks..."
        prek install "${PREK_HOOK_FLAGS[@]}"

        echo "[gpy] Git hooks installed successfully via prek."
        ;;
    *)
        usage
        exit 1
        ;;
esac
