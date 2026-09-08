//! Basic usage example for GPY Agent library
//!
//! Demonstrates how to use the agent programmatically for
//! git status and language detection.

use gpy_agent::{Result, agent::Agent};

#[tokio::main]
async fn main() -> Result<()> {
    // Create a new agent instance
    let _agent = Agent::new()?;

    // Example 1: Get git status for current directory
    let git_request = r#"{"type":"git_status","path":"."}"#;
    match Agent::handle_oneshot(git_request) {
        Ok(_response) => {}
        Err(_e) => {}
    }

    // Example 2: Detect languages in current directory
    let lang_request = r#"{"type":"language_detect","path":"."}"#;
    match Agent::handle_oneshot(lang_request) {
        Ok(_response) => {}
        Err(_e) => {}
    }

    Ok(())
}
