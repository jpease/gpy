//! Per-connection request serving. [`ConnectionHandler`] owns only what one
//! accepted connection needs; transport concerns stay in `EndpointHandle`.

use crate::formatter::{
    DirectoryEnv, Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter,
};
use crate::ipc::{
    ClientDirectory, Message, Response, Segment, handlers::HandlerRegistry, protocol,
};
use crate::palette::PaletteCache;
use crate::security::{GuardSettings, PidValidator};
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
    pub(super) notify_path: Option<String>,
    /// Render position and format; the default (JSON, middle) for control messages.
    pub(super) segment: Segment,
}

/// Result of [`ConnectionHandler::parse_and_route_blocking`].
struct BlockingLineResult {
    response: Response,
    notify_path: Option<String>,
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
                // A JSON error render touches no filesystem; safe on a worker.
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
                    response,
                    notify_path: None,
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
        client_registry: &Arc<ClientDirectory>,
    ) -> Result<bool> {
        let BlockingLineResult {
            response,
            notify_path,
            should_shutdown,
            payload,
        } = self.route_line_blocking(line_data).await;

        self.notify_clients(client_registry, &response, notify_path.as_deref());

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
        let (requested_format, parsed) = protocol::deserialize_message_with_format(line_data);
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
                    response,
                    notify_path: route_result.notify_path,
                    should_shutdown: is_shutdown,
                    payload,
                }
            }
            // Answer in the requested format when known, so a prompt gets an
            // empty segment rather than protocol JSON.
            Err(e) => {
                let response = Response::Error {
                    message: format!("Parse error: {e}"),
                };
                let payload = Self::render_payload(
                    &response,
                    requested_format.unwrap_or(Format::Json),
                    SegmentPosition::MIDDLE,
                    None,
                    config_manager,
                    theme_manager,
                    palette_cache,
                );
                BlockingLineResult {
                    response,
                    notify_path: None,
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
            && let Err(e) = PidValidator::validate_pid_liveness(pid.get())
        {
            return RouteResult {
                response: Response::Error {
                    message: format!("Invalid PID: {e}"),
                },
                notify_path: None,
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
            notify_path: msg.notify_path(),
            segment: msg.segment().cloned().unwrap_or_default(),
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
        let render = default_render_deps();
        let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
        let language_cache = crate::language::DetectionCache::new();
        let watcher: crate::ipc::server::handle::WatcherRef = Arc::new(Mutex::new(None));

        let git_handler = Arc::new(GitHandler::new(render.clone(), Arc::clone(&git_cache)));
        let language_handler =
            Arc::new(LanguageHandler::new(render.clone(), language_cache.clone()));
        let client_handler = Arc::new(ClientHandler::new(
            render.clone(),
            Arc::clone(&git_cache),
            Arc::clone(&watcher),
        ));

        let handler_registry = Arc::new(HandlerRegistry::new(
            git_handler,
            language_handler,
            client_handler,
        ));

        (handler_registry, render, language_cache)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_survives_client_disconnect_before_acknowledgement() {
        let (handler_registry, render, _language_cache) = test_handler_registry();
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
        let handler = ConnectionHandler {
            security_config: GuardSettings::default(),
            config_manager: render.config_manager,
            theme_manager: render.theme_manager,
            palette_cache: render.palette_cache,
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
        let (registry, render, _language_cache) = test_handler_registry();
        let result = ConnectionHandler::parse_and_route_blocking(
            br#"{"op":"directory","cwd":"/etc","format":"ansi"}"#,
            &registry,
            &GuardSettings::default(),
            &render.config_manager,
            &render.theme_manager,
            &render.palette_cache,
        );
        let payload = result.payload.expect("rendered");
        assert!(
            !payload.trim_start().starts_with('{'),
            "denied ANSI request must not get a JSON line, got {payload:?}"
        );
        assert!(
            matches!(result.response, Response::Error { .. }),
            "denied path must still be an error, got {:?}",
            result.response
        );
    }

    /// An op the agent does not know is answered in the requested format, not a JSON error line.
    #[test]
    fn ansi_request_with_unknown_op_keeps_ansi_format() {
        let (registry, render, _language_cache) = test_handler_registry();
        let result = ConnectionHandler::parse_and_route_blocking(
            br#"{"op":"teleport","format":"ansi"}"#,
            &registry,
            &GuardSettings::default(),
            &render.config_manager,
            &render.theme_manager,
            &render.palette_cache,
        );
        let payload = result.payload.expect("rendered");
        assert!(
            !payload.trim_start().starts_with('{'),
            "unknown-op ANSI request must not get a JSON line, got {payload:?}"
        );
        assert!(
            matches!(result.response, Response::Error { .. }),
            "unknown op must still be an error, got {:?}",
            result.response
        );
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
        assert!(
            !matches!(result.response, Response::Error { .. }),
            "unexpected error response: {:?}",
            result.response
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
