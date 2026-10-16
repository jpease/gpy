//! Per-connection request serving. [`ConnectionHandler`] owns only what one
//! accepted connection needs; transport concerns stay in `EndpointHandle`.

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
pub(super) struct RouteResult {
    pub(super) response: Response,
    pub(super) format: Format,
    pub(super) notify_path: Option<String>,
    pub(super) is_last: bool,
    pub(super) is_first: bool,
    pub(super) prev_bg: Option<String>,
}

/// Result of [`ConnectionHandler::parse_and_route_blocking`]; `route_elapsed` is
/// `None` when parsing failed.
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

    /// Handle a single client connection (NDJSON).
    ///
    /// # Errors
    /// Fails on non-recoverable read/write or serialization errors.
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
                Ok(Ok(0)) => break,
                Ok(Ok(read_bytes)) => {
                    if let Some(slice) = buffer.get(..read_bytes) {
                        accumulated.extend_from_slice(slice);
                    } else {
                        break;
                    }

                    if accumulated.len() > self.security_config.max_message_size {
                        warn_log!(
                            "connection",
                            "Client exceeded max message size ({} > {} bytes)",
                            accumulated.len(),
                            self.security_config.max_message_size
                        );
                        break;
                    }

                    if self
                        .process_complete_lines(&mut accumulated, &mut writer, &client_registry)
                        .await?
                    {
                        return Ok(());
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

    /// Process all complete lines; returns `Ok(true)` if shutdown was requested.
    ///
    /// # Errors
    /// Fails if message processing fails.
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

            if line_data.is_empty() {
                accumulated.drain(..=newline_pos);
                continue;
            }

            let shutdown_requested = self
                .process_message_line(line_data, writer, client_registry)
                .await?;

            accumulated.drain(..=newline_pos);

            if shutdown_requested {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Parse and route one line on the blocking pool. Parsing canonicalizes
    /// paths and routing can block on single-flight `Condvar`s, so a hung
    /// NFS cwd must not wedge a Tokio worker.
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

    /// Process one message line and write the response; returns `Ok(true)` on shutdown.
    ///
    /// # Errors
    /// Fails if processing, serialization, or writing fails.
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

        if let Err(e) = self
            .serialize_and_send_response(&response, format, position, prev_bg.as_deref(), writer)
            .await
        {
            warn_log!("connection", "Failed to send response: {e}");
        }

        // An accepted shutdown must take effect even if the requester timed
        // out and disconnected before receiving its acknowledgement.
        if should_shutdown {
            let _ = self.shutdown_tx.send(());
        }

        Ok(should_shutdown)
    }

    /// Blocking: parse and route one wire message (see [`Self::route_line_blocking`]).
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
            // Answer in the requested format when known, so a prompt gets an
            // empty segment rather than protocol JSON.
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
    /// MUST run on the blocking pool: handlers may wait on cache misses.
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

    /// Connection-scoped wrapper over `notify_clients_for_response`.
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

    /// Serialize and send a response. `AgentStatus` uses native protocol
    /// serialization to keep the enum wrapper for backwards compatibility.
    ///
    /// # Errors
    /// Fails if serialization or writing fails.
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
                    // answer with a JSON error rather than staying silent.
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

    /// Render a response into the requested format; an unparseable `prev_bg` becomes `None`.
    ///
    /// # Errors
    /// Fails if the format is unsupported or serialization fails.
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

    /// Production-wired `HandlerRegistry` plus the caches the language handler shares.
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

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_survives_client_disconnect_before_acknowledgement() {
        let (handler_registry, _instant_cache, _language_cache) = test_handler_registry();
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        let theme_manager = Arc::new(ThemeManager::builtin("default").expect("theme"));
        let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        let handler = ConnectionHandler {
            security_config: GuardSettings::default(),
            config_manager,
            theme_manager,
            palette_cache,
            latency_tracker: Arc::new(crate::ipc::LatencyTracker::new(100)),
            shutdown_tx,
            handler_registry,
        };
        let (client, server) = UnixStream::pair().expect("socket pair");
        let (_reader, mut writer) = server.into_split();
        drop(client);
        let registry = Arc::new(ClientDirectory::new());
        assert!(
            handler
                .process_message_line(br#"{"op":"shutdown"}"#, &mut writer, &registry)
                .await
                .expect("handle shutdown")
        );
        assert_eq!(
            shutdown_rx.try_recv(),
            Ok(()),
            "a disconnected client must not cancel an accepted shutdown"
        );
    }

    /// A denied `cwd` is answered in the requested format, not a JSON error line.
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

    /// One `LanguageDetect` writes each variant file once per dialect. The cache is
    /// pre-seeded so the background detection job (which also writes) doesn't race.
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

        // 4 position variants, each written once per output dialect.
        let expected = 4 * crate::formatter::PromptDialect::ALL.len();
        let calls = instant_cache.write_call_count() - before;
        assert_eq!(
            calls,
            u64::try_from(expected).expect("small count"),
            "one language request must write each of the 4 variant files exactly once per dialect, not twice"
        );
    }
}
