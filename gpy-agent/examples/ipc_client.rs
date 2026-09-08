//! IPC client example
//!
//! Demonstrates how to communicate with a running GPY agent via IPC:
//! - Sending requests to the agent
//! - Receiving responses
//! - Handling different request types
//!
//! Note: This example requires a running GPY agent daemon.
//! Start the agent with: `gpy-agent start`

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(clippy::unwrap_used)]

use gpy_agent::{Result, agent::Agent};

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    println!("GPY IPC Client Example\n");
    println!("This example demonstrates communicating with a running GPY agent.");
    println!("Make sure the agent is running: gpy-agent start\n");

    // Example 1: Git status request (JSON format)
    println!("--- Example 1: Git Status (JSON) ---");
    let git_request = r#"{
        "op": "git",
        "cwd": ".",
        "format": "json"
    }"#;

    match Agent::handle_oneshot(git_request) {
        Ok(response) => {
            println!("Git status response:");
            // Parse and pretty-print JSON
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or(response)
                );
            } else {
                println!("{response}");
            }
        }
        Err(e) => eprintln!("Error getting git status: {e}"),
    }

    // Example 2: Language detection request
    println!("\n--- Example 2: Language Detection ---");
    let lang_request = r#"{
        "op": "lang",
        "cwd": ".",
        "format": "json"
    }"#;

    match Agent::handle_oneshot(lang_request) {
        Ok(response) => {
            println!("Language detection response:");
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or(response)
                );
            } else {
                println!("{response}");
            }
        }
        Err(e) => eprintln!("Error detecting language: {e}"),
    }

    // Example 3: Agent ping/status request
    println!("\n--- Example 3: Agent Status ---");
    let ping_request = r#"{"op": "ping"}"#;

    match Agent::handle_oneshot(ping_request) {
        Ok(response) => {
            println!("Agent status response:");
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or(response)
                );
            } else {
                println!("{response}");
            }
        }
        Err(e) => eprintln!("Error pinging agent: {e}"),
    }

    // Example 4: Fish shell format (fish-rendered)
    println!("\n--- Example 4: Fish Shell Format ---");
    let fish_request = r#"{
        "op": "git",
        "cwd": ".",
        "format": "fish-rendered"
    }"#;

    match Agent::handle_oneshot(fish_request) {
        Ok(response) => {
            println!("Fish-rendered git status:");
            println!("{response}");
        }
        Err(e) => eprintln!("Error getting fish-rendered status: {e}"),
    }

    println!("\n✓ IPC client example completed");
    println!("\nNote: If you see errors, ensure the agent is running with:");
    println!("  gpy-agent start");

    Ok(())
}
