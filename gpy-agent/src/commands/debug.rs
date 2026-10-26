//! Debugging tools for the GPY agent.

use crate::Result;
#[cfg(unix)]
use crate::paths::get_socket_path;
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

/// Send one newline-delimited JSON request and return its round-trip time.
#[cfg(unix)]
#[allow(
    clippy::map_err_ignore,
    reason = "the request-timeout Elapsed carries no detail and attaching it would append to the CLI error text"
)]
async fn send_request(stream: &mut UnixStream, msg: &serde_json::Value) -> Result<Duration> {
    let timeout_duration = Duration::from_millis(REQUEST_TIMEOUT_MS);

    timeout(timeout_duration, async {
        let start = Instant::now();

        let mut json = msg.to_string();
        json.push('\n');

        stream
            .write_all(json.as_bytes())
            .await
            .map_err(|e| crate::Error::io_context("Failed to send request to agent", &e))?;
        stream
            .flush()
            .await
            .map_err(|e| crate::Error::io_context("Failed to send request to agent", &e))?;

        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];

        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| crate::Error::io_context("Failed to read response from agent", &e))?;
            if n == 0 {
                return Err(crate::Error::ipc("Connection closed by agent".to_owned()));
            }

            let received = chunk.get(..n).ok_or_else(|| {
                crate::Error::ipc("read reported more bytes than the buffer holds".to_owned())
            })?;
            buffer.extend_from_slice(received);

            if let Some(line) = buffer
                .iter()
                .position(|&b| b == b'\n')
                .and_then(|pos| buffer.get(..pos))
            {
                // The server sends formatted output, not a `Response`; only check it is JSON.
                let _json: serde_json::Value = serde_json::from_slice(line).map_err(|e| {
                    let line_str = String::from_utf8_lossy(line);
                    crate::Error::json(format!(
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

/// Time agent round trips (ping, git status, language detection) for the cwd.
///
/// # Errors
/// Returns an error if the agent is not running or IPC fails.
#[cfg(unix)]
#[allow(
    clippy::use_debug,
    reason = "Duration has no Display impl; the `{:.2?}` timings are this command's user-facing output"
)]
#[allow(
    clippy::map_err_ignore,
    reason = "the connect-timeout Elapsed carries no detail and attaching it would append to the CLI error text"
)]
pub fn prompt() -> Result<()> {
    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        return Err(crate::Error::agent(
            "Agent is not running. Start it with 'gpy start'.",
        ));
    }

    println!("GPY Prompt Timing Debugger");
    println!("==========================");
    println!("Socket: {}\n", socket_path.display());

    tokio::runtime::Runtime::new()?.block_on(async {
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
        .map_err(|e| crate::Error::io_context("Failed to connect to agent", &e))?;

        let cwd_str = std::env::current_dir()?.to_string_lossy().to_string();
        let safe_cwd = crate::security::SafePath::new(&cwd_str)?;
        let safe_cwd_str = safe_cwd
            .as_str()
            .ok_or_else(|| crate::Error::invalid("Current working directory is not valid UTF-8"))?;

        print!("Ping (Baseline):      ");
        std::io::stdout().flush()?;
        let ping_duration = send_request(&mut stream, &serde_json::json!({"op": "ping"})).await?;
        println!("{ping_duration:.2?}");

        print!("Git Status:           ");
        std::io::stdout().flush()?;
        let git_duration = send_request(
            &mut stream,
            &serde_json::json!({"op": "git", "cwd": safe_cwd_str, "format": "json"}),
        )
        .await?;
        println!("{git_duration:.2?}");

        print!("Language Detection:   ");
        std::io::stdout().flush()?;
        let lang_duration = send_request(
            &mut stream,
            &serde_json::json!({"op": "lang", "cwd": safe_cwd_str, "format": "json", "is_last": true}),
        )
        .await?;
        println!("{lang_duration:.2?}");

        println!("--------------------------");
        let total = git_duration.checked_add(lang_duration).unwrap_or_default();
        println!("Git + Language Total: {total:.2?}");
        println!("\n(Note: Timings are roundtrip latencies including serialization, transport, and agent processing.)");

        Ok(())
    })
}

/// Native Windows stub; see `crate::ipc::native_windows_unsupported`.
///
/// # Errors
/// Never returns an error on this platform.
#[cfg(not(unix))]
pub fn prompt() -> Result<()> {
    println!("Prompt timing debugger is not available on native Windows; run under WSL.");
    Ok(())
}
