//! # GPY Agent - Fast background process for Git status and language detection
//!
//! This library provides the core functionality for the GPY prompt system (Bash, Fish, and Zsh).
//! It handles Git repository analysis, programming language detection, and file watching
//! through a background agent process that communicates via IPC.
//!
//! ## This crate is an application, not a library
//!
//! `gpy-agent` is **not published to crates.io** (`publish = false`, #502). It
//! is distributed as two binaries — the `gpy-agent` daemon and the `gpy` CLI —
//! via tagged release archives and the one-line installer. A registry install
//! could deliver only those binaries, never the `fish/`, `bash/` and `zsh/`
//! trees the prompt renders from, so it would leave a broken install rather
//! than a partial one.
//!
//! The modules below are public because the crate's integration tests consume
//! them externally as `gpy_agent::`. **That surface carries no semver
//! guarantee**: any item here may change or disappear in any release, patch
//! releases included. The versioned contracts are the IPC protocol
//! (`schemas/message.json`, `schemas/response.json`, see `SCHEMA_EVOLUTION.md`)
//! and the CLI.
//!
//! ## Key Modules
//!
//! - [`agent`]: The main agent process logic and lifecycle management.
//! - [`cache`]: Cache policy abstractions and timing parameters.
//! - [`config`]: Configuration loading and schema definition.
//! - [`theme`]: Theme management with hot-reload support.
//! - [`git`]: Git repository status detection.
//! - [`language`]: Programming language detection and version parsing.
//! - [`ipc`]: Inter-process communication (IPC) server and protocol.
//! - [`watcher`]: Filesystem watching for real-time updates.
//! - [`security`]: Security features like path validation and rate limiting.
//! - [`error`]: Centralized error and result types.
//!
//! ## Common Operations for AI Agents
//!
//! This section documents frequent modification patterns to help AI agents understand
//! system entry points and data flows without requiring deep codebase exploration.
//!
//! ### How to Add a New IPC Operation
//!
//! The GPY agent uses a request-response pattern over Unix domain sockets. All operations
//! follow a consistent pipeline: Fish shell → IPC protocol → Agent handler → Formatter → Response.
//!
//! **Sense-Plan-Act-Verify Checklist:**
//!
//! 1. **Sense** (Understand requirements):
//!    - What data does the Fish shell need?
//!    - What input parameters are required (path, options, format)?
//!    - What format should the response take (JSON, Fish ANSI, Fish source)?
//!    - Read existing operations in [`ipc::Message`] and [`ipc::Response`]
//!
//! 2. **Plan** (Design the operation):
//!    - Choose a descriptive operation name (e.g., `WorkspaceUpdate`, `ThemeQuery`)
//!    - Define request fields (path validation, size limits per [`ipc::protocol`])
//!    - Define response structure (structured data vs. formatted output)
//!    - Determine if caching is needed (see [`cache`] module)
//!
//! 3. **Act** (Implement the operation):
//!    - **Step 1**: Add variant to [`ipc::Message`] enum in `ipc/mod.rs`
//!      ```rust,no_run
//!      # use serde::{Deserialize, Serialize};
//!      # use gpy_agent::ipc::Format;
//!      #[derive(Debug, Deserialize, Serialize)]
//!      enum Message {
//!          // ... existing variants
//!          NewOperation {
//!              path: String,
//!              #[serde(default)]
//!              format: Format,
//!          },
//!      }
//!      ```
//!    - **Step 2**: Add variant to [`ipc::Response`] enum in `ipc/mod.rs`
//!      ```rust
//!      enum Response {
//!          // ... existing variants
//!          NewOperationResult {
//!              data: String,
//!          },
//!      }
//!      ```
//!    - **Step 3**: Add Fish shell format converter in `ipc/protocol.rs` `FishMessage::into_message()`
//!      ```rust,no_run
//!      # use gpy_agent::ipc::{Message, Format};
//!      # use gpy_agent::security::SafePath;
//!      # fn example(op: &str, cwd: Option<String>, format: Format) -> Result<Message, Box<dyn std::error::Error>> {
//!      # match op {
//!      "new_op" => Ok(Message::RepositoryStatus {
//!          path: SafePath::new(&cwd.unwrap_or_else(|| ".".to_owned()))?,
//!          format,
//!          is_last: false,
//!          is_first: false,
//!          prev_bg: None,
//!      }),
//!      # _ => unimplemented!(),
//!      # }
//!      # }
//!      ```
//!    - **Step 4**: Add handler in `agent/mod.rs` `Agent::handle_message()` (see [`agent`] coordination)
//!      ```rust,no_run
//!      # use gpy_agent::ipc::{Message, Response};
//!      # use gpy_agent::git::RepositoryState;
//!      # struct Agent;
//!      # impl Agent {
//!      # fn new_operation_handler(&self, path: &str) -> Result<(String, u32, u32, u32, u32, u32, u32, RepositoryState), Box<dyn std::error::Error>> {
//!      #     Ok(("main".to_string(), 0, 0, 0, 0, 0, 0, RepositoryState::Clean))
//!      # }
//!      # fn handle_message(&self, msg: Message) -> Result<Response, Box<dyn std::error::Error>> {
//!      # match msg {
//!      Message::RepositoryStatus { path, format, is_last, is_first: _, prev_bg: _ } => {
//!          let (branch, ahead, behind, staged, unstaged, untracked, conflicts, state) = self.new_operation_handler(path.as_str())?;
//!          let status = gpy_agent::git::RepositoryStatus {
//!              branch, ahead, behind, ahead_capped: false, behind_capped: false, staged, unstaged, untracked, conflicts, state,
//!              stash_count: 0, detached: false, rebase_progress: None,
//!          };
//!          Ok(Response::RepositoryStatus(status))
//!      }
//!      # _ => unimplemented!(),
//!      # }
//!      # }
//!      # }
//!      ```
//!    - **Step 5**: Add formatter support in `formatter/` modules (JSON, Fish ANSI, Fish source)
//!      - `formatter/fish_ansi.rs` - ANSI escape sequences for terminal display
//!      - `formatter/fish_source.rs` - Fish variables for custom rendering
//!      - `formatter/json.rs` - Structured JSON for parsing
//!      - **Formatter decision guide**:
//!        - JSON: Always add match arm (serde auto-serialization handles the rest)
//!        - Fish source: Add if Fish shell needs to consume data as variables (e.g., `__gpy_my_value`)
//!        - Fish ANSI: Add if operation needs terminal formatting (colored output for display)
//!    - **Step 6**: Add validation to `ipc/protocol.rs` `validate_message_content()`
//!    - **Step 7**: If adding shared state (metrics, trackers, caches):
//!      - Create state module (e.g., `ipc/my_tracker.rs`)
//!      - Add to `EndpointHandle` struct in `ipc/server.rs`
//!      - Update all constructors: `new()`, `with_path()`, `with_path_and_state()`, `new_test_handle()`
//!      - Search for `EndpointHandle::with_path` in `tests/` and update all test files
//!      - See [`agent`] module "How to Add Shared State" section for detailed guide
//!      - **If tracking request metrics**, instrument in `serve_connection()` (ipc/server.rs ~line 379):
//!        ```text
//!        # use std::time::Instant;
//!        let start = Instant::now();
//!        let result = self.process_request_secure(message).await;
//!        let elapsed = start.elapsed();
//!        self.my_tracker.record(elapsed);
//!        ```
//!      - **Why `serve_connection()`?** Measures full request cycle: deserialization, validation,
//!        handler execution, and response formatting. For handler-specific metrics, instrument
//!        within individual message handlers instead.
//!
//! 4. **Verify** (Test the operation):
//!    - Add IPC protocol test in `tests/ipc_protocol_tests.rs`
//!    - Add formatter test in `tests/formatter_tests.rs`
//!    - Add integration test in `tests/integration_tests.rs`
//!    - Test Fish shell integration manually with `__gpy_request new_op $PWD`
//!    - Verify error handling (invalid paths, missing fields, timeout)
//!    - Run `./scripts/quality-check.sh` to ensure no regressions
//!
//! **Related Documentation:**
//! - IPC protocol specification: `docs/dev/architecture.md` section "IPC Protocol Design"
//! - Message handler coordination: [`agent`] module documentation
//! - Formatter architecture: `gpy-agent/docs/formatter-architecture.md`
//!
//! ### How to Add a New Language Detector
//!
//! Language detection uses `gengo-language`'s matcher tables for fast, accurate identification, with trait-based
//! version detection for extensibility. New languages require registering detection logic and
//! theme configuration.
//!
//! **Sense-Plan-Act-Verify Checklist:**
//!
//! 1. **Sense** (Understand requirements):
//!    - Is the language already detected by `gengo-language`? (Check `language/detector.rs`)
//!    - Does the language need version detection? (e.g., Python 3.11, Node.js 18.0)
//!    - What file extensions/patterns identify the language?
//!    - What icon and color should represent the language?
//!
//! 2. **Plan** (Design the detector):
//!    - Determine if this is a new language or alias (e.g., `TypeScript` → `Node.js`)
//!    - Choose version detection method (command line tool, file parsing, API call)
//!    - Plan caching strategy (versions change infrequently, aggressive caching OK)
//!    - Review existing detectors in `language/version.rs` for patterns
//!
//! 3. **Act** (Implement the detector):
//!    - **Step 1**: Add language normalization in `language/detector.rs` `normalize_language_name()`
//!      ```rust,no_run
//!      # fn normalize_language_name(name: &str) -> &str {
//!      # match name {
//!      "NewLanguage" | "NewLang" => "newlang",
//!      # _ => name,
//!      # }
//!      # }
//!      ```
//!    - **Step 2**: Implement the `ReleaseSource` trait in `language/version.rs`
//!      ```rust,no_run
//!      # trait ReleaseSource {
//!      #     fn language_name(&self) -> &'static str;
//!      #     fn version_command(&self) -> &[&str];
//!      # }
//!      pub struct NewLangDetector;
//!      impl ReleaseSource for NewLangDetector {
//!          fn language_name(&self) -> &'static str {
//!              "newlang"
//!          }
//!          fn version_command(&self) -> &[&str] {
//!              &["newlang", "--version"]
//!          }
//!      }
//!      ```
//!    - **Step 3**: Register the detector in `language/version.rs`'s
//!      `get_version_detectors()`
//!      ```rust,no_run
//!      # trait ReleaseSource {
//!      #     fn language_name(&self) -> &'static str;
//!      #     fn version_command(&self) -> &[&str];
//!      # }
//!      # struct NewLangDetector;
//!      # impl ReleaseSource for NewLangDetector {
//!      #     fn language_name(&self) -> &'static str { "newlang" }
//!      #     fn version_command(&self) -> &[&str] { &["newlang", "--version"] }
//!      # }
//!      # let mut detectors: Vec<Box<dyn ReleaseSource>> = Vec::new();
//!      detectors.push(Box::new(NewLangDetector));
//!      ```
//!    - **Step 4**: Add theme configuration via a new `impl Default` block in `theme/model.rs`
//!      ```toml
//!      newlang_icon = ""
//!      newlang_color = "#ABCDEF"
//!      ```
//!    - **Step 5**: Add formatter support in `formatter/fish_ansi.rs` and `formatter/fish_source.rs`
//!
//! 4. **Verify** (Test the detector):
//!    - Create test directory with language files
//!    - Run `gpy-agent oneshot lang --cwd /path/to/test`
//!    - Verify language appears in output with correct version
//!    - Add unit test in `tests/language_tests.rs`
//!    - Test caching behavior (version should be cached per directory)
//!    - Run `./scripts/quality-check.sh`
//!
//! **Related Documentation:**
//! - Language detection architecture: `language/mod.rs` module documentation
//! - Version detector examples: `language/version.rs`
//! - Theme customization: `docs/dev/create-theme.md`
//!
//! ### How to Add a New Prompt Segment
//!
//! Prompt segments are Fish shell functions that integrate with the agent via IPC.
//! Segments follow a two-function pattern: `segment_NAME_detect` (visibility) and
//! `segment_NAME_render` (output).
//!
//! **Sense-Plan-Act-Verify Checklist:**
//!
//! 1. **Sense** (Understand requirements):
//!    - What information should the segment display?
//!    - What conditions determine segment visibility? (e.g., git segment only in repos)
//!    - Should the segment use agent IPC or be pure Fish? (IPC for heavy operations)
//!    - What theme colors/icons are needed?
//!    - Review existing segments in `segments/` directory
//!
//! 2. **Plan** (Design the segment):
//!    - Choose segment name (lowercase, descriptive: `git`, `language`, `duration`)
//!    - Decide data source (agent IPC operation, environment variable, file check)
//!    - Plan delimiter behavior (segments use `is_last` flag for correct spacing)
//!    - Design theme configuration (colors, icons, enable/disable toggle)
//!
//! 3. **Act** (Implement the segment):
//!    - **Step 1**: Create segment file `fish/segments/NAME.fish`
//!      ```fish
//!      function segment_NAME_detect
//!          # Return 0 if segment should be shown, 1 otherwise
//!          # Check config: test "$GPY_NAME_ENABLED" != 0
//!          # Check condition: test -f "marker_file"
//!      end
//!
//!      function segment_NAME_render --argument-names is_last
//!          # Get data from agent (if needed)
//!          set -l data (__gpy_request name "$PWD" "$is_last")
//!
//!          # Or implement pure Fish rendering
//!          gpy_section_start $color blue
//!          gpy_section_append $color blue " $data"
//!          gpy_section_end blue
//!      end
//!      ```
//!    - **Step 2**: Add segment to enabled list in `fish/core/init.fish`
//!      ```fish
//!      set -g __enabled_segments clock duration directory NAME git
//!      ```
//!    - **Step 3**: Add theme configuration via `theme/model.rs` (if needed)
//!      ```toml
//!      [segments.NAME]
//!      enabled = true
//!      icon = ""
//!      color = "#ABCDEF"
//!      ```
//!    - **Step 4**: Update theme export in `theme/manager.rs` to include segment in `__enabled_segments`
//!    - **Step 5**: If using IPC, add new operation following "Add New IPC Operation" checklist above
//!
//! 4. **Verify** (Test the segment):
//!    - Source init script: `source core/init.fish`
//!    - Check segment detection: `segment_NAME_detect; echo $status`
//!    - Check segment rendering: `segment_NAME_render last`
//!    - Test with full prompt: `fish_prompt`
//!    - Verify delimiter handling (last segment should not have trailing delimiter)
//!    - Test config toggle (disable in config, verify segment hidden)
//!    - Add Fish integration test in `tests/fish/` directory
//!    - Run `./scripts/quality-check.sh`
//!
//! **Related Documentation:**
//! - Segment architecture: `docs/dev/segment-development.md` and `docs/dev/add-new-segment.md`
//! - Fish helper functions: `fish/core/renderer.fish` (`gpy_section_start`, `gpy_section_append`, etc.)
//! - IPC request helper: `fish/core/ipc.fish` (`__gpy_request` function)
//! - Existing segment examples: `fish/segments/git.fish`, `fish/segments/clock.fish`
//!
//! ### General Development Guidelines for AI Agents
//!
//! - **Follow existing patterns**: Read similar code before implementing new features
//! - **Respect module boundaries**: Cross-module coordination MUST happen in [`agent`] module
//! - **Validate inputs**: All external data (paths, PIDs, strings) must be validated per [`security`]
//! - **Test exhaustively**: Add unit tests, integration tests, and Fish tests
//! - **Document thoroughly**: Explain "why" not just "what" in code comments
//! - **Run quality checks**: Execute `./scripts/quality-check.sh` before committing
//! - **Check error paths**: Test failure scenarios (missing files, invalid input, timeouts)
//!
//! **Architecture References:**
//! - Overall design: `docs/dev/architecture.md`
//! - Formatter layer: `gpy-agent/docs/formatter-architecture.md`
//! - Test harness: `gpy-agent/tests/README.md`

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Justified clippy exceptions
#![expect(clippy::print_stdout, reason = "CLI tool needs stdout output")]
#![expect(clippy::print_stderr, reason = "error reporting to stderr")]
#![expect(
    clippy::multiple_crate_versions,
    reason = "unavoidable transitive dependency conflicts (see below)"
)]

