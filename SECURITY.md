# Security Policy

## Supported Versions

GPY is a personal project with one release line. Security fixes land on the latest release; there is no long-term-support branch and no backporting to older versions.

| Version | Supported |
| ------- | --------- |
| 0.7.x   | Yes       |
## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

### How to Report

Use GitHub private vulnerability reporting:

<https://github.com/jpease/gpy/security/advisories/new>

That form is the only reporting channel; no email address is published for
security reports. A report stays private to you and the maintainer until an
advisory is published.

Include what you have:

1. **Description** of the vulnerability
2. **Steps to reproduce** the issue
3. **Potential impact** assessment
4. **Suggested fix** (if you have one)

### What to Expect

GPY is a personal project with a single maintainer working on it in spare
time. No response window is guaranteed, no triage or fix timeline is promised,
and a report may sit unread for some time.

When a report is picked up: it is investigated in the private advisory it
arrived in, a fix is prepared there where one is warranted, and the advisory is
published once that fix ships. Reporters are credited in the advisory unless
they ask not to be.

### Disclosure Policy

- Discussion stays inside the private advisory while a fix is being prepared
- Disclosure timing is coordinated with the reporter where possible
- An advisory is published once a fix ships
- Public disclosure before a fix exists is discouraged, but nothing here obliges
  a reporter to wait indefinitely on a maintainer who has gone quiet

