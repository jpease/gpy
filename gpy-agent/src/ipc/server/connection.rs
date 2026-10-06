//! Per-connection request serving.
//!
//! [`ConnectionHandler`] owns exactly the dependencies needed to serve a
//! single client connection: parse and route one line at a time, notify
//! live-update subscribers, and serialize/send the response. (Routing writes
//! the language/git instant-prompt cache inside the handler it dispatches
//! to, not here.) It intentionally does NOT own transport
//! concerns (the listener, the bound-socket inode, the accept-time rate
//! limiter, or the concurrent-connection semaphore) — those belong to the
//! server (`EndpointHandle`) that accepts connections, not to the handler
//! that serves an already-accepted one.
//!
//! Before this split, `EndpointHandle::spawn_client_handler` built a second,
//! throwaway `EndpointHandle` (with a fake socket path, no listener, and a
//! disposable rate limiter) purely to get access to the handful of fields a
//! connection actually needs. Every field later added to `EndpointHandle` had
//! to be hand-duplicated there with a plausible dummy value, and calling a
//! server-lifecycle method (`stop()`, `bind_socket()`, ...) on that fake
//! handle would have misbehaved silently. `ConnectionHandler` has no such
//! fields to fake: it is constructed directly from real, non-optional data.

use crate::formatter::{Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter};
use crate::ipc::{ClientDirectory, Message, Response, handlers::HandlerRegistry, protocol};
use crate::palette::PaletteCache;
use crate::security::{GuardSettings, PidValidator};
use crate::template::{Color, parse_color};
use crate::theme::ThemeManager;
#[cfg(unix)]
use crate::warn_log;
use crate::{Error, Result};
use std::sync::Arc;
use std::time::Duration;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tokio::time::timeout;

/// Result of routing a single message.
///
/// Carries the response to send, the format to render it in, an optional
/// live-update notification path, whether this is the last prompt segment,
/// and the previous segment's background color for powerline chevron
/// rendering.
pub(super) struct RouteResult {
    pub(super) response: Response,
    pub(super) format: Format,
    pub(super) notify_path: Option<String>,
    pub(super) is_last: bool,
    pub(super) is_first: bool,
    pub(super) prev_bg: Option<String>,
}

/// Result of [`ConnectionHandler::parse_and_route_blocking`].
///
/// Carries everything [`RouteResult`] carries, plus whether shutdown was
/// requested and how long routing took (`None` when the message failed to
/// parse, since no route ran).
///
/// The chain position travels as a [`SegmentPosition`] rather than two
/// `bool`s — together with `should_shutdown` that would be three bools on one
/// struct, tripping `clippy::struct_excessive_bools` — and it is built once,
/// at the wire boundary, so nothing downstream converts it back (#586).
struct BlockingLineResult {
    response: Response,
    format: Format,
    notify_path: Option<String>,
    position: SegmentPosition,
    prev_bg: Option<String>,
    should_shutdown: bool,
    route_elapsed: Option<Duration>,
}

/// Serves a single already-accepted client connection.
///
/// Constructed fresh per connection in `EndpointHandle::spawn_client_handler`
/// from `Arc` clones of the server's shared collaborators; see the module
/// docs for why this replaced a throwaway `EndpointHandle` clone.
pub(super) struct ConnectionHandler {
    pub(super) security_config: GuardSettings,
    pub(super) config_manager: Arc<crate::config::manager::ConfigManager>,
    pub(super) theme_manager: Arc<ThemeManager>,
    pub(super) palette_cache: Arc<PaletteCache>,
    pub(super) latency_tracker: Arc<crate::ipc::LatencyTracker>,
    pub(super) shutdown_tx: broadcast::Sender<()>,
    pub(super) handler_registry: Arc<HandlerRegistry>,
}

impl ConnectionHandler {
    fn request_timeout(&self) -> Duration {
        let config = self.config_manager.get();
        Duration::from_secs(config.agent.timeout_seconds.get())
    }

