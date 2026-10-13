# GPY Agent

[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPLv3+-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

A fast, cross-platform agent for Git status detection and programming language identification, powering the [GPY prompt](https://github.com/jpease/gpy) for Bash, Fish, and Zsh.

## This crate is an application, not a library

`gpy-agent` is **not published to crates.io** and sets `publish = false` (#502).
It ships as two binaries — the `gpy-agent` daemon and the `gpy` CLI — through
[GitHub Releases](https://github.com/jpease/gpy/releases) and the one-line
installer. A registry install could only ever deliver those binaries, never the
`fish/`, `bash/` and `zsh/` trees the prompt renders from, so it would leave you
with a broken install rather than a partial one.

The crate root exports 21 modules because 52 of the 61 integration test targets in
`gpy-agent/tests/` consume it externally as `gpy_agent::`. That surface is an
implementation detail. **It carries no semver guarantee**: any module, type,
function or signature may change or disappear in any release, including a patch
release. The stable contracts are the IPC protocol (`schemas/message.json`,
`schemas/response.json`, versioned per `SCHEMA_EVOLUTION.md`) and the CLI.

Depend on it at your own risk, and pin a git revision if you do.

## Features

- 🚀 **High Performance**: Sub-millisecond IPC latency with real-time file watching
- 🔄 **Cross-Platform**: Native support for Linux and macOS; Windows via WSL (see [Supported Platforms and Shells](../docs/INSTALL.md#supported-platforms-and-shells))
- 📊 **Git Integration**: Comprehensive Git status detection (staged, unstaged, untracked, conflicts)
- 🏷️ **Language Detection**: Automatic programming language identification for development environments
- 💾 **Smart Caching**: Memory-efficient LRU caching with configurable TTL
- 📡 **Robust IPC**: Unix domain sockets for fast, secure communication
- 📈 **Observability**: Structured logging, metrics collection, and health monitoring
- 🛡️ **Fault Tolerant**: Circuit breaker pattern and graceful error handling

## Installation

Install GPY as a whole — binaries plus shell files — with the one-line installer:

```bash
curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | sh
```

See [docs/INSTALL.md](../docs/INSTALL.md) for release-archive, manual, and
build-from-source instructions.

## Usage

The runnable programs in `examples/` show how the in-tree code is wired
together. They are not an API to build against; see the unsupported-library
note above.

- `cargo run --example basic_usage`: in-process `Agent::handle_oneshot` with
  `{"type":"git_status"|"language_detect","path":...,"format":"json"}`.
- `cargo run --example ipc_client`: newline-delimited JSON to a running daemon
  on the socket from `agent::lifecycle::get_socket_path()` (`{"op":"ping"}`,
  `{"op":"git","cwd":...,"format":"json"}`, `{"op":"lang",...}`).

## Architecture

The GPY agent consists of several key components:

- **IPC Layer**: Handles communication between the agent and Fish shell
- **Git Status**: Monitors Git repositories for changes
- **Language Detection**: Identifies programming languages in projects
- **File Watcher**: Real-time filesystem monitoring with debouncing
- **Cache System**: LRU cache for performance optimization
- **Configuration**: TOML-based configuration management
- **Observability**: Structured logging and metrics collection

## Configuration

Create a `config.toml` file:

```toml
[agent]
# IPC configuration
socket_path = "/tmp/gpy-agent.sock"
tcp_fallback_port = 0

[cache]
# Cache settings
ttl_seconds = 300
max_entries = 1000

[git]
# Git status settings
max_depth = 3
follow_symlinks = false

[logging]
# Logging configuration
level = "info"
format = "json"
```

## Performance

Budgets and recorded baselines live in `../tests/performance-baselines.json`
and are enforced by `just bench-ci`. For example: IPC round trip 1 ms target
(0.037 ms baseline), cached prompt render 20 ms target, agent memory 15 MB
target (50 MB max). See `../docs/dev/performance/benchmarking.md`.

## Platform Support

See [Supported Platforms and Shells](../docs/INSTALL.md#supported-platforms-and-shells)
for the canonical OS/shell support matrix and per-shell IPC tool requirements.

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux | x86_64, aarch64 | ✅ Supported |
| macOS | x86_64, aarch64 | ✅ Supported |
| Windows, via WSL | x86_64, aarch64 | ✅ Supported (recommended path) |
| Windows, native | x86_64, aarch64 | CLI-only, unverified — see the canonical matrix |

**Note**: the shell prompt integration depends on Unix domain sockets, which
native Windows does not provide; WSL supplies them. All IPC uses Unix domain
sockets.

## Development

### Building

```bash
cargo build --release
```

### Testing

```bash
# Run all tests (high parallelism, ~5 seconds)
cargo nextest run --workspace

# Run doc tests (cargo-nextest does not support doc tests)
cargo test --doc

# Run with single thread (slower but more reliable on resource-constrained systems)
cargo nextest run --workspace --test-threads 1

# Run with coverage
cargo tarpaulin --workspace --all-features

# Run strict clippy checks (from the repo root)
just lint   # moon run gpy-agent:clippy
```

**Note**: All 179 tests pass with default parallelism in ~40 seconds. The `serial_test` crate is available for tests that explicitly need sequential execution (useful in CI/CD), but not required for correctness.

### Benchmarks

```bash
cargo bench
```

## Contributing

Contributions are welcome! Please see the main [GPY repository](https://github.com/jpease/gpy) for contribution guidelines.

## License

This project is licensed under the **GNU General Public License v3.0 or later**.

See the [LICENSE](../LICENSE) file in the repository root for full details.

## Related Projects

- [GPY](https://github.com/jpease/gpy) - The Fish shell prompt that uses this agent
- [Fisher](https://github.com/jorgebucaran/fisher) - Plugin manager for Fish shell