## Trust Boundaries

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
  [Release Integrity and Supply Chain](#release-integrity-and-supply-chain).
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
[Script Segment Trust Model](#script-segment-trust-model).

**Prompt data is escaped for the shell that draws it.** Bash and Zsh paste the
agent's prompt output into `PS1`/`PROMPT`, and both shells expand prompt
strings on every draw. Directory names, git branch names, language and
virtualenv names, hostnames, usernames and theme literals are untrusted text —
a cloned repository or an unpacked archive chooses them — so the agent escapes
every character the shell would expand in them (`\`, `$` and backticks for
Bash; those plus `%` for Zsh) through the `bash-prompt` and `zsh-prompt` output
formats (`gpy-agent/src/formatter/style_encoder.rs`). The instant-prompt
cache keeps a separate file per format. The integrations pin the prompt options
that escaping relies on: `shopt -s promptvars` in Bash, and `prompt_subst`,
`prompt_percent` and `no_prompt_bang` in Zsh. The clock's live-time token
(`\D{…}`/`%D{…}`) is the only agent text left for the shell to expand. Fish
prints its prompt literally and receives the unescaped `ansi` format. In every
format the agent also strips terminal control characters (ESC, BEL and the other
C0/C1 controls) from resolved values and from theme template text, so neither
can emit escape sequences (script segment output is sanitized by the shells;
see [Script Segment Trust Model](#script-segment-trust-model) for its gaps);
theme template text keeps only newline, and
`gpy theme validate` rejects themes whose templates contain any other control
character (`gpy-agent/src/template/eval.rs`).

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

**The Zsh integration does not trust `/tmp`.** Zsh memoizes prompt segments
through relay files kept in a private directory created by `mktemp -d` — mode
0700, unpredictable name — instead of a predictable `$$`-based path under a
world-writable `/tmp`. A local attacker can then neither guess the path nor
pre-create a file whose contents would be read back into `$PROMPT`, which would
be a terminal-escape and prompt-injection vector. If `mktemp` is unavailable the
memoization disables itself and every prompt does a full IPC round trip
(`zsh/core/init.zsh`). Script segments add no files: their cache stays in
shell variables, with no `/tmp` path and no PID-keyed state (see
[Script Segment Trust Model](#script-segment-trust-model)).

**Downloaded release artifacts are trusted only after verification.** What the
installers enforce, and how to check a download by hand, is in
[Release Integrity and Supply Chain](#release-integrity-and-supply-chain).

## Script Segment Trust Model

A plugin with
`entry_type = "script"` ships one POSIX `sh` script per segment. Bash, Zsh and
Fish execute it (they never source it) as you, once per command that passes
the segment's triggers, in every directory you enter, including untrusted ones
(a cloned repository, an unpacked archive) whose names and files reach the
script when it declares the `pwd` input. Its `require_command`, if any, runs
the same way. Installing a script plugin means trusting it like a shell
startup file, in all three shells. GPY does not protect you from the script;
it protects the shell and the terminal from the script's output and from the
manifest's names. The design record is
[ADR-0011 Addendum: Script Segment Security](docs/dev/adr/adr-0011-addendum-script-segment-security.md).

*Which file runs (containment and ownership).* `gpy theme export` re-resolves
`command` and `require_command` on every export (`ScriptPath::resolve` and
`check_owned_and_not_shared` in `gpy-agent/src/plugin/script.rs`). Each must be
a relative path without `..`, canonicalize (all symlinks resolved) to a regular
file inside the canonical plugin root, and sit at a UTF-8 path without control
characters. The script, and every directory from it up to the plugin root,
must be owned by you or by root and must not be group- or world-writable.
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

*What the script gets.* A clean environment, not the shell's: `PATH` without
empty, `.` or other relative entries, `HOME`, `USER`, `LOGNAME`, `LANG`,
`LC_*`, `TERM`, `TZ` and `TMPDIR` (each only when set), the names the manifest
declares in `env` (when set as a variable in your shell), and the `GPY_*`
inputs and `GPY_OPT_*` settings the runner sets itself. Credentials, tokens,
`SSH_AUTH_SOCK` and exported shell functions are absent unless declared. The
interpreter starts as `sh -c PROLOGUE SCRIPT` from an empty environment; the
prologue reads the values on descriptor 3 and exports them, so declared
values are never arguments of any process and `ps` does not show them to
other users. For a segment that declares `pwd` the runner keeps its inherited
working directory instead of taking it as an argument, so the directory name
is not in argv either. This keeps
credentials out of scripts by default; it is not a confidentiality boundary,
since a script running as you can still read your files. stdin is
`/dev/null`, stderr is discarded, and the working directory is the plugin root
(or `$PWD` when `pwd` is declared).

*What reaches the prompt.* Output is sanitized once, at the runner, before any
shell variable holds it; raw output is never stored. More than 4096 bytes hides
the segment. C0 controls and DEL are stripped without depending on the locale
(also under `LC_ALL=C`); output that still holds a UTF-8-encoded C1 control
(U+0080 to U+009F) hides the segment. The remaining text is escaped for the
drawing shell (`\`, `$` and backticks in Bash; those plus `%` in Zsh; Fish
prints it with `printf '%s'` and needs no escape). Manifest
names (segment tokens, `env` and `require_env` names) are checked against
`^[A-Za-z_][A-Za-z0-9_]*$`-style patterns before any indirection, so a name
such as `a[$(cmd)]` never executes; `require_files`, `require_folders` and
`require_extensions` names with glob characters are matched literally.

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

*No files.* The cache and the once-per-command guard live in shell variables,
keyed by the segment's fixed key plus the current values of its declared
inputs and `env` names, and expire after `ttl_secs`. A non-zero exit, a
failed `require_command` or C1 output is cached as hidden; a timeout,
over-cap output or Ctrl-C is not cached, and a failed trigger drops the
entry. No cache, relay, marker or
output file is written, nothing under `/tmp`, and no state is keyed by a PID.
A repaint (Fish SIGURG, Zsh `reset-prompt`) reuses the cached text; the
script runs at most once per command.

*Known gaps.* A process that leaves the group (`setsid`, a daemonizing double
fork) escapes the kill. In a container whose PID 1 does not reap, timed-out
runs can leave zombies; run containers with `--init`. Text a script writes to
`/dev/tty` bypasses the sanitizer. A lone non-UTF-8 byte `0x80`–`0x9F` is kept
by the shells. Unicode format characters (bidi controls, zero-width marks) are
not controls and pass. A script run in `$PWD` may run repository-local tools
(`git` hooks, `core.fsmonitor`) and they may write to the repository.

## Not Covered by This Design

- **No sandbox.** The agent runs with the full authority of the user who started
  it. There is no seccomp filter, no namespace confinement, and no macOS
  entitlement restriction. Running as your own user is not a sandbox: the agent
  can read anything you can read and write anywhere you can write. Script
  segments have no CPU, memory, file or process limit beyond their timeout.
- **No defence against a compromised account.** Every mitigation above rests on
  file ownership and permissions. Code already running as you defeats all of it.
- **No code signing.** Binaries are not signed or notarized on any platform; see
  [Code signing: not done, deliberately](#code-signing-not-done-deliberately).

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
- **Connection rate limiting** to prevent abuse
- **Message size limits** to prevent DoS
- **Operation timeouts** so a slow or hostile request cannot pin the agent

### File System Access

- **Repositories are read, never written.** GPY reads git state and language
  metadata from your working tree and puts nothing back into it. A script
  segment that declares `pwd` is the exception: it runs in the repository, and
  tools it calls (such as `git`) may write index and lock files there. The file
  watcher's liveness probe does write a sentinel file, but into a private
  directory under the system temp dir, outside every watched repository
  (`gpy-agent/src/watcher/filesystem/mod.rs`).
- **GPY writes to its own paths only**: the instant-prompt and theme-export
  caches under `~/.cache/gpy/`, its runtime directory, and `config.toml` plus
  theme and palette files under `~/.config/gpy/` (touched by `gpy theme`,
  `gpy palette`, `gpy plugin` and config edits). A debug log is appended to only
  when debug logging is switched on.
- **Path sanitization** — paths are validated before access.
- **No elevated privileges** — the agent runs as the invoking user and never
  asks for more.

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

For the per-advisory assessment and audit history, see [Security Audit Notes](docs/dev/security-audit.md).

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

## Release Integrity and Supply Chain

### What every release publishes

| Artifact | Purpose |
|----------|---------|
| `install-oneline.sh` | The one-line installer, with the release tag built in. Published as an asset of each release, never fetched from a branch |
| `gpy-shell-files.tar.gz` | The Fish, Zsh and Bash scripts the installer puts in your config directory |
| `uninstall.sh` | The uninstaller (every shell) |
| `<asset>.sha256` | SHA-256 digest beside each binary, archive and script |
| `SHA256SUMS` | Aggregate manifest covering every downloadable file |
| `gpy-sbom.cdx.json` | CycloneDX SBOM of the Rust dependency graph the binaries were built from |
| Build attestation | Sigstore-backed provenance tying each binary, archive, script and the SBOM to its workflow run and commit (the `.sha256` files and `SHA256SUMS` are not attested) |

Checksums and the SBOM are generated by `scripts/package-release.sh` and
`scripts/generate-sbom.sh`, both exercised by the local quality gate rather
than only on a tag push. The release fails if any published file lacks a
matching digest.

### What the installers enforce

Both `install-oneline.sh` and `install.sh` verify every binary before it is
installed, and fail closed. `install-oneline.sh` also verifies the shell
scripts: it downloads `gpy-shell-files.tar.gz`, checks it against its
published digest, and only then unpacks it and installs from it, so every file
it installs is covered by a checked digest. (`install.sh` installs from an
archive you already extracted; it checks the binaries against the digests in
that archive, and verifying the archive itself is your step — see below.)

- A missing checksum aborts the install. A release that publishes no
  verification metadata is indistinguishable from one whose metadata was
  stripped in transit, so both are refused.
- A mismatched checksum aborts the install, leaving any existing installation
  untouched.
- A machine with neither `sha256sum` nor `shasum` aborts before downloading
  anything.
- `install-oneline.sh` installs only from a release tag: the one stamped into
  the asset you downloaded, or the one named by `GPY_VERSION`. It has no
  fallback to a mutable branch, and it never executes a script fetched from a
  branch.
- The `gpy` CLI is required, like the agent: it installs the shell
  integration, so a missing, unverifiable or non-functional CLI aborts the
  install.

### Trust model

What the checksum does and does not prove:

- The digests are published in the **same release** as the files they describe.
  They detect corruption, truncation, and a mismatched mix of files. They do
  **not** detect a compromised release: anyone able to alter an asset in the
  release (a stolen maintainer credential, a compromised workflow or GitHub
  itself) can alter its digest too.
- Fetching the installer from `releases/.../install-oneline.sh` rather than a
  branch means a given tag always serves the same script, and the script is
  checksummed and attested like every other asset. The first download of the
  installer is still trusted on the strength of HTTPS and GitHub alone.
- The build-provenance attestation is the independent check. It is signed
  through Sigstore, so it is evidence the file was produced by this
  repository's release workflow at a specific commit, and it is stored outside
  the release's asset list. It is **optional**, requires the `gh` CLI, and is
  **not** run by the installer: a compromised installer could skip it, so you
  run it on the installer before running that. It shows where a file came from,
  not that the code is free of bugs or malicious changes.
- Nothing is signed with a long-lived project key (see below).

### Verifying a download yourself

```sh
curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/SHA256SUMS
curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/gpy-release.tar.gz
sha256sum --ignore-missing -c SHA256SUMS

# Provenance: which workflow run and commit produced this file
gh attestation verify gpy-release.tar.gz --repo jpease/gpy
```

To check the one-line installer before running it, download it instead of
piping it, verify it, then run it:

```sh
curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/install-oneline.sh
gh attestation verify install-oneline.sh --repo jpease/gpy
sh install-oneline.sh
```

A checksum proves your download matches what the release page says. An
attestation proves the release page's files came from this repository's CI.

### Code signing: not done, deliberately

GPY binaries are **not** code-signed or notarized on any platform. Apple
Developer ID signing and Windows Authenticode both require paid certificates
tied to a legal identity, which this project does not hold.

The practical consequences:

- **macOS**: Gatekeeper would kill an unsigned binary extracted from a
  downloaded archive, so the installers remove the `com.apple.quarantine`
  attribute — but only after the binary's checksum has been verified. Nothing
  unverified ever has a barrier lowered for it.
- **Windows**: SmartScreen may warn on first run.
- **All platforms**: checksums plus build attestation are the integrity
  guarantee. They establish that a binary came from this repository's CI
  unaltered; they do not establish a signed real-world identity behind it.

This is a resourcing decision, and it will be revisited if the project takes on
funding or an organizational owner.

### SBOM scope

`gpy-sbom.cdx.json` covers the transitive closure of the agent's normal and
build dependencies as resolved by the committed `Cargo.lock`.
Dev-dependencies (test harnesses, benchmark crates) are excluded because they
are not linked into a shipped binary. The document does not cover the Fish,
Bash, and Zsh integration scripts, which have no third-party dependencies; they
ship checksummed in `gpy-shell-files.tar.gz`.
## Best Practices

1. **Keep GPY updated** — install security updates promptly.
2. **Install from the release assets**, and let the installer do the checksum
   verification. If you install by hand, verify the download yourself first.
3. **Guard `gpy-agent` the way you guard your shell startup files.** Whatever
   can overwrite the binary on your `PATH`, or the theme-export cache under
   `~/.cache/gpy/`, can run code in every shell you open.

## Security Audits

- No third-party security audit has been performed.
- Dependency auditing is automated: `cargo audit` and `cargo deny` run in the
  quality gate on every change.
- Accepted advisories and their per-crate assessments are recorded in
  [Security Audit Notes](docs/dev/security-audit.md).
