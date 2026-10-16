//! Talk to a running agent: newline-delimited JSON in the shells' `op` form.
//! Start an agent first (`gpy-agent start`), then run
//! `cargo run --example ipc_client [DIR]` (defaults to `.`).

#![allow(clippy::print_stdout)]

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let dir = std::fs::canonicalize(std::env::args().nth(1).unwrap_or_else(|| ".".to_owned()))?;
    let cwd = dir.to_str().ok_or("directory path is not UTF-8")?;

    let socket_path = gpy_agent::agent::lifecycle::get_socket_path()?;
    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        format!(
            "cannot connect to {}: {e} (is `gpy-agent start` running?)",
            socket_path.display()
        )
    })?;
    let (reader, mut writer) = stream.into_split();
    let mut replies = BufReader::new(reader).lines();

    let requests = [
        serde_json::json!({ "op": "ping" }),
        serde_json::json!({ "op": "git", "cwd": cwd, "format": "json" }),
        serde_json::json!({ "op": "lang", "cwd": cwd, "format": "json" }),
    ];
    for request in requests {
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        writer.write_all(&line).await?;

        let reply = replies
            .next_line()
            .await?
            .ok_or("agent closed the connection")?;
        println!("{request}\n{}\n", pretty(&reply));
    }

    Ok(())
}

/// Pretty-print a JSON reply, or return it unchanged if it is not JSON.
#[cfg(unix)]
fn pretty(reply: &str) -> String {
    serde_json::from_str::<serde_json::Value>(reply)
        .and_then(|json| serde_json::to_string_pretty(&json))
        .unwrap_or_else(|_| reply.to_owned())
}

#[cfg(not(unix))]
fn main() {
    println!("The agent's IPC transport is Unix-socket only.");
}
