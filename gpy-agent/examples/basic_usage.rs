//! Render git status and language detection in-process, without a daemon.
//!
//! Uses `Agent::handle_oneshot`, the same path `gpy-agent oneshot` takes
//! when no agent is running.
//! Run with `cargo run --example basic_usage [DIR]` (defaults to `.`).

#![allow(clippy::print_stdout)]

use gpy_agent::agent::Agent;

fn main() -> gpy_agent::Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());

    for kind in ["git_status", "language_detect"] {
        let request = serde_json::json!({ "type": kind, "path": dir, "format": "json" });
        let response = Agent::handle_oneshot(&request.to_string())?;
        println!("{kind}: {response}");
    }

    Ok(())
}
