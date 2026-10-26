# gpy-agent

`gpy-agent` is the Rust crate behind the [GPY prompt](../README.md) for Bash,
Fish and Zsh. It builds two binaries:

- `gpy-agent`: the daemon. It watches repositories, caches Git status and
  language detection, and renders prompt segments for the shells over a Unix
  domain socket.
- `gpy`: the CLI for setup, configuration, themes, palettes and diagnostics.

Licensed under GPL-3.0-or-later; see [LICENSE](../LICENSE).

## Not a library

The crate sets `publish = false` and is not on crates.io. A registry install
would deliver the binaries without the `fish/`, `bash/` and `zsh/` trees the
prompt renders from.

The `gpy_agent` library target exists so the integration tests in `tests/` can
use it. It carries no semver guarantee: any module, type or signature may
change in any release. The stable contracts are the CLI, the config and theme
files, and the IPC protocol in [`schemas/`](schemas/README.md), versioned as
described in [SCHEMA_EVOLUTION.md](SCHEMA_EVOLUTION.md).

## Install

See [docs/INSTALL.md](../docs/INSTALL.md) for the installer, release archives,
building from source, and the supported platforms and shells.

## Build and test

Run from `gpy-agent/`:

```bash
cargo build --release                          # target/release/{gpy-agent,gpy}
cargo nextest run --features test-support      # unit and integration tests
cargo test --doc                               # doc tests (nextest skips them)
cargo clippy --all-targets --all-features      # lint policy: [lints] in Cargo.toml
cargo fmt --check
cargo bench                                    # Criterion benchmarks in benches/
```

The `justfile` at the repository root wraps these (`just test-rust`,
`just lint`, `just coverage`, `just bench-ci`). [docs/dev/testing.md](../docs/dev/testing.md)
lists every test tier and its gate; [tests/README.md](tests/README.md)
describes the integration test layout.

The programs in `examples/` run with `cargo run --example <name>`
(`basic_usage`, `ipc_client`).

## Further reading

- [Architecture](../docs/dev/architecture.md)
- [Configuration reference](../docs/user/configuration-reference.md)
- [CLI reference](../docs/user/cli-reference.md)
- [Benchmarking and performance budgets](../docs/dev/performance/benchmarking.md)
- [Contributing](../CONTRIBUTING.md)
