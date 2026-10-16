#!/usr/bin/env bash
# Install the repository git hooks. prek owns the hook shims and runs the
# checks configured in .pre-commit-config.yaml.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$REPO_ROOT"

HOOK_TYPES=(pre-commit pre-push)
PREK_HOOK_FLAGS=()
for hook_type in "${HOOK_TYPES[@]}"; do
    PREK_HOOK_FLAGS+=(--hook-type "$hook_type")
done

usage() {
    cat <<'EOF'
Usage: ./scripts/install-hooks.sh [--uninstall]

Installs the repository git hooks via prek:
  - pre-commit: fast checks filtered to the file types that changed (clippy,
                rustfmt, fish syntax/format, and secret scans)
  - pre-push:   repeats the clippy pass; the heavy gates (Rust gate,
                shell suites, perf canary) run in CI and via 'just'
See .pre-commit-config.yaml. Run once per clone.

Options:
  --uninstall    Remove the prek shims and any custom hooksPath.
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
            # commit-msg: older installs also had one.
            prek uninstall "${PREK_HOOK_FLAGS[@]}" --hook-type commit-msg
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

        # prek uses the default hooks dir; drop a legacy .githooks hooksPath.
        if [[ "$(git config --get core.hooksPath || true)" == ".githooks" ]]; then
            git config --unset core.hooksPath
            echo "[gpy] Cleaned up legacy .githooks configuration."
        fi

        echo "[gpy] Installing prek hooks..."
        prek install "${PREK_HOOK_FLAGS[@]}"

        echo "[gpy] Git hooks installed successfully via prek."
        ;;
    *)
        usage
        exit 1
        ;;
esac
