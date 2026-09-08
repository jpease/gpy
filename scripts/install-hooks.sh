#!/usr/bin/env bash
# Configure git to use the repository-provided hooks (pre-push quality checks).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

usage() {
    cat <<'EOF'
Usage: ./scripts/install-hooks.sh [--uninstall]

Installs the repository git hooks via prek:
  - pre-commit: fast checks filtered to the file types that changed (clippy,
                rustfmt, fish syntax/format, attribution and secret scans)
  - pre-push:   path-aware gate: shell-only pushes run 'just check-shell';
                anything touching Rust, Cargo.*, config/ or scripts/ also
                runs 'just check-rust', 'just lint' (moon, #584) and a
                same-machine perf canary. See .pre-commit-config.yaml.
Run once per clone.

Options:
  --uninstall    Remove the hooksPath override so git uses default hooks.
EOF
}

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
    --uninstall)
        if command -v prek >/dev/null 2>&1; then
            echo "[gpy] Uninstalling prek hooks..."
            prek uninstall
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

        echo "[gpy] Installing prek hooks..."
        prek install --hook-type pre-commit --hook-type pre-push

        # Clean up old manual hooks configuration if it exists
        if [[ "$(git config --get core.hooksPath)" == ".githooks" ]]; then
            git config --unset core.hooksPath
            echo "[gpy] Cleaned up legacy .githooks configuration."
        fi

        echo "[gpy] Pre-commit hooks installed successfully via prek."
        ;;
    *)
        usage
        exit 1
        ;;
esac