// ## Duplicate Crate Versions
//
// Canonical register: `docs/dev/security-audit.md`. That document holds the
// full duplicate-version list and the review schedule; `gpy-agent/deny.toml`
// holds the list cargo-deny enforces. This comment exists because the
// `multiple_crate_versions` allow directly above it has to be justified where
// it sits. `tests/bash/dependency_advisory_claims.test.bash` keeps the two in
// agreement with `Cargo.lock`.
//
// Audited 2026-09-01 against `gpy-agent/Cargo.lock` (319 crate dependencies).
// Next review 2027-03-01: not an advisory expiry -- there is no advisory to
// expire -- but a check that the "why" column below still matches the
// lockfile, since dependency upgrades can move a duplicated crate onto a
// different parent without changing whether it duplicates at all.
//
// `cargo audit` reports zero advisories. There used to be five: all five
// arrived through hyperpolyglot 0.1.7, the previous language detector, last
// published 2020-07-26 and abandoned since. #503 waived them for the 0.1.0
// launch; #523 removed hyperpolyglot in favor of `gengo-language`'s tables,
// and #525 removed the now-inapplicable waiver along with it. See "History"
// in `docs/dev/security-audit.md` for the retired advisory IDs.
//
// ### Duplicate crate versions
//
// `gpy-agent/Cargo.lock` has 15 crates with more than one version as of
// 2026-09-01 (`cargo tree --duplicates` gives the full set). This table
// covers only the ones `gpy-agent/deny.toml`'s `[bans] skip` names, not every
// duplicate in the tree: a crate belongs here exactly when it is in `skip`,
// which keeps the table mechanically derivable rather than a judgment call,
// and `tests/bash/dependency_advisory_claims.test.bash` checks both lists
// against `Cargo.lock` directly. The other 12 duplicated crates (bit-set,
// bit-vec, cfg_aliases, hashbrown, itertools, nix, r-efi, rand, rand_core,
// syn, thiserror, thiserror-impl) don't need a skip: `cargo deny check bans`
// passes for them unskipped, mostly because their second copy is used only
// in tests or benches (proptest, criterion, portable-pty) or arrives through
// an optional ratatui backend this crate never enables, and cargo-deny's
// bans graph does not count those edges.
//
// | crate | versions | why |
// |---|---|---|
// | bitflags | 1.3.2, 2.13.1 | 1.3.2 used only for the Unix `test-support` PTY test (portable-pty 0.9.0); 2.13.1 under crossterm, notify, nix and the ratatui workspace |
// | getrandom | 0.3.4, 0.4.3 | both used only in tests: 0.3.4 under rand_core 0.9.5, via proptest; 0.4.3 under tempfile, via insta, proptest and rusty-fork |
// | windows-sys | 0.60.2, 0.61.2 | 0.60.2 under notify; 0.61.2 under rustix, mio, socket2, clap's anstream/anstyle-wincon, and ignore/walkdir's winapi-util |
//
// Re-derive any of this with
// `cargo tree -i <crate>[@version] -e normal,build,dev --target all` against
// `gpy-agent/Cargo.lock` (dev-only chains need `,dev` added to `-e`, which
// `cargo tree`'s default `normal` edge kind omits).

