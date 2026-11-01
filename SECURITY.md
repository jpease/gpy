# Security Policy

## Supported Versions

GPY is a personal project with one release line. Security fixes land on the latest release; there is no long-term-support branch and no backporting to older versions.

| Version | Supported |
| ------- | --------- |
| 0.8.x   | Yes       |

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

GPY is three cooperating pieces: a shell integration for Fish, Bash and Zsh; a
long-running local agent (`gpy-agent`); and a Unix domain socket between them.
The full detail is in [the security model](docs/dev/security-model.md).

- Your shell evaluates the agent's theme export at every start, so whoever can
  replace `gpy-agent` or the export cache runs code in every shell you open.
- Fish `.fish` plugin segments are sourced into your shell, as you, with full
  privileges; Bash and Zsh never source them.
- Script segments run as you in all three shells (see below).
- The agent listens on a Unix socket only, not on the network, and trusts whoever
  can open that socket; the socket and its runtime directory are private to you.
- IPC guards: a maximum message size, a connection rate limit (a connection over
  it is closed without a reply), and PID and path validation. A PID is not
  authentication.
- Downloaded release artifacts are trusted only after verification (see
  [Release Integrity and Supply Chain](#release-integrity-and-supply-chain)).

**Prompt data is escaped for the shell that draws it.** Bash and Zsh paste the
agent's prompt output into `PS1`/`PROMPT`, and both shells expand prompt
strings on every draw. Directory names, git branch names, language and
virtualenv names, hostnames, usernames and theme literals are untrusted text —
a cloned repository or an unpacked archive chooses them — so the agent escapes
every character the shell would expand in them (`\`, `$` and backticks for
Bash; those plus `%` for Zsh) through the `bash-prompt` and `zsh-prompt` output
formats. The instant-prompt
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
character. A lone non-UTF-8 byte `0x80`–`0x9F` is not stripped, and text a script writes to
`/dev/tty` is not sanitized.

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

The script must resolve to a file inside the plugin root and be owned by you
or root. It must not be writable by other users, and the same applies to its
directories and to the plugin root's ancestors (ACLs included on macOS; the
exact ancestor rules are in the
[design record](docs/dev/security-model.md#script-segment-mechanism)). A run
gets the script's `timeout_ms` (at most 500 ms), and its process group is killed
when the deadline passes or the script exits. With Bash 3.2 (stock macOS; Fish
and Zsh use a Bash child, and `/bin/bash` is 3.2 when no Bash 4 is installed),
Ctrl-C during a slow script returns to the prompt only when the script's timeout
expires, and the group is still killed. The plugin-author view is in
[plugins.md](docs/dev/plugins.md#trust).

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

## File System Access

- **Repositories are read, never written.** GPY reads git state and language
  metadata from your working tree and puts nothing back into it. A script
  segment that declares `pwd` is the exception: it runs in the repository, and
  tools it calls (such as `git`) may write index and lock files there. The file
  watcher's liveness probe writes a sentinel file only into a private directory
  under the system temp dir, outside every watched repository.
- **GPY writes to its own paths only**: the instant-prompt and theme-export
  caches under `~/.cache/gpy/`, its runtime directory, and `config.toml` plus
  theme and palette files under `~/.config/gpy/` (touched by `gpy theme`,
  `gpy palette`, `gpy plugin` and config edits). A debug log is appended to only
  when debug logging is switched on.
- **Path sanitization**: paths are validated before access.
- **No elevated privileges**: the agent runs as the invoking user and never
  asks for more.

## Release Integrity and Supply Chain

### What every release publishes

| Artifact | Purpose |
|----------|---------|
| `install-oneline.sh` | The one-line installer, with the release tag built in. Published as an asset of each release, never fetched from a branch |
| `gpy-shell-files.tar.gz` | The Fish, Zsh and Bash scripts the installer puts in your config directory |
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
