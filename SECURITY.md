# Security Policy

## Supported Versions

GPY is a personal project with one release line. Security fixes land on the latest release; there is no long-term-support branch and no backporting to older versions.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

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
and a report may sit unread for some time. That is a capacity limit, not a
judgment about the report.

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
them. Who trusts what across those seams is the whole security model.

**Your shell trusts the agent completely.** Every new shell runs the agent's
theme export as shell code. Bash and Zsh do it with `eval "$(gpy-agent theme
export ...)"` (`bash/core/init.bash`, `zsh/core/init.zsh`). Fish pipes the same
output into `source`, or sources the theme-export cache file the agent wrote
under `~/.cache/gpy/` (`fish/core/init.fish`). Prompt content returned over the
socket reaches the prompt the same way. This is what a prompt generator is —
something has to emit shell code — but it puts the agent binary on the same
footing as your shell startup files.

The consequence, stated plainly: anyone who can replace `gpy-agent` on your
`PATH`, or write to the theme-export cache, gets arbitrary code execution in
every shell you open from then on. Three things stand in the way, and only
these three:

- The installers verify every downloaded binary against its published SHA-256
  and abort on a mismatch or a missing checksum. See
  [Release Integrity and Supply Chain](#release-integrity-and-supply-chain).
- Both the cache directory and the install directory are ordinary user-owned
  paths. Anything already running as you can write to them.
- Nothing else. There is no signature check at load time and no integrity check
  on the cache file.

**Prompt data is escaped for the shell that draws it.** Bash and Zsh paste the
agent's prompt output into `PS1`/`PROMPT`, and both shells expand prompt
strings on every draw. Directory names, git branch names, language and
virtualenv names, hostnames, usernames and theme literals are untrusted text —
a cloned repository or an unpacked archive chooses them — so the agent escapes
every character the shell would expand in them (`\`, `$` and backticks for
Bash; those plus `%` for Zsh) through the `bash-prompt` and `zsh-prompt` output
formats (`gpy-agent/src/formatter/style_encoder.rs`, #677). The instant-prompt
cache keeps a separate file per format. The integrations pin the prompt options
that escaping relies on: `shopt -s promptvars` in Bash, and `prompt_subst`,
`prompt_percent` and `no_prompt_bang` in Zsh. The clock's live-time token
(`\D{…}`/`%D{…}`) is the only agent text left for the shell to expand. Fish
prints its prompt literally and receives the unescaped `ansi` format.

**The agent trusts whoever can open its socket.** The socket is chmod'ed to
`0600` immediately after bind, and the agent refuses to bind at all if its
runtime directory is owned by another user or is group- or world-writable
(`gpy-agent/src/ipc/server/handle.rs`). That owner-only file permission is the
access control. There is no handshake and no peer-credential check: any process
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
(`zsh/core/init.zsh`).

**Downloaded release artifacts are trusted only after verification.** What the
installers enforce, and how to check a download by hand, is in
[Release Integrity and Supply Chain](#release-integrity-and-supply-chain).

## Not Covered by This Design

- **No sandbox.** The agent runs with the full authority of the user who started
  it. There is no seccomp filter, no namespace confinement, and no macOS
  entitlement restriction. Running as your own user is not a sandbox: the agent
  can read anything you can read and write anywhere you can write.
- **No defence against a compromised account.** Every mitigation above rests on
  file ownership and permissions. Code already running as you defeats all of it.
- **No code signing.** Binaries are not signed or notarized on any platform; see
  [Code signing: not done, deliberately](#code-signing-not-done-deliberately).

## Hardening

### IPC

- **Unix domain sockets** — local only, no network listener
- **Socket mode `0600`** in a runtime directory the agent refuses to use when it
  is owned by another user or is group- or world-writable
- **Path validation** to prevent directory traversal
- **Message content validated against the protocol schema**
  (`gpy-agent/src/ipc/protocol.rs`, `gpy-agent/schemas/`)
- **Connection rate limiting** to prevent abuse
- **Message size limits** to prevent DoS
- **Operation timeouts** so a slow or hostile request cannot pin the agent

### File System Access

- **Repositories are read, never written.** GPY reads git state and language
  metadata from your working tree and puts nothing back into it. The file
  watcher's liveness probe does write a sentinel file, but into a private
  directory under the system temp dir, outside every watched repository
  (`gpy-agent/src/watcher/filesystem.rs`).
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
  on the repository (#507) and is the authoritative control. Contributors who
  have `gitleaks` installed also get it run on staged changes by the
  repository's pre-commit hook (`.raven/git-hooks/pre-commit` and
  `.pre-commit-config.yaml`). A separate Gitleaks CI job is deliberately not
  added: it would duplicate push protection, add one more pinned action or
  binary download to maintain, and only report after a secret is already in a
  pushed branch.
- **Untrusted input:** workflow `run:` steps never interpolate `${{ ... }}`
  expressions from event data; values are passed through `env:`.

## Release Integrity and Supply Chain

### What every release publishes

| Artifact | Purpose |
|----------|---------|
| `<asset>.sha256` | SHA-256 digest beside each binary and archive |
| `SHA256SUMS` | Aggregate manifest covering every downloadable file |
| `gpy-sbom.cdx.json` | CycloneDX SBOM of the Rust dependency graph the binaries were built from |
| Build attestation | Sigstore-backed provenance tying each artifact to its workflow run and commit |

Checksums and the SBOM are generated by `scripts/package-release.sh` and
`scripts/generate-sbom.sh`, both exercised by the local quality gate rather
than only on a tag push. The release fails if any published file lacks a
matching digest.

### What the installers enforce

Both `install-oneline.sh` and `install.sh` verify every binary before it is
installed, and fail closed:

- A missing checksum aborts the install. A release that publishes no
  verification metadata is indistinguishable from one whose metadata was
  stripped in transit, so both are refused.
- A mismatched checksum aborts the install, leaving any existing installation
  untouched.
- A machine with neither `sha256sum` nor `shasum` aborts before downloading
  anything.
- `install-oneline.sh` installs only from a resolved release tag. It has no
  fallback to a mutable branch.

The one exception is scope, not strictness: if a release is missing the `gpy`
CLI asset entirely, the installer warns and continues with the prompt alone.
The CLI only powers completions and subcommands. A `gpy` binary that downloads
but fails verification still aborts.

### Verifying a download yourself

```sh
curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/SHA256SUMS
curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/gpy-release.tar.gz
sha256sum --ignore-missing -c SHA256SUMS

# Provenance: which workflow run and commit produced this file
gh attestation verify gpy-release.tar.gz --repo jpease/gpy
```

A checksum proves your download matches what the release page says. An
attestation proves the release page itself came from this repository's CI.

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
Bash, and Zsh integration scripts, which have no third-party dependencies.

## Best Practices for Users

1. **Keep GPY updated** — install security updates promptly.
2. **Install from the release assets**, and let the installer do the checksum
   verification. If you install by hand, verify the download yourself first.
3. **Guard `gpy-agent` the way you guard your shell startup files.** Whatever
   can overwrite the binary on your `PATH`, or the theme-export cache under
   `~/.cache/gpy/`, can run code in every shell you open.
4. **Report unexpected behavior** through the private advisory form above.

## Security Audit History

- No third-party security audit has been performed.
- Dependency auditing is automated: `cargo audit` and `cargo deny` run in the
  quality gate on every change.
- Accepted advisories and their per-crate assessments are recorded in
  [Security Audit Notes](docs/dev/security-audit.md).

## Published Advisories

Advisories are published at
<https://github.com/jpease/gpy/security/advisories>.

*(None to date.)*

## Contact

Security reports go through
[private vulnerability reporting](https://github.com/jpease/gpy/security/advisories/new),
never through a public issue. Everything else: open a GitHub issue.
