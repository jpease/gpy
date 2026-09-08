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

The snippets below mirror the runnable programs in `examples/`
(`cargo run --example basic_usage`). They document how the in-tree code is
wired together; they are not an API you should build against — see the
unsupported-library note above.

### Basic Agent Operation

```rust
use gpy_agent::{ipc, config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize configuration
    let config = config::Config::default();

    // Start IPC server
    let server = ipc::Server::new(config).await?;
    server.run().await?;

    Ok(())
}
```

### Git Status Detection

```rust
use gpy_agent::git_status::GitStatusDetector;

// Detect Git status for current directory
let detector = GitStatusDetector::new();
let status = detector.detect(".")?;
println!("Git status: {:?}", status);
```

### Language Detection

```rust
use gpy_agent::lang_detect::Detector;

// Detect programming language
let detector = Detector::new();
let languages = detector.detect_all(".")?;
for lang in languages {
    println!("Detected: {} ({})", lang.name, lang.confidence);
}
```

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

GPY consistently outperforms Starship across all repository sizes (n=1000):

| Repository Size | GPY Performance | Starship | Advantage |
|----------------|-----------------|----------|-----------|
| Tiny (200 files) | **25.4ms ± 0.9ms** | 27.3ms ± 1.4ms | **6.96% faster** |
| Small (1.4k files) | **25.6ms ± 0.8ms** | 27.6ms ± 1.3ms | **7.25% faster** |
| Medium (28k files) | **33.1ms ± 1.4ms** | 35.0ms ± 1.1ms | **5.43% faster** |
| Large (91k files) | **37.5ms ± 1.3ms** | 39.3ms ± 1.2ms | **4.58% faster** |

**Additional Metrics:**
- **IPC Latency**: <1ms roundtrip
- **Memory Usage**: <15MB RSS
- **CPU Usage**: Minimal when idle

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
