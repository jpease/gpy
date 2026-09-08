# Formatter Architecture

The formatter layer isolates every "render this response for shell X" concern from the rest of the
agent. Core logic (git status collection, language detection, IPC plumbing) never touches ANSI escape
codes or Fish-specific quoting anymore; it simply builds an `ipc::Response` and hands it to a formatter.

## Module Layout

- `formatter/mod.rs` defines the shared `Format` enum, `Formatter` trait, factory helper (`create_formatter`), and `RenderContext`
- `formatter/json.rs` handles the JSON wire format used by tests, CLI tooling, and IPC default paths
- `formatter/fish.rs` produces space-separated `--flag value` arguments for the fish function wrappers; re-exported as both `FishFormatter` and `ZshFormatter` since Zsh's argument-based rendering is byte-identical
- `formatter/fish_source.rs` emits `set -g` assignments so Fish can populate its session variables
- `formatter/fish_ansi.rs` renders the fully-coloured prompt segments through the Starship-compatible template engine (`src/template/`); re-exported as `AnsiFormatter`, the shell-agnostic ANSI format
- `formatter/style_encoder.rs` encodes the template engine's format-agnostic `Span`s into shell-specific escape sequences (one shared implementation instead of per-shell copies, #586)
- `formatter/separator.rs` is the single powerline-separator implementation shared by every segment resolver (#586)
- `formatter/*_resolver.rs` (`character`, `directory`, `duration`, `git`, `hostname`, `language`, `username`) each map one segment's data to Starship-exact template variable names, so Starship `format` strings can be dropped in verbatim (#186)

`Format::BashSource` and `Format::ZshSource` are declared but not yet implemented; `create_formatter`
returns a config error naming `ansi`/`json` as the working alternative rather than silently
substituting one.

All formatters receive the same `RenderContext`, which carries the active `Config`, `ThemeConfig`,
the previous segment's resolved foreground/background colors, the active color `Palette`, and a
`SegmentPosition` (first/middle/last) used to pick delimiters and continue color chaining.

## Adding Another Format

1. Add a `Format` enum variant and update `create_formatter` to return your implementation (or a
   `not yet implemented` config error, as `BashSource`/`ZshSource` do today)
2. Create `formatter/<your_format>.rs` implementing the `Formatter` trait
3. Leverage `RenderContext` instead of loading config/theme from disk inside the formatter
4. Extend `tests/formatter_integration_tests.rs` to cover the new format with parity expectations
5. Update CLI and IPC code if the new format should be externally selectable

## Testing Expectations

- Unit tests live next to each formatter for basic behaviour (escaping, empty responses, delimiter checks)
- `tests/formatter_integration_tests.rs` exercises every `Format` variant through the `Formatter` trait, including the `not yet implemented` error path for `BashSource`/`ZshSource`
- `tests/formatter_snapshot_tests.rs` uses `insta` to catch unintended output drift (`cargo insta review` / `cargo insta accept` to update)
- IPC client/protocol tests ensure the `Format` enum serialises/deserialises as expected
- Run `cargo nextest run --test formatter_integration_tests --test formatter_snapshot_tests` plus the IPC suites whenever a formatter changes

## Interaction Points

- CLI (`main.rs`) and IPC (`ipc/mod.rs`) both use the shared `Format` enum, so validation, defaults, and help text stay in sync
- Agent oneshot handlers build `Response` values and immediately delegate to `create_formatter`
- IPC server obtains current config/theme from the managers, builds a `RenderContext`, and renders through the trait
- Future shells can be added by shipping a formatter crate file without touching agent internals or watchers