    /// Handle a single client connection using raw JSON protocol (Fish shell compatible)
    ///
    /// # Errors
    ///
    /// Returns an error if reading from or writing to the client fails in a non-recoverable
    /// way, or if serialization of a response cannot be completed.
    #[cfg(unix)]
    pub(super) async fn handle_client(
        &self,
        stream: UnixStream,
        client_registry: Arc<ClientDirectory>,
    ) -> Result<()> {
        use tokio::io::AsyncReadExt;

        let (mut reader, mut writer) = stream.into_split();
        let mut buffer = [0u8; 1024];
        let mut accumulated = Vec::with_capacity(512);

        loop {
            let read_result = timeout(self.request_timeout(), reader.read(&mut buffer)).await;

            match read_result {
                Ok(Ok(0)) => break, // EOF
                Ok(Ok(read_bytes)) => {
                    // Append new data to accumulated buffer
                    if let Some(slice) = buffer.get(..read_bytes) {
                        accumulated.extend_from_slice(slice);
                    } else {
                        break; // Invalid read_bytes value
                    }

                    // Enforce protocol size limit
                    if accumulated.len() > self.security_config.max_message_size {
                        warn_log!(
                            "connection",
                            "Client exceeded max message size ({} > {} bytes)",
                            accumulated.len(),
                            self.security_config.max_message_size
                        );
                        break;
                    }

                    // Process complete lines
                    if self
                        .process_complete_lines(&mut accumulated, &mut writer, &client_registry)
                        .await?
                    {
                        return Ok(()); // Shutdown requested
                    }
                }
                Ok(Err(e)) => {
                    warn_log!("connection", "Failed to read from client: {e}");
                    break;
                }
                Err(_) => {
                    warn_log!(
                        "connection",
                        "Client read timeout after {}ms",
                        self.request_timeout().as_millis()
                    );
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process all complete lines in the accumulated buffer
    ///
    /// Returns `Ok(true)` if shutdown was requested, `Ok(false)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if message processing fails.
    #[cfg(unix)]
    async fn process_complete_lines(
        &self,
        accumulated: &mut Vec<u8>,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        client_registry: &Arc<ClientDirectory>,
    ) -> Result<bool> {
        while let Some(newline_pos) = accumulated.iter().position(|&b| b == b'\n') {
            let Some(line_data) = accumulated.get(..newline_pos) else {
                accumulated.clear();
                break;
            };

            // Skip empty lines
            if line_data.is_empty() {
                accumulated.drain(..=newline_pos);
                continue;
            }

            // Process the message and write response
            let shutdown_requested = self
                .process_message_line(line_data, writer, client_registry)
                .await?;

            // Remove processed line from accumulated buffer
            accumulated.drain(..=newline_pos);

            // Exit if shutdown was requested
            if shutdown_requested {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Parse and route one message line on the blocking pool, returning a
    /// synthesized error result if the blocking task itself failed to join.
    ///
    /// Split out of [`Self::process_message_line`] because it is the whole
    /// blocking half of that function's work: parsing (`SafePath`
    /// canonicalization) and routing (which itself writes the language/git
    /// instant caches inside the handler) both touch the filesystem or can
    /// block on synchronous
    /// single-flight `Condvar`s during cache misses (git status, language
    /// detection). All of it must stay off the Tokio worker threads: a
    /// single stalled `canonicalize()` (e.g. a hung NFS/automount cwd) must
    /// not wedge every other connection (#154, #314).
    #[cfg(unix)]
    async fn route_line_blocking(&self, line_data: &[u8]) -> BlockingLineResult {
        let line_data_owned = line_data.to_vec();
        let handler_registry = Arc::clone(&self.handler_registry);
        let security_config = self.security_config.clone();

        let blocking_result = tokio::task::spawn_blocking(move || {
            Self::parse_and_route_blocking(&line_data_owned, &handler_registry, &security_config)
        })
        .await;

        match blocking_result {
            Ok(result) => {
                if let Some(elapsed) = result.route_elapsed {
                    self.latency_tracker.record(elapsed);
                }
                result
            }
            Err(join_err) => BlockingLineResult {
                response: Response::Error {
                    message: format!("Request processing failed: {join_err}"),
                },
                format: Format::Json,
                notify_path: None,
                position: SegmentPosition::MIDDLE,
                prev_bg: None,
                should_shutdown: false,
                route_elapsed: None,
            },
        }
    }

    /// Process a single complete message line and write response
    ///
    /// Returns `Ok(true)` if shutdown was requested, `Ok(false)` otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if message processing, serialization, or writing fails.
    #[cfg(unix)]
    async fn process_message_line(
        &self,
        line_data: &[u8],
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        client_registry: &Arc<ClientDirectory>,
    ) -> Result<bool> {
        let BlockingLineResult {
            response,
            format,
            notify_path,
            position,
            prev_bg,
            should_shutdown,
            route_elapsed: _,
        } = self.route_line_blocking(line_data).await;

        self.notify_clients(client_registry, &response, notify_path.as_deref());

        // Serialize and send the response
        if let Err(e) = self
            .serialize_and_send_response(&response, format, position, prev_bg.as_deref(), writer)
            .await
        {
            warn_log!("connection", "Failed to send response: {e}");
            return Ok(should_shutdown);
        }

        // If shutdown was requested, trigger the shutdown signal
        if should_shutdown {
            let _ = self.shutdown_tx.send(());
        }

        Ok(should_shutdown)
    }

    /// Blocking: parses the wire message and routes it (routing writes the
    /// language/git instant cache inside the handler). Runs on the blocking
    /// pool via `spawn_blocking` (#314) since parsing does a `canonicalize()`
    /// and routing can wait on synchronous single-flight `Condvar`s during
    /// cache misses (#154).
    fn parse_and_route_blocking(
        line_data: &[u8],
        handler_registry: &HandlerRegistry,
        security_config: &GuardSettings,
    ) -> BlockingLineResult {
        let (requested_format, parsed) = protocol::deserialize_message_with_format(line_data);
        match parsed {
            Ok(message) => {
                let is_shutdown = matches!(message, Message::Shutdown);

                let start = std::time::Instant::now();
                let route_result =
                    Self::route_request_secure(handler_registry, security_config, &message);
                let elapsed = start.elapsed();

                BlockingLineResult {
                    response: route_result.response,
                    format: route_result.format,
                    notify_path: route_result.notify_path,
                    position: SegmentPosition::new(
                        IsLast::from(route_result.is_last),
                        IsFirst::from(route_result.is_first),
                    ),
                    prev_bg: route_result.prev_bg,
                    should_shutdown: is_shutdown,
                    route_elapsed: Some(elapsed),
                }
            }
            // A request that parsed far enough to name its format (e.g. one
            // whose `cwd` fails path validation) is answered in that format,
            // so a prompt request gets an empty segment rather than protocol
            // JSON (#680). Unparseable bytes have no format: answer in JSON.
            Err(e) => BlockingLineResult {
                response: Response::Error {
                    message: format!("Parse error: {e}"),
                },
                format: requested_format.unwrap_or(Format::Json),
                notify_path: None,
                position: SegmentPosition::MIDDLE,
                prev_bg: None,
                should_shutdown: false,
                route_elapsed: None,
            },
        }
    }

    /// Route a request with security validation and handler dispatch.
    ///
    /// This is a static helper (taking the shared registry and guard settings by
    /// reference) so the caller can move cheap `Arc`/`GuardSettings` clones into a
    /// `spawn_blocking` closure. Handlers may perform synchronous single-flight
    /// waits on cache misses, so this MUST run on the blocking pool rather than a
    /// Tokio worker thread (#154).
    ///
    /// Every segment-render message variant carries the same `format`/`is_last`/
    /// `prev_bg` triple; rather than destructuring each variant in its own match
    /// arm (as before #359), that shared shape is pulled once via
    /// [`Message::request_meta`], and the live-update notification path via
    /// [`Message::notify_path`]. `RegisterClient` remains a special case because
    /// (uniquely) it can short-circuit routing entirely on PID validation failure.
    pub(super) fn route_request_secure(
        handler_registry: &HandlerRegistry,
        security_config: &GuardSettings,
        msg: &Message,
    ) -> RouteResult {
        if let Message::RegisterClient { pid, .. } = msg
            && security_config.validate_pids
            && let Err(e) = PidValidator::validate_pid_liveness(pid.get())
        {
            return RouteResult {
                response: Response::Error {
                    message: format!("Invalid PID: {e}"),
                },
                format: Format::Json,
                notify_path: None,
                is_last: false,
                is_first: false,
                prev_bg: None,
            };
        }

        let response = handler_registry
            .route(msg)
            .unwrap_or_else(|e| Response::Error {
                message: e.to_string(),
            });
        let meta = msg.request_meta();
        let notify_path = msg.notify_path();

        RouteResult {
            response,
            format: meta.as_ref().map_or(Format::Json, |m| m.format),
            notify_path,
            is_last: meta.as_ref().is_some_and(|m| m.is_last),
            is_first: meta.as_ref().is_some_and(|m| m.is_first),
            prev_bg: meta.and_then(|m| m.prev_bg),
        }
    }

    /// See `EndpointHandle::notify_clients` for the shared notification logic;
    /// this is the connection-scoped copy used from `process_message_line`.
    fn notify_clients(
        &self,
        client_registry: &Arc<ClientDirectory>,
        response: &Response,
        notify_path: Option<&str>,
    ) {
        super::handle::notify_clients_for_response(
            &self.config_manager,
            client_registry,
            response,
            notify_path,
        );
    }

    /// Serialize and send a response to the client.
    ///
    /// For `AgentStatus` responses, uses native protocol serialization to preserve
    /// the Response enum variant wrapper for backwards compatibility.
    /// For other responses, uses the formatter for the requested format.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "6 params (incl. self): response + format + position + prev_bg + writer. `prev_bg` is threaded alongside the other per-request context that flows from route_request_secure through to the template engine, and it is mandatory for correct powerline chevron rendering, so it cannot be folded elsewhere without adding an indirection struct that would obscure a straightforward data flow"
    )]
    #[cfg(unix)]
    async fn serialize_and_send_response(
        &self,
        response: &Response,
        format: Format,
        position: SegmentPosition,
        prev_bg: Option<&str>,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        // Serialize response according to requested format
        // For AgentStatus responses, use native protocol serialization to preserve
        // the Response enum variant wrapper for backwards compatibility
        let payload = if matches!(response, Response::AgentStatus { .. }) {
            match protocol::serialize_response(response) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => {
                    return Err(Error::ipc(format!("Failed to serialize response: {e}")));
                }
            }
        } else {
            match self.render_response(response, format, position, prev_bg) {
                Ok(rendered) => rendered,
                Err(e) => {
                    // The client asked for a format the socket does not serve:
                    // answer with a JSON error rather than staying silent (#759).
                    warn_log!("connection", "Failed to render response: {e}");
                    let error = Response::Error {
                        message: e.to_string(),
                    };
                    let config = self.config_manager.get();
                    let theme = self.theme_manager.get();
                    let ctx = RenderContext::new(&config, &theme, position);
                    create_formatter(Format::Json)?.render(&error, &ctx)?
                }
            }
        };

        // Write response with timeout
        let write_result = timeout(self.request_timeout(), async {
            writer.write_all(payload.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await
        })
        .await;

        match write_result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(Error::ipc(format!("Failed to write response: {e}"))),
            Err(_) => {
                let timeout_ms = self.request_timeout().as_millis();
                Err(Error::ipc(format!("Write timeout after {timeout_ms}ms")))
            }
        }
    }

    /// Render a response into the requested output format.
    ///
    /// The `prev_bg` string (a color name or hex code from the IPC wire format) is parsed
    /// into a [`Color`] and threaded into the [`RenderContext`] so the template engine can
    /// render the opening powerline chevron with the correct previous-segment background.
    /// An unrecognised color string gracefully becomes `None` (terminal default background).
    ///
    /// # Errors
    ///
    /// Returns an error if serialization for the requested format fails.
    fn render_response(
        &self,
        response: &Response,
        format: Format,
        position: SegmentPosition,
        prev_bg: Option<&str>,
    ) -> Result<String> {
        match format {
            Format::Fish => Err(Error::ipc(format!(
                "IPC format '{format}' is only available to local CLI oneshot commands."
            ))),
            Format::BashSource | Format::ZshSource => Err(Error::ipc(format!(
                "IPC format {format:?} is not yet implemented for socket clients."
            ))),
            Format::Ansi
            | Format::BashPrompt
            | Format::ZshPrompt
            | Format::Json
            | Format::FishSource
            | Format::Zsh => {
                let config = self.config_manager.get();
                let theme = self.theme_manager.get();
                let palette = self.palette_cache.get();
                let parsed_prev_bg: Option<Color> = prev_bg.and_then(|s| parse_color(s).ok());
                let ctx = RenderContext::new(&config, &theme, position)
                    .with_palette(palette)
                    .with_prev_colors(None, parsed_prev_bg);
                create_formatter(format)?.render(response, &ctx)
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    missing_docs
)]
mod tests {
    use super::*;
    use crate::config::manager::ConfigManager;
    use crate::formatter::Format;
    use crate::ipc::handlers::{
        ClientHandler, GitHandler, HandlerRegistry, LanguageHandler, MiscHandler, RenderDeps,
        ThemeHandler,
    };
    use crate::ipc::registry::ClientDirectory;
    use crate::palette::PaletteCache;
    use crate::security::SafePath;
    use std::sync::Mutex;

    /// Build a `HandlerRegistry` wired the same way production does (mirrors
    /// the handler wiring in `EndpointHandle::with_path_and_state`).
    ///
    /// Adds the standalone `InstantPromptCache`/`DetectionCache` the language
    /// handler shares -- exactly what `parse_and_route_blocking` needs for a
    /// `LanguageDetect` request.
    fn test_handler_registry() -> (
        Arc<HandlerRegistry>,
        Arc<crate::cache::InstantPromptCache>,
        crate::language::DetectionCache,
    ) {
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
        let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
        let theme_manager = Arc::new(ThemeManager::builtin("default").expect("theme"));
        let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
        let language_cache = crate::language::DetectionCache::new();
        let client_registry = Arc::new(ClientDirectory::new());
        let watcher: crate::ipc::server::handle::WatcherRef = Arc::new(Mutex::new(None));
        let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));

        let render = RenderDeps {
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            palette_cache: Arc::clone(&palette_cache),
            instant_cache: Arc::clone(&instant_cache),
            client_registry: Arc::clone(&client_registry),
        };

        let git_handler = Arc::new(GitHandler::new(render.clone(), Arc::clone(&git_cache)));
        let language_handler =
            Arc::new(LanguageHandler::new(render.clone(), language_cache.clone()));
        let theme_handler = Arc::new(ThemeHandler::new(Arc::clone(&theme_manager)));
        let client_handler = Arc::new(ClientHandler::new(
            render,
            Arc::clone(&git_cache),
            Arc::clone(&watcher),
            Arc::clone(&latency_tracker),
        ));
        let misc_handler = Arc::new(MiscHandler::new());

        let handler_registry = Arc::new(HandlerRegistry::new(
            git_handler,
            language_handler,
            theme_handler,
            client_handler,
            misc_handler,
        ));

        (handler_registry, instant_cache, language_cache)
    }

    /// #680: a prompt request whose `cwd` fails path validation is answered
    /// in the format it asked for, so the shell gets an empty segment rather
    /// than a JSON error line printed into the prompt.
    #[test]
    fn ansi_request_with_denied_path_keeps_ansi_format() {
        let (registry, _instant_cache, _language_cache) = test_handler_registry();
        let result = ConnectionHandler::parse_and_route_blocking(
            br#"{"op":"directory","cwd":"/etc","format":"ansi"}"#,
            &registry,
            &GuardSettings::default(),
        );
        assert_eq!(result.format, Format::Ansi);
        assert!(
            matches!(result.response, Response::Error { .. }),
            "denied path must still be an error, got {:?}",
            result.response
        );
    }

    /// #570: a single `LanguageDetect` request must write each of the 4
    /// variant cache files (`lang`, `lang_last`, `lang_first`,
    /// `lang_first_last`) exactly once, not twice.
    ///
    /// Before the fix, the handler wrote all 4 variants via
    /// `write_language_variants`, then `parse_and_route_blocking` wrote the
    /// same 4 files again via the now-removed
    /// `write_language_instant_cache_blocking`, for 8 total `write_cache_file`
    /// calls.
    ///
    /// The detection result is pre-seeded into the `DetectionCache` so the
    /// request takes the synchronous cache-hit path in
    /// `handle_language_detection` rather than spawning the background
    /// single-flight detection job -- that job independently calls
    /// `write_language_variants` again on its own thread once it finishes,
    /// which would race this test's write-count assertion.
    #[test]
    fn language_request_writes_each_variant_file_once() {
        let (handler_registry, instant_cache, language_cache) = test_handler_registry();
        let security_config = GuardSettings::default();

        let repo = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(repo.path().join("Cargo.toml"), "[package]\nname = \"t\"\n")
            .expect("write Cargo.toml");
        let repo_root = std::fs::canonicalize(repo.path()).expect("canonicalize repo path");
        language_cache.set(
            &repo_root,
            vec![crate::language::DetectedLanguage {
                name: "rust".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 1,
            }],
        );

        let message = Message::LanguageDetect {
            path: SafePath::new(repo.path().to_str().expect("utf8 path")).expect("safe path"),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        };
        let line = protocol::serialize_message(&message).expect("serialize message");

        let before = instant_cache.write_call_count();
        let result =
            ConnectionHandler::parse_and_route_blocking(&line, &handler_registry, &security_config);
        assert!(
            !matches!(result.response, Response::Error { .. }),
            "unexpected error response: {:?}",
            result.response
        );

        // 4 position variants, each written once per output dialect (#677).
        let expected = 4 * crate::formatter::PromptDialect::ALL.len();
        let calls = instant_cache.write_call_count() - before;
        assert_eq!(
            calls,
            u64::try_from(expected).expect("small count"),
            "one language request must write each of the 4 variant files exactly once per dialect, not twice (#570)"
        );
    }
}
