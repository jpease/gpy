//! Configuration management example
//!
//! Demonstrates how to work with GPY configuration:
//! - Loading configuration from file
//! - Accessing configuration values
//! - Using default configuration

#![allow(clippy::print_stdout)]
#![allow(clippy::use_debug)]
#![allow(clippy::str_to_string)]
#![allow(clippy::unwrap_used)]

use gpy_agent::{
    Result,
    config::{Config, manager::ConfigManager},
};

fn main() -> Result<()> {
    println!("GPY Configuration Management Example\n");

    // Example 1: Load configuration (tries user config, falls back to defaults)
    let config_manager = ConfigManager::new().or_else(|e| {
        println!("⚠ Failed to load config file: {e}");
        println!("  Using default configuration instead\n");
        ConfigManager::with_defaults()
    })?;

    let config = config_manager.get();

    // Example 2: Display Git configuration
    println!("Git Settings:");
    println!("  Enabled: {}", config.git.enabled);
    println!("  Show upstream: {}", config.git.show_upstream);
    println!("  Max branch length: {}", config.git.max_branch_length);
    println!("  Timeout: {}s", config.git.timeout_seconds);

    // Example 3: Display Language detection configuration
    println!("\nLanguage Detection Settings:");
    println!("  Enabled: {}", config.language.enabled);
    println!("  Show versions: {}", config.language.show_versions);
    println!("  Cache TTL: {} hours", config.language.cache_ttl_hours);
    println!("  Display mode: {}", config.language.display);

    // Example 4: Display UI configuration
    println!("\nUI Settings:");
    println!("  Theme: {}", config.ui.theme);
    println!("  Show icons: {}", config.ui.show_icons);
    println!("  Enabled segments: {:?}", config.ui.enabled_segments);
    println!("  Max path length: {}", config.ui.directory.max_length);

    // Example 5: Display agent configuration
    println!("\nAgent Settings:");
    println!("  Enabled: {}", config.agent.enabled);
    println!("  Live updates: {}", config.agent.live_updates);
    println!("  Supervisor enabled: {}", config.agent.supervisor.enabled);
    println!("  Timeout: {}s", config.agent.timeout_seconds);

    // Example 6: Create and use a custom configuration
    println!("\n--- Custom Configuration Example ---");
    let mut custom_config = Config::default();
    custom_config.git.enabled = true;
    custom_config.language.enabled = true;
    custom_config.language.show_versions = false; // Hide version numbers
    custom_config.ui.theme =
        gpy_agent::config::types::ThemeName::new("custom".to_string()).unwrap();

    println!("Custom config created:");
    println!("  Git enabled: {}", custom_config.git.enabled);
    println!(
        "  Show language versions: {}",
        custom_config.language.show_versions
    );
    println!("  Theme: {}", custom_config.ui.theme);

    println!("\n✓ Configuration management example completed");
    Ok(())
}
