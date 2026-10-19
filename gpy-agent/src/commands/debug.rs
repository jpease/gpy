//! Debugging tools for the GPY agent.

use crate::Result;
#[cfg(unix)]
use crate::agent::lifecycle::get_socket_path;
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
async fn send_request(stream: &mut UnixStream, msg: &serde_json::Value) -> Result<Duration> {
    let timeout_duration = Duration::from_millis(REQUEST_TIMEOUT_MS);

    timeout(timeout_duration, async {
        let start = Instant::now();

        let mut json = msg.to_string();
        json.push('\n');

        stream
            .write_all(json.as_bytes())
            .await
            .map_err(|e| crate::Error::ipc(e.to_string()))?;
        stream
            .flush()
            .await
            .map_err(|e| crate::Error::ipc(e.to_string()))?;

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

            buffer.extend_from_slice(&chunk[..n]);

            if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line = &buffer[..pos];
                // The server sends formatted output, not a `Response`; only check it is JSON.
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

/// Time agent round trips (ping, git status, language detection) for the cwd.
///
/// # Errors
/// Returns an error if the agent is not running or IPC fails.
#[expect(
    clippy::print_stdout,
    reason = "this is an interactive CLI debug command; its whole output is timing results printed to stdout"
)]
#[cfg(unix)]
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
        .map_err(|e| crate::Error::ipc(format!("Failed to connect to agent: {e}")))?;

        let cwd_str = std::env::current_dir()?.to_string_lossy().to_string();
        let safe_cwd = crate::security::SafePath::new(&cwd_str)?;

        print!("Ping (Baseline):      ");
        std::io::stdout().flush()?;
        let ping_duration = send_request(&mut stream, &serde_json::json!({"op": "ping"})).await?;
        println!("{ping_duration:.2?}");

        print!("Git Status:           ");
        std::io::stdout().flush()?;
        let git_duration = send_request(
            &mut stream,
            &serde_json::json!({"op": "git", "cwd": safe_cwd.as_str(), "format": "json"}),
        )
        .await?;
        println!("{git_duration:.2?}");

        print!("Language Detection:   ");
        std::io::stdout().flush()?;
        let lang_duration = send_request(
            &mut stream,
            &serde_json::json!({"op": "lang", "cwd": safe_cwd.as_str(), "format": "json", "is_last": true}),
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
#[expect(
    clippy::print_stdout,
    reason = "this is an interactive CLI stub message printed to stdout, matching the Unix variant's output channel"
)]
#[cfg(not(unix))]
pub fn prompt() -> Result<()> {
    println!("Prompt timing debugger is not available on native Windows; run under WSL.");
    Ok(())
}
