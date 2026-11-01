//! A `ui.palette` that cannot be loaded falls back to the builtin default
//! palette, so palette-role colours keep rendering.
#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

mod common;

use common::cli_harness::CliTestEnv;

const OVERLAY: &str = "extends = \"default\"\n[segments.language]\nrust_bg_color = \"orange\"\n";

fn render_language(env: &CliTestEnv) -> common::cli_harness::CliCommandResult {
    let repo = env.root().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "").unwrap();
    let repo_arg = repo.to_str().expect("utf-8 temp path");
    env.run_gpy_agent(&["oneshot", "lang", "--cwd", repo_arg, "--format", "ansi"])
        .expect("run gpy-agent")
}

#[test]
fn oneshot_renders_palette_role_with_unloadable_palette() {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"mine\"\npalette = \"nosuch\"\n").unwrap();
    std::fs::create_dir_all(env.themes_dir()).unwrap();
    std::fs::write(env.themes_dir().join("mine.toml"), OVERLAY).unwrap();

    let result = render_language(&env);
    assert!(result.stdout.contains("48;5;208"), "{result:?}");
    assert!(
        result.stderr.contains("using the builtin default palette"),
        "{result:?}"
    );
}

#[test]
fn oneshot_renders_palette_role_with_broken_user_default_palette() {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"mine\"\npalette = \"default\"\n").unwrap();
    std::fs::create_dir_all(env.themes_dir()).unwrap();
    std::fs::write(env.themes_dir().join("mine.toml"), OVERLAY).unwrap();
    let palettes = env.config_dir().join("palettes");
    std::fs::create_dir_all(&palettes).unwrap();
    std::fs::write(palettes.join("default.toml"), "name = [\n").unwrap();

    let result = render_language(&env);
    assert!(result.stdout.contains("48;5;208"), "{result:?}");
}
