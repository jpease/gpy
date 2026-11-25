//! The agent's segments take the colour depth of the terminal `gpy prompt`
//! draws in, not of the one that started the agent (#1265): the agent
//! renders 24-bit colour into its replies and its instant cache, and the
//! prompt fits them to its own `GPY_COLOR_DEPTH`.

use crate::common::git::TestRepo;
use crate::common::wait::wait_until;
use crate::harness::{LANDING, Shell};
use serial_test::serial;
use std::time::Duration;

/// The git pill's background, `#336699`, in 24-bit colour.
const TRUECOLOR_BG: &str = "48;2;51;102;153";
/// The same background as its nearest 256-colour index.
const INDEXED_BG: &str = "48;5;60";

/// The git prompt of a clean repository whose pill is `#336699`, drawn
/// under `GPY_COLOR_DEPTH=prompt_depth` with a live agent started under
/// `GPY_COLOR_DEPTH=agent_depth`: first from the agent's reply (nothing is
/// cached yet), then from the instant cache that reply filled.
fn git_prompts(agent_depth: &str, prompt_depth: &str) -> [String; 2] {
    let shell = Shell::with_theme(
        r#""git""#,
        "hexgit",
        Some("extends = \"default\"\n\n[segments.git]\nclean_bg_color = \"#336699\"\n"),
    );
    let repo = TestRepo::new().with_initial_commit();
    let socket = shell.socket().to_string_lossy().into_owned();
    shell
        .sandbox
        .env
        .run_gpy_with_env(
            &["start"],
            &[
                ("GPY_AGENT_SOCKET_PATH", socket.as_str()),
                ("GPY_COLOR_DEPTH", agent_depth),
            ],
        )
        .expect("spawn gpy start")
        .assert_success("gpy start");
    shell.sandbox.wait_until_responding();

    let depth = [("GPY_COLOR_DEPTH", prompt_depth)];
    let replied = shell.prompt(repo.path(), &depth).stdout;
    let entry = shell.cache_file(repo.path(), "git_first_last", Some("black"));
    let cached = wait_until(LANDING, Duration::from_millis(50), || entry.is_file());
    assert!(cached, "the agent did not cache {}", entry.display());
    let served = shell.prompt(repo.path(), &depth).stdout;
    [replied, served]
}

#[test]
#[serial]
fn an_agent_started_at_256_colours_draws_truecolor_in_a_truecolor_prompt() {
    for (source, prompt) in ["reply", "cache"]
        .iter()
        .zip(git_prompts("256", "truecolor"))
    {
        assert!(
            prompt.contains(TRUECOLOR_BG) && !prompt.contains(";5;"),
            "the git pill from the agent's {source} is not 24-bit: {prompt:?}"
        );
    }
}

#[test]
#[serial]
fn an_agent_started_at_truecolor_draws_256_colours_in_a_256_colour_prompt() {
    for (source, prompt) in ["reply", "cache"]
        .iter()
        .zip(git_prompts("truecolor", "256"))
    {
        assert!(
            prompt.contains(INDEXED_BG) && !prompt.contains(";2;"),
            "the git pill from the agent's {source} is not 256-colour: {prompt:?}"
        );
    }
}
