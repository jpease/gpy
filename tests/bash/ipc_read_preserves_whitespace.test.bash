#!/usr/bin/env bash
# Every IPC response read must use `IFS= read -r`.
#
# A bare `read` splits on the default IFS, which strips leading and trailing
# whitespace from the line it assigns. Agent responses are rendered prompt
# fragments where that whitespace is significant: the character segment's
# template ends in a space so the prompt reads `❯ cmd`, and a bare `read` ate
# it, giving `❯cmd` in zsh while fish rendered it correctly. Dropping `-r`
# additionally mangles backslashes, which now matters because the clock
# response embeds bash's `\D{…}` prompt token.
#
# Every read site in both shells already followed this convention except the
# zsh zsocket fd read — the fastest path, and therefore the one that actually
# rendered. This is a shape guard rather than a behavioral test because the
# alternative needs a live agent on a real socket; it pins the invariant at
# the only layer where it can be checked cheaply.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

status=0

echo "=== IPC reads preserve significant whitespace ==="

for f in zsh/core/ipc.zsh bash/core/ipc.bash; do
    if [[ ! -f "$f" ]]; then
        echo "FAIL: expected IPC file missing: $f"
        status=1
        continue
    fi

    # Every line that assigns a response via `read`, minus the compliant ones.
    # Comments are excluded so prose about `read` does not trip the guard.
    offenders="$(grep -n '[^#]*\bread\b.*response' "$f" \
        | grep -v '^\s*[0-9]*:\s*#' \
        | grep -v 'IFS= read -r' || true)"

    if [[ -n "$offenders" ]]; then
        echo "FAIL: $f has IPC read(s) that are not 'IFS= read -r':"
        echo "$offenders"
        status=1
    else
        echo "✓ $f: all response reads use 'IFS= read -r'"
    fi
done

if [[ "$status" -eq 0 ]]; then
    echo "PASS"
fi
exit "$status"
