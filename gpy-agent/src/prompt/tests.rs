//! [`Render`] units that need no shell, agent or files: the root prompt.

use super::link::Link;
use super::pill::{Depth, Paint};
use super::vars::ShellVars;
use super::{Dialect, PromptArgs, Render};
use crate::config::Config;
use crate::ipc::Response;
use crate::template::Palette;
use crate::theme::ThemeConfig;
use std::collections::HashMap;
use std::path::PathBuf;

/// Draw a prompt with no segments, as root or not, for exit `status`.
fn draw(theme: &ThemeConfig, is_root: bool, status: i32) -> (String, String) {
    let mut config = Config::default();
    config.ui.enabled_segments.clear();
    config.ui.transient_prompt = false;
    let vars = ShellVars::new(HashMap::from([
        ("__icon_root_prompt".to_owned(), "#".to_owned()),
        ("__root_prompt_color".to_owned(), "magenta".to_owned()),
        ("__gpy_add_newline".to_owned(), "0".to_owned()),
    ]));
    let mut render = Render {
        dialect: Dialect::Fish,
        paint: Paint {
            depth: Depth::TrueColor,
            dialect: Dialect::Fish,
        },
        link: Link::new(false, 1),
        config: &config,
        theme,
        palette: Palette::default(),
        vars,
        args: PromptArgs {
            status,
            duration_ms: None,
            pid: 1,
            cmd: String::new(),
        },
        pwd: PathBuf::from("/"),
        canonical: None,
        git_root: None,
        scripts: None,
        oneshot_used: false,
        is_root,
    };
    let character = render.render_response(
        &Response::Character {
            success: status == 0_i32,
        },
        super::pill::Place {
            is_last: true,
            is_first: false,
        },
        "black",
    );
    (render.prompt().prompt, character)
}

/// As root the prompt ends in the theme's root symbol in its root colour
/// (`__icon_root_prompt`, `__root_prompt_color`), never the `character`
/// segment; otherwise the `character` segment is the whole tail.
#[test]
fn root_draws_the_root_symbol_instead_of_the_character() {
    let theme = crate::theme::ThemeManager::builtin("default")
        .expect("the default builtin theme loads")
        .get();
    for status in [0_i32, 1_i32] {
        let (root, character) = draw(&theme, true, status);
        assert!(
            root.ends_with("\u{1b}[35;49m# \u{1b}[0m"),
            "root prompt (exit {status}) should end in a magenta `#`: {root:?}"
        );
        assert!(
            !character.is_empty() && !root.contains(&character),
            "root prompt (exit {status}) should not draw the character segment {character:?}: {root:?}"
        );

        let (user, user_character) = draw(&theme, false, status);
        assert_eq!(
            user, user_character,
            "a non-root prompt (exit {status}) is the character segment alone"
        );
        assert!(!user.contains('#'), "no root symbol for a user: {user:?}");
    }
}
