//! Theme customization example
//!
//! Demonstrates how to work with GPY themes programmatically:
//! - Loading themes
//! - Switching themes
//! - Exporting theme variables for Fish shell

#![allow(clippy::print_stdout)]
#![allow(clippy::indexing_slicing)]

use gpy_agent::{Result, config::Config, shell::Shell, theme::ThemeManager};

fn main() -> Result<()> {
    // Example 1: Load the default theme
    let theme_manager = ThemeManager::new("default")?;
    println!("✓ Loaded default theme");

    // Example 2: Get current theme configuration
    let theme = theme_manager.get();
    println!("\nCurrent theme settings:");
    println!("  Directory color: {}", theme.segments.directory.text_color);
    println!("  Git text color: {}", theme.segments.git.text_color);
    println!("  Git background color: {}", theme.segments.git.bg_color);

    // Example 3: Export theme variables for Fish shell
    let config = Config::default();
    let fish_vars = theme_manager.export(Shell::Fish, &config);
    println!("\nFish shell theme variables:");
    for line in fish_vars.lines().take(5) {
        println!("  {line}");
    }
    println!("  ... ({} lines total)", fish_vars.lines().count());

    // Example 4: List all available themes
    let available_themes = ThemeManager::list_available_themes();
    println!("\nAvailable themes:");
    for theme_name in &available_themes {
        println!("  - {theme_name}");
    }

    // Example 5: Switch to a different theme
    if available_themes.len() > 1 {
        let second_theme = &available_themes[1];
        println!("\nSwitching to theme: {second_theme}");
        theme_manager.switch_theme(second_theme)?;
        println!("✓ Theme switched successfully");
    }

    Ok(())
}
