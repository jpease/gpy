# GPY Design Decisions

## Core Principle

GPY is a prompt for Fish, Zsh and Bash on Unix-like systems. Evaluate a design change against three questions:

1. Does it solve a problem users of those shells actually have?
2. Does it improve the prompt meaningfully?
3. Is the complexity justified by the benefit?

## Rejected Approaches

### Cross-platform transports nobody uses

The shells GPY integrates with run on Linux and macOS, and on Windows under WSL, all of which provide Unix domain sockets. GPY uses Unix domain sockets only. There is no Windows transport: `ipc/transport.rs` and the client/server entry points return `native_windows_unsupported()` on non-Unix targets, so native Windows is CLI-only and the prompt integration runs under WSL.

### Unmeasured optimization

IPC over a Unix socket is already well under a millisecond (`ipc_latency` budget: 1 ms target, 1.5 ms max in `tests/performance-baselines.json`). Running git is the real cost. Profile first; add caching or pooling only where a measurement shows the need.

### Enterprise features

GPY runs locally, with one user per agent and no network communication beyond its local socket. Authentication, audit logging and distributed deployment are out of scope.

## Accepted Complexity

### TOML configuration

Users customise timeouts, themes and language preferences. Cost: serde integration, validation and defaults.

### Typed error handling

A custom error enum with `thiserror`, propagated as `Result`, so a failure produces a clear message and the prompt never hangs on it.

### Async agent (Tokio)

Concurrent requests from many shells, and non-blocking git and language work, keep a cached prompt render inside its budget (`prompt_render`: 20 ms target, 50 ms max).

### Native git subprocess

The agent runs `git status --porcelain=v2` and parses the output.

- `gix` (Gitoxide) lacked parity for features such as sparse checkouts and submodules ([ADR-0003](adr/adr-0003-gix-for-git-operations.md)).
- `libgit2` adds a C dependency.
- The `git` binary gets git's own optimisations (`core.untrackedCache`, fsmonitor) for free.

Cost: output parsing and version compatibility.

### Language detection (gengo-language)

`gengo-language` supplies linguist's matcher tables (filenames, path globs, extensions, interpreters, content heuristics) as a data dependency, so mixed-language projects are classified without per-language configuration.

- The directory walk stays GPY's own: the `gengo` wrapper crate's `Gengo::analyze` reads every file in the tree, which does not fit the `language_detection` budget (200 ms max), and its default features pull in `gix`.
- Detection is path-first: a filename, glob or extension that names exactly one language never opens the file, so only ambiguous paths pay for a read.
- The read window (51,200 bytes), extension-before-shebang precedence, and the linguist vendor and documentation exclusion globs (vendored in `gpy-agent/src/language/filters.rs`) match the `hyperpolyglot` behaviour this replaced.

### Plain background process

The agent detaches with `fork()` + `setsid()` instead of registering with systemd or launchd, which would differ per platform. The `daemonize` crate is unmaintained and conflicts with Tokio.

## Decision Framework

Before adding complexity, ask:

1. **User problem**: What specific problem does this solve?
2. **Usage**: How many users will use it?
3. **Platform**: Do the supported shells run on the platforms it targets?
4. **Simpler alternative**: Is there a smaller change that gives most of the benefit?
5. **Maintenance**: Who maintains it when edge cases arise?

## Complexity Budget

**High (justified)**: async agent; git subprocess and porcelain v2 parsing; language detection over a GPY-owned walk.

**Medium (justified)**: TOML configuration; error handling; file watching and debouncing.

**Low (keep simple)**: IPC transport (Unix sockets only); protocol serialisation (JSON); path handling (XDG plus fallback); background process management.

## Adding a Feature

1. Write the user story first: "As a user of shell S, I want X so that Y".
2. Show the problem exists.
3. Start with the simplest solution.
4. Measure the impact on the budgets in `tests/performance-baselines.json`.
5. Record the rationale here or in an [ADR](adr/README.md).
