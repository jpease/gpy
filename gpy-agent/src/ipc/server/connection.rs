//! Per-connection request serving. [`ConnectionHandler`] owns only what one
//! accepted connection needs; transport concerns stay in `EndpointHandle`.

use crate::formatter::{
    DirectoryEnv, Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter,
};
use crate::ipc::{Message, Response, Segment, handlers::HandlerRegistry, protocol};
use crate::palette::PaletteCache;
use crate::security::{GuardSettings, validate_pid_liveness};
use crate::template::{Color, parse_color};
use crate::theme::ThemeManager;
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
    /// Render position and format; the default (JSON, middle) for control messages.
    pub(super) segment: Segment,
}

/// Result of [`ConnectionHandler::parse_and_route_blocking`].
struct BlockingLineResult {
    should_shutdown: bool,
    payload: Result<String>,
}

/// Serves a single already-accepted client connection.
pub(super) struct ConnectionHandler {
    pub(super) security_config: GuardSettings,
    pub(super) config_manager: Arc<crate::config::manager::ConfigManager>,
    pub(super) theme_manager: Arc<ThemeManager>,
    pub(super) palette_cache: Arc<PaletteCache>,
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
    /// A line whose newline arrives before the buffer passes `max_message_size`
    /// is size-checked by `protocol::validate_message_size` and answered; once
    /// the buffer holds more than that without a newline, the connection is
    /// closed without a reply. A reply rings no doorbell: handlers repaint
    /// live shells only when the instant cache changed
    /// (`handlers::notify_if_changed`).
    ///
    /// # Errors
    /// Fails on non-recoverable read/write or serialization errors.
    #[cfg(unix)]
    pub(super) async fn handle_client(&self, stream: UnixStream) -> Result<()> {
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

                    if self
                        .process_complete_lines(&mut accumulated, &mut writer)
                        .await?
                    {
                        return Ok(());
                    }

                    // Only an unterminated line remains; a complete one was
                    // size-checked by the parser.
                    if accumulated.len() > self.security_config.max_message_size {
                        warn_log!(
                            "connection",
                            "Client exceeded max message size ({} > {} bytes without a newline)",
                            accumulated.len(),
                            self.security_config.max_message_size
                        );
                        break;
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

            let shutdown_requested = self.process_message_line(line_data, writer).await?;

            accumulated.drain(..=newline_pos);

            if shutdown_requested {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Parse, route and render one line on the blocking pool. Parsing
    /// canonicalizes paths, routing can block on single-flight `Condvar`s and
    /// rendering resolves directories, so a hung NFS cwd must not wedge a
    /// Tokio worker.
    #[cfg(unix)]
    async fn route_line_blocking(&self, line_data: &[u8]) -> BlockingLineResult {
        let line_data_owned = line_data.to_vec();
        let handler_registry = Arc::clone(&self.handler_registry);
        let security_config = self.security_config.clone();
        let config_manager = Arc::clone(&self.config_manager);
        let theme_manager = Arc::clone(&self.theme_manager);
        let palette_cache = Arc::clone(&self.palette_cache);

        let blocking_result = tokio::task::spawn_blocking(move || {
            Self::parse_and_route_blocking(
                &line_data_owned,
                &handler_registry,
                &security_config,
                &config_manager,
                &theme_manager,
                &palette_cache,
            )
        })
        .await;

        match blocking_result {
            Ok(result) => result,
            Err(join_err) => {
                let response = Response::Error {
                    message: format!("Request processing failed: {join_err}"),
                };
                // Like a refused request, answered with a `{"error":...}` line in
                // any format. A JSON error render touches no filesystem; safe on a worker.
                let payload = Self::render_payload(
                    &response,
                    Format::Json,
                    SegmentPosition::MIDDLE,
                    None,
                    &self.config_manager,
                    &self.theme_manager,
                    &self.palette_cache,
                );
                BlockingLineResult {
                    should_shutdown: false,
                    payload,
                }
            }
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
    ) -> Result<bool> {
        let BlockingLineResult {
            should_shutdown,
            payload,
        } = self.route_line_blocking(line_data).await;

        if let Err(e) = self.serialize_and_send_response(payload, writer).await {
            warn_log!("connection", "Failed to send response: {e}");
        }

        // An accepted shutdown must take effect even if the requester timed
        // out and disconnected before receiving its acknowledgement.
        if should_shutdown {
            let _ = self.shutdown_tx.send(());
        }

        Ok(should_shutdown)
    }

    /// Blocking: parse, route and render one wire message (see [`Self::route_line_blocking`]).
    fn parse_and_route_blocking(
        line_data: &[u8],
        handler_registry: &HandlerRegistry,
        security_config: &GuardSettings,
        config_manager: &crate::config::manager::ConfigManager,
        theme_manager: &ThemeManager,
        palette_cache: &PaletteCache,
    ) -> BlockingLineResult {
        let parsed = protocol::deserialize_message(line_data);
        match parsed {
            Ok(message) => {
                let is_shutdown = matches!(message, Message::Shutdown);

                let route_result =
                    Self::route_request_secure(handler_registry, security_config, &message);

                let response = route_result.response;
                let segment = route_result.segment;
                let position = SegmentPosition::new(
                    IsLast::from(segment.is_last),
                    IsFirst::from(segment.is_first),
                );
                let payload = Self::render_payload(
                    &response,
                    segment.format,
                    position,
                    segment.prev_bg.as_deref(),
                    config_manager,
                    theme_manager,
                    palette_cache,
                );
                BlockingLineResult {
                    should_shutdown: is_shutdown,
                    payload,
                }
            }
            // A request the agent refuses (bad JSON, unknown op, missing field,
            // denylisted or invalid path) gets a `{"error":...}` line in every
            // format. An empty line would read as "agent alive but late" to the
            // shells, which then skip their oneshot fallback; an error line is a
            // permanent refusal they never print or memoize.
            Err(e) => {
                let response = Response::Error {
                    message: format!("Parse error: {e}"),
                };
                let payload = Self::render_payload(
                    &response,
                    Format::Json,
                    SegmentPosition::MIDDLE,
                    None,
                    config_manager,
                    theme_manager,
                    palette_cache,
                );
                BlockingLineResult {
                    should_shutdown: false,
                    payload,
                }
            }
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
            && let Err(e) = validate_pid_liveness(pid.get())
        {
            return RouteResult {
                response: Response::Error {
                    message: format!("Invalid PID: {e}"),
                },
                segment: Segment::default(),
            };
        }

        let response = handler_registry
            .route(msg)
            .unwrap_or_else(|e| Response::Error {
                message: e.to_string(),
            });
        RouteResult {
            response,
            segment: msg.segment().cloned().unwrap_or_default(),
        }
    }

    /// Render a reply line in the requested format.
    ///
    /// # Errors
    /// Fails if serialization fails.
    fn render_payload(
        response: &Response,
        format: Format,
        position: SegmentPosition,
        prev_bg: Option<&str>,
        config_manager: &crate::config::manager::ConfigManager,
        theme_manager: &ThemeManager,
        palette_cache: &PaletteCache,
    ) -> Result<String> {
        match Self::render_response(
            config_manager,
            theme_manager,
            palette_cache,
            response,
            format,
            position,
            prev_bg,
        ) {
            Ok(rendered) => Ok(rendered),
            Err(e) => {
                // Rendering failed: answer with a JSON error rather than staying silent.
                warn_log!("connection", "Failed to render response: {e}");
                let error = Response::Error {
                    message: e.to_string(),
                };
                let config = config_manager.get();
                let theme = theme_manager.get();
                let ctx = RenderContext::new(&config, &theme, position);
                create_formatter(Format::Json).render(&error, &ctx)
            }
        }
    }

    /// Write a reply already rendered on the blocking pool.
    ///
    /// # Errors
    /// Fails if rendering failed or writing fails.
    #[cfg(unix)]
    async fn serialize_and_send_response(
        &self,
        payload: Result<String>,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let rendered = payload?;

        let write_result = timeout(self.request_timeout(), async {
            writer
                .write_all(protocol::encode_reply_newlines(&rendered).as_bytes())
                .await?;
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
    /// A directory rendered as prompt text gets its [`DirectoryEnv`] probed here.
    ///
    /// # Errors
    /// Fails if serialization fails.
    fn render_response(
        config_manager: &crate::config::manager::ConfigManager,
        theme_manager: &ThemeManager,
        palette_cache: &PaletteCache,
        response: &Response,
        format: Format,
        position: SegmentPosition,
        prev_bg: Option<&str>,
    ) -> Result<String> {
        let config = config_manager.get();
        let theme = theme_manager.get();
        let palette = palette_cache.get();
        let parsed_prev_bg: Option<Color> = prev_bg.and_then(|s| parse_color(s).ok());
        let directory = DirectoryEnv::for_response(response, format, &config.ui.directory);
        let ctx = RenderContext::new(&config, &theme, position)
            .with_palette(palette)
            .with_prev_colors(None, parsed_prev_bg)
            .with_directory(&directory);
        create_formatter(format).render(response, &ctx)
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
    use crate::ipc::fixtures::default_render_deps;
    use crate::ipc::handlers::{
        ClientHandler, GitHandler, HandlerRegistry, LanguageHandler, RenderDeps,
    };
    use crate::ipc::registry::ClientDirectory;
    use std::sync::Mutex;

    /// Production-wired `HandlerRegistry`, the render deps it was built from, and
    /// the language cache its handler shares.
    fn test_handler_registry() -> (
        Arc<HandlerRegistry>,
        RenderDeps,
        crate::language::DetectionCache,
    ) {
        handler_registry_over(default_render_deps())
    }

    /// [`test_handler_registry`] over the given render deps.
    fn handler_registry_over(
        render: RenderDeps,
    ) -> (
        Arc<HandlerRegistry>,
        RenderDeps,
        crate::language::DetectionCache,
    ) {
        let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
        let language_cache = crate::language::DetectionCache::new();
        let watcher: crate::ipc::server::handle::WatcherRef = Arc::new(Mutex::new(None));

        let git_handler = Arc::new(GitHandler::new(render.clone(), Arc::clone(&git_cache)));
        let language_handler =
            Arc::new(LanguageHandler::new(render.clone(), language_cache.clone()));
        let client_handler = Arc::new(ClientHandler::new(
            render.clone(),
            Arc::clone(&git_handler),
            Arc::clone(&watcher),
        ));

        let handler_registry = Arc::new(HandlerRegistry::new(
            git_handler,
            language_handler,
            client_handler,
        ));

        (handler_registry, render, language_cache)
    }

    /// A connection handler over `render` and `handler_registry`.
    fn connection_handler(
        render: &RenderDeps,
        handler_registry: Arc<HandlerRegistry>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> ConnectionHandler {
        ConnectionHandler {
            security_config: GuardSettings::default(),
            config_manager: Arc::clone(&render.config_manager),
            theme_manager: Arc::clone(&render.theme_manager),
            palette_cache: Arc::clone(&render.palette_cache),
            shutdown_tx,
            handler_registry,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_survives_client_disconnect_before_acknowledgement() {
        let (handler_registry, render, _language_cache) = test_handler_registry();
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        let handler = connection_handler(&render, handler_registry, shutdown_tx);
        let (client, server) = UnixStream::pair().expect("socket pair");
        let (_reader, mut writer) = server.into_split();
        drop(client);
        assert!(
            handler
                .process_message_line(br#"{"op":"shutdown"}"#, &mut writer)
                .await
                .expect("handle shutdown")
        );
        assert_eq!(
            shutdown_rx.try_recv(),
            Ok(()),
            "a disconnected client must not cancel an accepted shutdown"
        );
    }

    /// A reply rings no doorbell by itself; the only ring is the handler's
    /// change-gated publish. A first git reply therefore rings once, not
    /// twice, and an unchanged repeat rings nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn git_reply_rings_only_for_a_published_change() {
        let mut counting = default_render_deps();
        let (registry, rings) = ClientDirectory::counting_rings();
        counting.client_registry = registry.shared();
        let (handler_registry, render, _language_cache) = handler_registry_over(counting);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);
        let handler = connection_handler(&render, handler_registry, shutdown_tx);
        let (_client, server) = UnixStream::pair().expect("socket pair");
        let (_reader, mut writer) = server.into_split();

        let repo = tempfile::TempDir::new().expect("temp dir");
        let init = std::process::Command::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .expect("git init");
        assert!(init.status.success(), "git init failed: {init:?}");
        let line = serde_json::json!({"op": "git", "cwd": repo.path().to_str().expect("utf8 path"), "format": "json"})
            .to_string();

        handler
            .process_message_line(line.as_bytes(), &mut writer)
            .await
            .expect("first reply");
        assert_eq!(
            rings.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the first git reply rings once, for the instant cache it published"
        );

        handler
            .process_message_line(line.as_bytes(), &mut writer)
            .await
            .expect("repeat reply");
        assert_eq!(
            rings.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an unchanged repeat reply rings nothing"
        );
    }

    /// A refused request gets a `{"error":...}` line in every format, never the
    /// empty line that shells read as "agent alive but late".
    #[test]
    fn refused_requests_get_json_error_lines_in_every_format() {
        let (registry, render, _language_cache) = test_handler_registry();
        let esc = '\u{1b}';
        for format in ["ansi", "bash-prompt", "zsh-prompt", "json"] {
            let requests = [
                // Denylisted cwd.
                format!(r#"{{"op":"directory","cwd":"/etc","format":"{format}"}}"#),
                // Unknown op.
                format!(r#"{{"op":"teleport","format":"{format}"}}"#),
                // Raw C0 control byte: not valid JSON at all.
                format!(r#"{{"op":"directory","cwd":"/tmp/a{esc}b","format":"{format}"}}"#),
            ];
            for request in requests {
                let result = ConnectionHandler::parse_and_route_blocking(
                    request.as_bytes(),
                    &registry,
                    &GuardSettings::default(),
                    &render.config_manager,
                    &render.theme_manager,
                    &render.palette_cache,
                );
                let payload = result.payload.expect("rendered");
                assert!(
                    payload.starts_with(r#"{"error""#),
                    "refused {request:?} must get a JSON error line, got {payload:?}"
                );
            }
        }
    }

    /// One `LanguageDetect` writes each variant file once per dialect. The cache is
    /// pre-seeded so the background detection job (which also writes) doesn't race.
    #[test]
    fn language_request_writes_each_variant_file_once() {
        let (handler_registry, render, language_cache) = test_handler_registry();
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

        let line = serde_json::json!({"op": "lang", "cwd": repo.path().to_str().expect("utf8 path"), "format": "json"})
            .to_string()
            .into_bytes();

        let before = render.instant_cache.write_call_count();
        let result = ConnectionHandler::parse_and_route_blocking(
            &line,
            &handler_registry,
            &security_config,
            &render.config_manager,
            &render.theme_manager,
            &render.palette_cache,
        );
        let payload = result.payload.expect("rendered");
        assert!(
            !payload.starts_with(r#"{"error""#),
            "unexpected error reply: {payload}"
        );

        // 4 position variants, each written once per output dialect.
        let expected = 4 * crate::formatter::PromptDialect::ALL.len();
        let calls = render.instant_cache.write_call_count() - before;
        assert_eq!(
            calls,
            u64::try_from(expected).expect("small count"),
            "one language request must write each of the 4 variant files exactly once per dialect, not twice"
        );
    }
}
