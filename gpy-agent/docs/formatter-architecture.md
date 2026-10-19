# Formatter Architecture

The formatter layer isolates every "render this response for shell X" concern from the rest of the
agent. Core logic (git status collection, language detection, IPC plumbing) never touches ANSI escape
codes or Fish-specific quoting; it builds an `ipc::Response` and hands it to a formatter.

## Module Layout

- `formatter/mod.rs` defines the shared `Format` enum, `Formatter` trait, factory helper (`create_formatter`), and `RenderContext`
- `formatter/json.rs` handles the JSON wire format used by tests, CLI tooling, and IPC default paths
- `formatter/fish_ansi/mod.rs` renders the fully-coloured prompt segments through the Starship-compatible template engine (`src/template/`); re-exported as `AnsiFormatter`. It serves three formats that share one color encoding and differ only in how segment text is escaped (`PromptDialect`): `ansi` (Fish, text verbatim), `bash-prompt` (escaped for `PS1` with `promptvars`) and `zsh-prompt` (escaped for `PROMPT` with `prompt_subst`/`prompt_percent`), so data such as directory and branch names is never expanded as prompt code
- `formatter/style_encoder.rs` encodes the template engine's format-agnostic `Span`s into shell-specific escape sequences (one shared implementation for every shell). A span is either `Text` (escaped by the prompt dialects) or a `PromptToken` (the clock's `\D{…}`/`%D{…}`, emitted raw so the shell keeps it ticking)
- `formatter/separator.rs` is the single powerline-separator implementation shared by every segment resolver
- `formatter/*_resolver.rs` (`character`, `clock`, `directory`, `duration`, `git`, `hostname`, `language`, `username`) each map one segment's data to Starship-exact template variable names, so Starship `format` strings can be dropped in verbatim

All formatters receive the same `RenderContext`, which carries the active `Config`, `ThemeConfig`,
the previous segment's resolved foreground/background colors, the active color `Palette`, and a
`SegmentPosition` (first/middle/last) used to pick delimiters and continue color chaining.

## Adding Another Format

1. Add a `Format` enum variant and return your implementation from `create_formatter`
2. Create `formatter/<your_format>.rs` implementing the `Formatter` trait
3. Leverage `RenderContext` instead of loading config/theme from disk inside the formatter
4. Extend `tests/formatter_integration_tests.rs` to cover the new format with parity expectations
5. Update CLI and IPC code if the new format should be externally selectable

## Testing Expectations

- Unit tests live next to each formatter for basic behaviour (escaping, empty responses, delimiter checks)
- `tests/formatter_integration_tests.rs` exercises each `Format` through the `Formatter` trait
- `tests/formatter_snapshot_tests.rs` uses `insta` to catch unintended output drift (`cargo insta review` / `cargo insta accept` to update)
- IPC client/protocol tests ensure the `Format` enum serialises/deserialises as expected
- Run `cargo nextest run --test formatter_integration_tests --test formatter_snapshot_tests` plus the IPC suites whenever a formatter changes

## Interaction Points

- CLI (`main.rs`) and IPC (`ipc/mod.rs`) both use the shared `Format` enum, so validation, defaults, and help text stay in sync
- Agent oneshot handlers build `Response` values and immediately delegate to `create_formatter`
- IPC server obtains current config/theme from the managers, builds a `RenderContext`, and renders through the trait
- A new shell needs a formatter (plus its `Format` variant) and shell-side integration; the agent's data collection and watchers are unaffected
