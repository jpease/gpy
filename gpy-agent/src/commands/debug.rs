//! Debugging tools for GPY agent

use crate::Result;
#[cfg(unix)]
use crate::agent::lifecycle::get_socket_path;
#[cfg(unix)]
use crate::formatter::Format;
#[cfg(unix)]
use crate::ipc::Message;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::time::{Duration, Instant};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(unix)]
use tokio::time::timeout;

#[cfg(unix)]
const CONNECT_TIMEOUT_MS: u64 = 250;
#[cfg(unix)]
const REQUEST_TIMEOUT_MS: u64 = 500;

/// Send a request to the agent and wait for a response using newline-delimited JSON
///
/// # Errors
///
/// Returns an error if serialization, socket I/O, or deserialization fails.
#[cfg(unix)]
async fn send_request(stream: &mut UnixStream, msg: &Message) -> Result<Duration> {
    let timeout_duration = Duration::from_millis(REQUEST_TIMEOUT_MS);

    timeout(timeout_duration, async {
        let start = Instant::now();

        // 1. Serialize message to JSON
        // Use serde_json directly to ensure we send the native format the server expects
        let mut json = serde_json::to_string(msg).map_err(|e| crate::Error::ipc(e.to_string()))?;
        json.push('\n');

        // 2. Write to socket
        stream
            .write_all(json.as_bytes())
            .await
            .map_err(|e| crate::Error::ipc(e.to_string()))?;
        stream
            .flush()
            .await
            .map_err(|e| crate::Error::ipc(e.to_string()))?;

        // 3. Read response (line-based)
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];

        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| crate::Error::ipc(e.to_string()))?;
            if n == 0 {
                return Err(crate::Error::ipc("Connection closed by agent".to_owned()));
            }

            #[expect(
                clippy::indexing_slicing,
                reason = "safe because n <= chunk.len(), n is the exact byte count read() just reported"
            )]
            buffer.extend_from_slice(&chunk[..n]);

            if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                #[expect(
                    clippy::indexing_slicing,
                    reason = "safe because pos was just found by position() as a valid index into buffer"
                )]
                let line = &buffer[..pos];
                // 4. Validate response parsing (to ensure we got a valid response)
                // Note: The server sends formatted JSON which doesn't match the Response enum structure directly.
                // We just verify it's valid JSON.
                let _json: serde_json::Value = serde_json::from_slice(line).map_err(|e| {
                    let line_str = String::from_utf8_lossy(line);
                    crate::Error::ipc(format!(
                        "Failed to deserialize response: {e}. Raw: {line_str}"
                    ))
                })?;
                break;
            }
        }

        Ok(start.elapsed())
    })
    .await
    .map_err(|_| crate::Error::ipc(format!("Request timed out after {REQUEST_TIMEOUT_MS}ms")))?
}

/// Run the debug prompt timing analysis
///
/// This command connects to the running agent and simulates a prompt render sequence
/// (Git status + Language detection) to measure IPC roundtrip times.
///
/// # Errors
///
/// Returns an error if connection fails, I/O fails, or socket path cannot be found.
#[expect(
    clippy::print_stdout,
    reason = "this is an interactive CLI debug command; its whole output is timing results printed to stdout"
)]
#[expect(
    clippy::use_debug,
    reason = "Duration has no Display impl; `{:.2?}` is the standard way to print a rounded duration"
)]
#[cfg(unix)]
pub fn prompt() -> Result<()> {
    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        println!("Agent is not running. Please start it with 'gpy start'.");
        return Ok(());
    }

    println!("GPY Prompt Timing Debugger");
    println!("==========================");
    println!("Socket: {}\n", socket_path.display());

    // We need an async runtime for IPC
    tokio::runtime::Runtime::new()?.block_on(async {
        // Connect to agent
        let mut stream = timeout(
            Duration::from_millis(CONNECT_TIMEOUT_MS),
            UnixStream::connect(&socket_path),
        )
        .await
        .map_err(|_| {
            crate::Error::ipc(format!(
                "Connection timed out after {CONNECT_TIMEOUT_MS}ms. Is the agent hung?"
            ))
        })?
        .map_err(|e| crate::Error::ipc(format!("Failed to connect to agent: {e}")))?;

        let cwd_str = std::env::current_dir()?.to_string_lossy().to_string();
        let safe_cwd = crate::security::SafePath::new(&cwd_str)?;

        // 1. Measure Ping (Baseline Latency)
        print!("Ping (Baseline):      ");
        std::io::stdout().flush()?;
        let ping_msg = Message::Ping;
        let ping_duration = send_request(&mut stream, &ping_msg).await?;
        println!("{ping_duration:.2?}");

        // 2. Measure Git Status
        print!("Git Status:           ");
        std::io::stdout().flush()?;

        let git_msg = Message::RepositoryStatus {
            path: safe_cwd.clone(),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        };
        let git_duration = send_request(&mut stream, &git_msg).await?;
        println!("{git_duration:.2?}");

        // 3. Measure Language Detection
        print!("Language Detection:   ");
        std::io::stdout().flush()?;

        let lang_msg = Message::LanguageDetect {
            path: safe_cwd,
            format: Format::Json,
            is_last: true,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        };
        let lang_duration = send_request(&mut stream, &lang_msg).await?;
        println!("{lang_duration:.2?}");

        println!("--------------------------");
        let total = git_duration.checked_add(lang_duration).unwrap_or_default();
        println!("Total Render Estimate: {total:.2?}");
        println!("\n(Note: Timings are roundtrip latencies including serialization, transport, and agent processing.)");

        Ok(())
    })
}

/// Run the debug prompt timing analysis -- native Windows stub (#284).
///
/// GPY's IPC transport is Unix-socket based and not yet implemented on native
/// Windows; this diagnostic is unavailable there.
///
/// # Errors
///
/// Never returns an error on this platform.
#[expect(
    clippy::print_stdout,
    reason = "this is an interactive CLI stub message printed to stdout, matching the Unix variant's output channel"
)]
#[cfg(not(unix))]
pub fn prompt() -> Result<()> {
    println!("Prompt timing debugger is not available on native Windows; run under WSL.");
    Ok(())
}
