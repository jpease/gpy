#!/bin/bash
# Build and test gpy-agent on Windows via a local Parallels VM over SSH.
#
# The repo is expected to be reachable inside the VM through a Parallels
# shared folder, so no file transfer happens here -- this script only runs
# remote commands. Mirrors the windows-latest job in
# .github/workflows/cross-platform-test.yml so local results are predictive
# of CI.
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
# rustc's rlib archive step fails with "os error 87" when writing directly to
# a \\Mac\... SMB share -- build output goes to a local disk instead; source
# is still read live from the shared folder, so no sync step is needed.
WINVM_TARGET_DIR="${GPY_WINVM_TARGET_DIR:-C:\\gpy-target}"

info "Target: $WINVM_HOST ($WINVM_PATH)"

if ! ssh -o ConnectTimeout=5 -o BatchMode=yes "$WINVM_HOST" "echo ok" >/dev/null 2>&1; then
    error "Cannot reach $WINVM_HOST over SSH."
    echo "  Check: the VM is running, OpenSSH Server (sshd) is started, and" >&2
    echo "  ~/.ssh/config has a Host entry -- or set GPY_WINVM_HOST=user@ip." >&2
    exit 1
fi
success "SSH connection to $WINVM_HOST OK"

# Windows OpenSSH sets HOME in the session it hands us; windows-latest has no
# HOME, and no XDG_* either. That single difference decided two tests: the run
# measured for #513 saw 1 failure here against 8 on the runner, because with
# HOME set the cache- and config-root tests took a branch the runner cannot
# reach (#530). Clear the three variables so a local run starts from the
# runner's environment shape.
#
# Set GPY_WINVM_KEEP_HOME=1 to deliberately test the HOME-present path -- that
# is the shape a real native-Windows user running under an interactive shell
# gets, so it is worth exercising too, just not as the default.
if [ "${GPY_WINVM_KEEP_HOME:-0}" = "1" ]; then
    ENV_PRELUDE=""
    info "Keeping HOME/XDG_* as the VM presents them (GPY_WINVM_KEEP_HOME=1)"
else
    ENV_PRELUDE="Remove-Item Env:\\HOME,Env:\\XDG_CONFIG_HOME,Env:\\XDG_CACHE_HOME -ErrorAction SilentlyContinue; "
    info "Clearing HOME/XDG_* to match the windows-latest runner"
fi

# Assumes the remote default shell resolves powershell.exe (true for a stock
# OpenSSH-Win32 install using the cmd.exe fallback shell). If sshd_config's
# DefaultShell is set to PowerShell instead, drop the leading "powershell
# -NoProfile -Command" wrapper below.
run_remote() {
    local label="$1"
    local cargo_cmd="$2"
    info "Running: $label"
    # SC2029: the client-side expansion is deliberate -- $WINVM_PATH and the
    # cargo command are resolved here and sent as a literal remote command.
    # shellcheck disable=SC2029
    if ssh "$WINVM_HOST" "powershell -NoProfile -Command \"cd '$WINVM_PATH'; \$env:CARGO_TARGET_DIR = '$WINVM_TARGET_DIR'; $ENV_PRELUDE$cargo_cmd; exit \$LASTEXITCODE\""; then
        success "$label passed"
    else
        error "$label failed"
        exit 1
    fi
}

# Report what `bash` resolves to before the run. The theme::export tests source
# real shells, and four of them fail on windows-latest while passing here
# against Git Bash (#528) -- so a VM/CI divergence in shell resolution has to be
# visible in this output, not inferred afterwards from identical assertions.
info "Probing remote shell resolution"
ssh "$WINVM_HOST" "powershell -NoProfile -Command \"\$bash = (Get-Command bash -ErrorAction SilentlyContinue).Source; if (\$bash) { Write-Output \\\"bash: \$bash\\\"; & bash --version 2>&1 | Select-Object -First 1; Write-Output \\\"bash --version exit: \$LASTEXITCODE\\\" } else { Write-Output 'bash: not on PATH' }\"" ||
    warn "Could not probe remote shell resolution; continuing"

run_remote "cargo build --locked --verbose" "cargo build --locked --verbose"
run_remote "cargo test --locked --lib --verbose" "cargo test --locked --lib --verbose"

success "Windows build + lib tests passed via $WINVM_HOST"
