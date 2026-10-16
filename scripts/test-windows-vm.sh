#!/bin/bash
# Build and test gpy-agent on Windows via a local Parallels VM over SSH,
# mirroring the windows-latest job in .github/workflows/cross-platform-test.yml.
# The repo is read through a Parallels shared folder; nothing is copied.
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RESET='\033[0m'

info() { printf "${BLUE}i${RESET} %s\n" "$1"; }
success() { printf "${GREEN}\xe2\x9c\x93${RESET} %s\n" "$1"; }
warn() { printf "${YELLOW}!${RESET} %s\n" "$1"; }
error() { printf "${RED}x${RESET} %s\n" "$1"; }

# Override via env: GPY_WINVM_HOST=user@1.2.3.4 GPY_WINVM_PATH='\\Mac\gpy\gpy-agent'
WINVM_HOST="${GPY_WINVM_HOST:-gpy-winvm}"
WINVM_PATH="${GPY_WINVM_PATH:-\\\\Mac\\gpy\\gpy-agent}"
# rustc's rlib step fails with "os error 87" writing to a \\Mac\... SMB share,
# so build output goes to a local disk.
WINVM_TARGET_DIR="${GPY_WINVM_TARGET_DIR:-C:\\gpy-target}"

info "Target: $WINVM_HOST ($WINVM_PATH)"

if ! ssh -o ConnectTimeout=5 -o BatchMode=yes "$WINVM_HOST" "echo ok" >/dev/null 2>&1; then
    error "Cannot reach $WINVM_HOST over SSH."
    echo "  Check: the VM is running, OpenSSH Server (sshd) is started, and" >&2
    echo "  ~/.ssh/config has a Host entry -- or set GPY_WINVM_HOST=user@ip." >&2
    exit 1
fi
success "SSH connection to $WINVM_HOST OK"

# windows-latest has no HOME or XDG_*; clear them so cache/config-root tests
# take the runner's branch. GPY_WINVM_KEEP_HOME=1 tests the HOME-present path.
if [ "${GPY_WINVM_KEEP_HOME:-0}" = "1" ]; then
    ENV_PRELUDE=""
    info "Keeping HOME/XDG_* as the VM presents them (GPY_WINVM_KEEP_HOME=1)"
else
    ENV_PRELUDE="Remove-Item Env:\\HOME,Env:\\XDG_CONFIG_HOME,Env:\\XDG_CACHE_HOME -ErrorAction SilentlyContinue; "
    info "Clearing HOME/XDG_* to match the windows-latest runner"
fi

# Assumes the remote default shell resolves powershell.exe (stock cmd.exe
# fallback); drop the wrapper if sshd_config's DefaultShell is PowerShell.
run_remote() {
    local label="$1"
    local cargo_cmd="$2"
    info "Running: $label"
    # Client-side expansion is deliberate.
    # shellcheck disable=SC2029
    if ssh "$WINVM_HOST" "powershell -NoProfile -Command \"cd '$WINVM_PATH'; \$env:CARGO_TARGET_DIR = '$WINVM_TARGET_DIR'; $ENV_PRELUDE$cargo_cmd; exit \$LASTEXITCODE\""; then
        success "$label passed"
    else
        error "$label failed"
        exit 1
    fi
}

# Surface bash resolution: theme::export tests source real shells, and Git Bash
# vs. CI divergence must be visible in the output.
info "Probing remote shell resolution"
ssh "$WINVM_HOST" "powershell -NoProfile -Command \"\$bash = (Get-Command bash -ErrorAction SilentlyContinue).Source; if (\$bash) { Write-Output \\\"bash: \$bash\\\"; & bash --version 2>&1 | Select-Object -First 1; Write-Output \\\"bash --version exit: \$LASTEXITCODE\\\" } else { Write-Output 'bash: not on PATH' }\"" ||
    warn "Could not probe remote shell resolution; continuing"

run_remote "cargo build --locked --verbose" "cargo build --locked --verbose"
run_remote "cargo test --locked --lib --verbose" "cargo test --locked --lib --verbose"

success "Windows build + lib tests passed via $WINVM_HOST"