/// The main agent process logic and lifecycle management.
pub mod agent;
/// Cache policy abstractions and timing parameters.
pub mod cache;
/// CLI command implementations.
pub mod commands;
/// Configuration loading and schema definition.
pub mod config;
/// Two-level logging: opt-in `debug_log!` and always-visible `warn_log!`.
pub mod debug;
/// Centralized error and result types.
pub mod error;
/// Nerd Font capability detection for icon defaults.
pub mod font;
/// Output formatter layer (JSON, Fish, ANSI, etc.).
pub mod formatter;
/// Small filesystem predicates shared across handlers.
pub mod fs_util;
/// Git repository status detection.
pub mod git;
/// Importers for third-party prompt configurations (Starship).
pub mod import;
/// Inter-process communication (IPC) server and protocol.
pub mod ipc;
/// Programming language detection and version parsing.
pub mod language;
/// Named color palette loading and conversion.
pub mod palette;
/// Pure path resolution for the runtime, cache, and config roots (#477).
pub mod paths;
/// Extensibility framework
pub mod plugin;
/// Shared subprocess wait-with-timeout primitive (#590).
pub mod process;
/// Simple profiling utilities for performance analysis.
pub mod profiling;
/// Security features like path validation and rate limiting.
pub mod security;
/// Shell identification and syntax templates.
pub mod shell;
/// Starship-compatible prompt template engine (parser + evaluator).
pub mod template;
/// Theme management with hot-reload support.
pub mod theme;
/// Filesystem watching for real-time updates.
pub mod watcher;
/// Windows Subsystem for Linux detection (#849).
pub mod wsl;

pub use error::{Error, Result};
pub use shell::Shell;

/// Agent version for compatibility checking, from `Cargo.toml`.
///
/// This constant is used for version negotiation with clients and for display
/// in the agent's status information.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
