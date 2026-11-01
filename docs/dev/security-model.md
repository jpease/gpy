# Security Model

Design record behind [SECURITY.md](../../SECURITY.md); the policy there wins on conflict.

## Trust boundaries in detail

GPY is three cooperating pieces: a shell integration written for Fish, Bash and
Zsh; a long-running local agent (`gpy-agent`); and a Unix domain socket between
them.

**Your shell trusts the agent completely.** Every new shell runs the agent's
theme export as shell code. Bash and Zsh do it with `eval "$(gpy-agent theme
export ...)"` (`bash/core/init.bash`, `zsh/core/init.zsh`). Fish pipes the same
output into `source`, or sources the theme-export cache file the agent wrote
under `~/.cache/gpy/` (`fish/core/init.fish`). Prompt content returned over the
socket reaches the prompt the same way. This is what a prompt generator is —
something has to emit shell code — but it puts the agent binary on the same
footing as your shell startup files.

Anyone who can replace `gpy-agent` on your
`PATH`, or write to the theme-export cache, gets arbitrary code execution in
every shell you open from then on. Three things stand in the way, and only
these three:

- The installers verify every downloaded binary (and, for the one-line
  installer, the shell-script archive) against its published SHA-256 and abort
  on a mismatch or a missing checksum. The digests ship in the same release,
  so this catches corruption, not a compromised release. See
  [Release Integrity and Supply Chain](../../SECURITY.md#release-integrity-and-supply-chain).
- Both the cache directory and the install directory are ordinary user-owned
  paths. Anything already running as you can write to them.
- Nothing else. There is no signature check at load time and no integrity check
  on the cache file.

**Plugin segment files run as you.** Third-party plugins ship Fish scripts
(`segments/<name>.fish`) that `fish/core/init.fish` sources, with your full user
privileges, when the segment is enabled. Only the Fish theme export carries
their paths (`__gpy_plugin_segment_file_*`); Bash and Zsh never source these
`.fish` files. Installing a plugin means trusting its code like a shell startup
file.

**Script segments run as you, in all three shells.** Bash, Zsh and Fish
execute script plugins (`entry_type = "script"`) as you, once per command; see
[Script Segment Trust Model](../../SECURITY.md#script-segment-trust-model).

**The agent trusts whoever can open its socket.** The socket is chmod'ed to
`0600` immediately after bind. That mode does not stop anyone who can write the
socket's directory from replacing the socket, so the directory is checked for
every socket path, the default `$XDG_RUNTIME_DIR/gpy` (or its cache, home or
`/tmp/gpy` fallback) and an explicit `GPY_AGENT_SOCKET_PATH` or `--socket`
alike. The agent refuses to use a directory that is a symlink, is not a
directory, or is owned by another user, so a socket directly in `/tmp` is
refused. A directory owned by you that is group- or world-writable is treated
by where the path came from: GPY's own default runtime directory is set to
`0700` and checked again before use, while the directory of an explicit socket
path is refused, never chmod'ed (`chmod 700` it, or choose a private
directory). `gpy-agent start` applies the check before it creates the start
lock, removes a stale socket or pings a running agent there, and the daemon
applies it again immediately before bind (`secure_socket_dir` in
`gpy-agent/src/ipc/server/handle.rs`). Only the directory itself is checked,
not its parents. The owner-only socket in that directory is the access
control. There is no handshake and no peer-credential check: any process
running as you can talk to the agent, which is the same authority it already
has to read your files and rewrite your shell config.

Requests carry a PID. `validate_pid` (`gpy-agent/src/security.rs`) checks that
the PID is well-formed and that the process is alive via `kill(pid, 0)`; it does
not authenticate the caller, and a local process may claim any live PID. Treat
it as a liveness and hygiene check, not as an authentication mechanism.

**The Zsh integration writes no memo files.** Zsh renders the prompt in the
interactive shell and memoizes the directory and prompt-character renders in
shell variables, so no file under `/tmp` or `$TMPDIR` can be pre-created by a
local attacker and read back into `$PROMPT` (`zsh/core/init.zsh`). Script
segments add no files either: their cache stays in
shell variables, with no `/tmp` path and no PID-keyed state (see
[Script Segment Trust Model](../../SECURITY.md#script-segment-trust-model)).

**Downloaded release artifacts are trusted only after verification.** What the
installers enforce, and how to check a download by hand, is in
[Release Integrity and Supply Chain](../../SECURITY.md#release-integrity-and-supply-chain).

## Script segment mechanism

*Which file runs (containment and ownership).* `gpy theme export` re-resolves
`command` and `require_command` on every export (`ScriptPath::resolve` and
`check_owned_and_not_shared` in `gpy-agent/src/plugin/script.rs`). Each must be
a relative path without `..`, canonicalize (all symlinks resolved) to a regular
file inside the canonical plugin root, and sit at a UTF-8 path without control
characters. The script, and every directory from it up to the plugin root,
must be owned by you or by root and must not be group- or world-writable.
On macOS the same applies to extended ACLs, for the script, its directories
and every ancestor: an allow entry for another principal granting write,
append, delete, delete-child, write-security or change-owner is refused
(deny entries are ignored; `chmod -N` removes an ACL).
Every canonical ancestor of the plugin root, up to `/`, must also be unable
to let another user replace the root: owned by you or root and not group- or
world-writable, or writable only with the sticky bit set (as in `/tmp`) and
owned by you or root, or group-writable (not world-writable) through your own
user-private group (named like you, no other member), as `$HOME` is with
umask 002 on user-private-group systems. Renaming or replacing a directory needs write
permission on its parent, not on the directory itself, so without the
ancestor rule another user with write access to a parent could swap the root
after export. A segment failing any check is not exported, so no shell runs
it. The export writes the canonical absolute path, and the shells run exactly
that path without resolving it again. Re-resolving at run time would add no
protection: after export, replacing the script or a directory on its path
needs write access to a file or directory (or its parent) that the rules have
just shown only you (or root) can change. Anyone able to do that already
runs code as you, which is outside this design (see "No defence against a
compromised account" below). These rules are what keep other local users out:
without them, a user who could write the plugin directory or its parent could
run code as you on every command. The interpreter is `/bin/sh` by an absolute
path the agent resolves at export time (on macOS, the target of
`/private/var/select/sh` when it passes the same ownership and mode checks),
never a bare `sh` looked up on `PATH`.

*Time and processes.* Triggers run in a fixed order: `require_env`, then the
`require_files`/`require_folders`/`require_extensions` scan, then the cache,
then `require_command`. `require_command` and the script run in one process
group under the script's `timeout_ms` deadline (at most 500 ms); all due
segments of one prompt start together, so one prompt waits at most 500 ms
plus the grace, and at most 40 segments run per prompt. On timeout or Ctrl-C
the group gets TERM, CONT,
then KILL after a 50 ms grace; members left behind after a normal exit are
killed the same way, and the runner never waits for a grandchild that holds
the output pipe open. The scan runs in a child with a 30 ms deadline and is
killed with SIGKILL when it overruns; it walks up through physical parents.
Two consecutive scan timeouts, or two consecutive run timeouts or over-cap
outputs (separate counters), disable the segment for that shell. Bash runs
the runner in a subshell of itself; Fish and Zsh run it in a Bash child, the
first Bash 4 or later found once at shell start, else `/bin/bash`. Without
any Bash, script segments are disabled. With Bash 3.2 (stock macOS) Ctrl-C
during a slow script returns to the prompt only when the script's timeout
expires, and the group is still killed.

## Hardening

### IPC

- **Unix domain sockets** — local only, no network listener
- **Socket mode `0600`** in a directory the agent refuses to use when it is a
  symlink or owned by another user; when it is yours but group- or
  world-writable, the default runtime directory is set to `0700` and an
  explicit socket path's directory is refused
- **Path validation** to prevent directory traversal
- **Message content validated against the protocol schema**
  (`gpy-agent/src/ipc/protocol/mod.rs`, `gpy-agent/schemas/`)
- **Connection rate limiting** to prevent abuse; an over-limit connection is
  closed without a reply, and `gpy-agent start` treats that as a busy agent,
  not a wedged one
- **Message size limits** to prevent DoS
- **Operation timeouts** so a slow or hostile request cannot pin the agent

### File system access

The file watcher's liveness probe does write a sentinel file, but into a private
directory under the system temp dir, outside every watched repository
(`gpy-agent/src/watcher/filesystem/mod.rs`). The other file-system promises are
in [SECURITY.md](../../SECURITY.md#file-system-access).

### Subprocesses

- Commands are built as argument vectors through `std::process::Command`, so
  repository, branch and path names are passed as arguments and never spliced
  into a shell string.
- Python version detection reads a project-local `.venv`/`venv` only through
  its `pyvenv.cfg`. A venv's interpreter is run to read its version only for
  the environment your shell activated and forwarded (`VIRTUAL_ENV`, or a
  non-base conda env); a project venv without a usable `pyvenv.cfg` version
  shows no version.

### Dependencies

- `cargo audit` and `cargo deny` run in the local quality gate, not only in CI
- Minimal direct dependency footprint
- The current warning set is whatever `cargo audit` prints against
  `gpy-agent/Cargo.lock` — that is the source of truth, not a count copied into
  this file. Any suppressed ID would be listed with its rationale in
  `gpy-agent/deny.toml`.

For the per-advisory assessment and audit history, see [Security Audit Notes](security-audit.md).

### CI and repository policy

- **Workflow permissions:** every workflow declares a read-only top-level
  `permissions: contents: read`. Only the release's `create-release` job widens
  it (`contents: write` to publish, `id-token: write` and `attestations: write`
  for build provenance), and `actions/checkout` uses
  `persist-credentials: false` everywhere because no job pushes.
- **Pinned actions:** every `uses:` is a full 40-character commit SHA with a
  `# vX.Y.Z` comment. Dependabot (`.github/dependabot.yml`) opens weekly
  GitHub Actions update PRs; Cargo version-update PRs are deliberately not
  enabled, since `cargo audit` and `cargo deny` already gate every push.
- **Secret scanning:** GitHub secret scanning with push protection is enabled
  on the repository and is the authoritative control. Contributors who
  have `gitleaks` installed also get it run on staged changes by the
  repository's pre-commit hook (`.pre-commit-config.yaml`). A separate
  Gitleaks CI job is deliberately not
  added: it would duplicate push protection, add one more pinned action or
  binary download to maintain, and only report after a secret is already in a
  pushed branch.
- **Untrusted input:** workflow `run:` steps never interpolate `${{ ... }}`
  expressions from event data; values are passed through `env:`.
