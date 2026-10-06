//! Differential test suite for `gpy theme import` (the STARSHIP IMPORT TEST
//! SUITE, introduced with #734).
//!
//! Every row is a `starship.toml` snippet plus a scenario for one module. The
//! oracle is the real `starship` binary: the row's module is rendered with
//! `starship module <name>` (config via `STARSHIP_CONFIG`), the same snippet is
//! imported with `gpy theme import` and activated exactly as a user would
//! (`theme use --force`, `palette use`), and the matching segment is rendered
//! with `gpy-agent oneshot <segment> --format ansi`. Both renderings are then
//! compared after the normalization below.
//!
//! # Adding a row
//!
//! One line in the [`parity_rows!`] table at the bottom of the file:
//!
//! ```text
//! name_of_the_row: Scenario::Duration(5_000), "[cmd_duration]\nmin_time = 500\n";
//! ```
//!
//! The name becomes the test name, so a failing row is reported (and can be
//! run) on its own. A new kind of module needs a [`Scenario`] variant: its
//! starship module name, the starship arguments, the agent arguments.
//!
//! # Normalization
//!
//! Raw bytes are not compared: Starship and GPY legitimately encode the same
//! look differently (`1;36` vs `0;1;36`, `37;44` vs `0;37;44`,
//! trailing newlines, ...). Each output is run through the terminal model in
//! `common/sgr.rs` (the one the encoder property tests use), which yields every
//! drawn character with the style it carries, and then:
//!
//! 1. newlines and trailing whitespace are dropped (a module's trailing space
//!    is a separator, and only one side prints a final newline);
//! 2. whitespace keeps only the style that can be seen on blank cells (the
//!    background, inverse, underline, strikethrough), so a foreground or bold
//!    on a space cannot cause a difference.
//!
//! What remains must be identical: same visible text, same style on every
//! character.
//!
//! # Environment
//!
//! `starship` must be on `PATH`. When it is missing the test skips visibly and
//! fails under `CI` (the #650 skip contract, `common/skip.rs`). Both sides run
//! with a scrubbed environment: `HOME` is the throwaway test root, the
//! fixture directory lives in it, and `TERM` is fixed.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::use_debug)]
// Lints on the shared SGR interpreter (`common/sgr.rs`) this crate includes.
#![allow(clippy::many_single_char_names)]
#![allow(clippy::missing_errors_doc)]
#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "common/cli_harness.rs"]
mod cli_harness;
#[path = "common/sgr.rs"]
mod sgr;
#[path = "common/skip.rs"]
mod skip;

use cli_harness::CliTestEnv;
use sgr::{INVERSE, Look, STRIKE, UNDERLINE, interpret};

/// Drawn characters, each with the style it carries.
type Cells = Vec<(char, Look)>;

/// Name the imported theme and palette are installed under.
const IMPORT_NAME: &str = "parity";

/// What is rendered for a row.
#[derive(Debug, Clone, Copy)]
enum Scenario {
    /// The `directory` module, in this path relative to `$HOME`.
    Directory(&'static str),
    /// The `cmd_duration` module after a command that took this many ms.
    Duration(u64),
    /// The `hostname` module, on this machine's hostname.
    Hostname,
    /// The `character` module after a command that exited with this status.
    Character(i32),
    /// The prompt's layout: whether a blank line precedes it and whether it
    /// breaks onto a second line. Starship decides this in `starship prompt`;
    /// GPY's shells decide it from the theme's `add_newline` / `two_line`, so
    /// the imported theme's exported `__gpy_*` flags are compared instead.
    Layout,
    /// The `git` segment, which Starship draws as `$git_branch$git_status`:
    /// both modules are rendered and concatenated, in a repository with one
    /// untracked file. Rows must not use `$symbol` (GPY's branch icon has no
    /// trailing space, Starship's has), so they compare the join only.
    Git,
}

impl Scenario {
    /// The Starship module this scenario renders.
    const fn starship_module(self) -> &'static str {
        match self {
            Self::Directory(_) => "directory",
            Self::Duration(_) => "cmd_duration",
            Self::Hostname => "hostname",
            Self::Character(_) => "character",
            Self::Layout => "prompt",
            Self::Git => "git_branch",
        }
    }

    /// The `gpy-agent oneshot` arguments rendering this scenario as ANSI.
    fn agent_args(self, cwd: &Path, hostname: &str) -> Vec<String> {
        let mut args = vec!["oneshot".to_owned()];
        match self {
            Self::Directory(_) => args.extend([
                "directory".to_owned(),
                "--cwd".to_owned(),
                cwd.to_string_lossy().into_owned(),
            ]),
            Self::Duration(millis) => args.extend([
                "duration".to_owned(),
                "--duration-ms".to_owned(),
                millis.to_string(),
            ]),
            Self::Hostname => args.extend([
                "hostname".to_owned(),
                "--hostname".to_owned(),
                hostname.to_owned(),
            ]),
            Self::Character(status) => args.extend([
                "character".to_owned(),
                "--exit-code".to_owned(),
                status.to_string(),
            ]),
            Self::Layout => {
                return ["theme", "export", "--format", "fish"]
                    .map(str::to_owned)
                    .to_vec();
            }
            Self::Git => args.extend([
                "git".to_owned(),
                "--cwd".to_owned(),
                cwd.to_string_lossy().into_owned(),
            ]),
        }
        args.extend(["--format".to_owned(), "ansi".to_owned()]);
        args
    }
}

/// A scrubbed command: only what rendering needs, with `HOME` at `home`.
fn scrubbed(program: &str, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "xterm-256color");
    command
}

fn starship_is_installed() -> bool {
    Command::new("starship")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// This machine's hostname, which Starship's `hostname` module reads itself.
fn machine_hostname(home: &Path) -> String {
    let output = scrubbed("hostname", home).output().expect("run `hostname`");
    assert!(output.status.success(), "`hostname` failed");
    String::from_utf8(output.stdout)
        .expect("hostname is UTF-8")
        .trim()
        .to_owned()
}

/// Render `scenario` with the real `starship` binary using `config`.
fn render_with_starship(env: &CliTestEnv, config: &Path, scenario: Scenario, cwd: &Path) -> String {
    // The git segment is `$git_branch$git_status`: concatenate both modules.
    if matches!(scenario, Scenario::Git) {
        let mut result = String::new();
        for module in ["git_branch", "git_status"] {
            let mut command = scrubbed("starship", env.root());
            command
                .env("STARSHIP_CONFIG", config)
                .env("STARSHIP_CACHE", env.root().join(".starship-cache"))
                .env("STARSHIP_LOG", "error")
                .env("PWD", cwd)
                .current_dir(cwd)
                .args(["module", module]);
            let output = command.output().expect("run starship");
            assert!(
                output.status.success(),
                "starship module {} failed: {}",
                module,
                String::from_utf8_lossy(&output.stderr)
            );
            result.push_str(&String::from_utf8(output.stdout).expect("starship output is UTF-8"));
        }
        return result;
    }

    let mut command = scrubbed("starship", env.root());
    command
        .env("STARSHIP_CONFIG", config)
        .env("STARSHIP_CACHE", env.root().join(".starship-cache"))
        .env("STARSHIP_LOG", "error")
        .env("PWD", cwd)
        .current_dir(cwd)
        .args(["module", scenario.starship_module()]);
    if let Scenario::Duration(millis) = scenario {
        command.args(["--cmd-duration", &millis.to_string()]);
    }
    if let Scenario::Character(status) = scenario {
        command.args(["--status", &status.to_string()]);
    }
    let output = command.output().expect("run starship");
    assert!(
        output.status.success(),
        "starship module failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("starship output is UTF-8")
}

/// Import `config` into the isolated GPY environment and render `scenario`
/// with the agent, the way a user who imported and activated it would see it.
fn render_with_gpy(env: &CliTestEnv, config: &Path, scenario: Scenario, cwd: &Path) -> String {
    let source = config.to_str().expect("config path is UTF-8");
    for args in [
        vec![
            "theme",
            "import",
            source,
            "--name",
            IMPORT_NAME,
            "--apply-layout",
        ],
        vec!["theme", "use", IMPORT_NAME, "--force"],
        vec!["palette", "use", IMPORT_NAME],
    ] {
        env.run_gpy(&args)
            .expect("run gpy")
            .assert_success(&args.join(" "));
    }
    let owned = scenario.agent_args(cwd, &machine_hostname(env.root()));
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();
    let result = env.run_gpy_agent(&args).expect("run gpy-agent");
    result.assert_success(&args.join(" "));
    result.stdout
}

/// The drawn characters of `output` after the normalization in the module
/// docs.
fn normalized(output: &str) -> Result<Cells, String> {
    let (drawn, _) = interpret(output)?;
    let mut cells: Cells = drawn
        .into_iter()
        .filter(|(ch, _)| *ch != '\n' && *ch != '\r')
        .collect();
    while cells.last().is_some_and(|(ch, _)| ch.is_whitespace()) {
        cells.pop();
    }
    for (ch, look) in &mut cells {
        if ch.is_whitespace() {
            look.fg = None;
            look.attrs &= INVERSE | UNDERLINE | STRIKE;
        }
    }
    Ok(cells)
}

/// `text` runs of equal style, one per line, for a readable failure.
fn describe(cells: &[(char, Look)]) -> String {
    let mut runs: Vec<(String, Look)> = Vec::new();
    for (ch, look) in cells {
        match runs.last_mut() {
            Some((text, last)) if last == look => text.push(*ch),
            _ => runs.push((ch.to_string(), *look)),
        }
    }
    runs.iter()
        .map(|(text, look)| format!("    {text:?} {look:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Create a repository in `cwd` with one commit on `main` and an untracked
/// file, so `git_status` has something to report. Every git call must succeed.
fn setup_git_repo(cwd: &Path) {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--quiet"]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "initial"]);
    fs::write(cwd.join("untracked.txt"), "untracked").expect("create untracked file");
}

/// Run one row: render both sides and require identical normalized output.
fn check_row(name: &str, config: &str, scenario: Scenario) {
    if !starship_is_installed() {
        skip::skip_test("`starship` is not installed, so there is no oracle to compare with");
        return;
    }
    let env = CliTestEnv::new().expect("test env");
    let config_path = env.root().join("starship.toml");
    fs::write(&config_path, config).expect("write starship config");
    let cwd = match scenario {
        Scenario::Directory(relative) => env.root().join(relative),
        Scenario::Duration(_)
        | Scenario::Hostname
        | Scenario::Character(_)
        | Scenario::Layout
        | Scenario::Git => env.root().to_path_buf(),
    };
    fs::create_dir_all(&cwd).expect("create fixture directory");
    if matches!(scenario, Scenario::Git) {
        setup_git_repo(&cwd);
    }
    if matches!(scenario, Scenario::Layout) {
        check_layout_row(name, &env, &config_path, config, &cwd);
        return;
    }
    let starship = render_with_starship(&env, &config_path, scenario, &cwd);
    let gpy = render_with_gpy(&env, &config_path, scenario, &cwd);

    let want = normalized(&starship).expect("interpret starship output");
    let got = normalized(&gpy).expect("interpret gpy output");
    assert!(
        want == got,
        "row `{name}` ({scenario:?}) differs from starship\n\
         config:\n{config}\n\
         starship drew:\n{}\n\
         gpy drew:\n{}\n\
         raw starship: {starship:?}\n\
         raw gpy: {gpy:?}",
        describe(&want),
        describe(&got),
    );
}

/// The layout Starship's full prompt draws: `(blank line before, two lines)`.
fn starship_layout(env: &CliTestEnv, config: &Path, cwd: &Path) -> (bool, bool) {
    let output = scrubbed("starship", env.root())
        .env("STARSHIP_CONFIG", config)
        .env("STARSHIP_CACHE", env.root().join(".starship-cache"))
        .env("STARSHIP_LOG", "error")
        .env("PWD", cwd)
        .current_dir(cwd)
        .arg("prompt")
        .output()
        .expect("run starship prompt");
    assert!(
        output.status.success(),
        "starship prompt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("starship output is UTF-8");
    let (drawn, _) = interpret(&text).expect("interpret starship prompt");
    let drawn_text: String = drawn.into_iter().map(|(ch, _)| ch).collect();
    let blank_before = drawn_text.starts_with('\n');
    let two_line = drawn_text.trim().contains('\n');
    (blank_before, two_line)
}

/// The layout the imported theme gives GPY's shells: the `0`/`1` value
/// `gpy-agent theme export` assigns to `__gpy_<flag>`.
fn gpy_flag(export: &str, flag: &str) -> bool {
    let name = format!("__gpy_{flag}");
    let line = export
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("`{name}` missing from theme export:\n{export}"));
    line.contains('1')
}

/// A layout row: Starship's full prompt against the imported theme's flags.
fn check_layout_row(name: &str, env: &CliTestEnv, config_path: &Path, config: &str, cwd: &Path) {
    let want = starship_layout(env, config_path, cwd);
    let source = config_path.to_str().expect("config path is UTF-8");
    for args in [
        vec![
            "theme",
            "import",
            source,
            "--name",
            IMPORT_NAME,
            "--apply-layout",
        ],
        vec!["theme", "use", IMPORT_NAME, "--force"],
    ] {
        env.run_gpy(&args)
            .expect("run gpy")
            .assert_success(&args.join(" "));
    }
    let owned = Scenario::Layout.agent_args(cwd, "");
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();
    let result = env.run_gpy_agent(&args).expect("run gpy-agent");
    result.assert_success(&args.join(" "));
    let got = (
        gpy_flag(&result.stdout, "add_newline"),
        gpy_flag(&result.stdout, "two_line"),
    );
    assert!(
        want == got,
        "row `{name}` layout differs from starship\n\
         config:\n{config}\n\
         starship (blank line before, two lines): {want:?}\n\
         gpy      (add_newline, two_line):        {got:?}\n\
         theme export:\n{}",
        result.stdout,
    );
}

/// One test per row: `name: scenario, "starship.toml snippet";`.
macro_rules! parity_rows {
    ($($name:ident: $scenario:expr, $config:expr;)*) => {
        $(
            #[test]
            fn $name() {
                check_row(stringify!($name), $config, $scenario);
            }
        )*
    };
}

parity_rows! {
    // #734: a present module table without `style` takes Starship's default.
    directory_without_style: Scenario::Directory("work/proj"), "[directory]\ntruncation_length = 3\n";
    cmd_duration_without_style: Scenario::Duration(5_000), "[cmd_duration]\nmin_time = 500\n";
    hostname_without_style: Scenario::Hostname, "[hostname]\nssh_only = false\n";
    // #735: character symbols with trailing text or several groups.
    character_success_capitalized_bold: Scenario::Character(0), "[character]\nsuccess_symbol = \"[❯](Bold green)\"\nerror_symbol = \"[✗](Bold red)\"\n";
    character_success_trailing_space: Scenario::Character(0), "[character]\nsuccess_symbol = \"[➜](bold green) \"\nerror_symbol = \"[✗](bold red) \"\n";
    character_error_trailing_space: Scenario::Character(1), "[character]\nsuccess_symbol = \"[➜](bold green) \"\nerror_symbol = \"[✗](bold red) \"\n";
    character_success_leading_trailing_space: Scenario::Character(0), "[character]\nsuccess_symbol = \" [➜](bold green)  \"\n";
    character_success_not_bold_trailing_space: Scenario::Character(0), "[character]\nsuccess_symbol = \"[➜](green) \"\nerror_symbol = \"[✗](red) \"\n";
    // #792: `fg:`/`bg:` prefixes and palette aliases in character styles.
    character_success_fg_prefix: Scenario::Character(0), "[character]\nsuccess_symbol = \"[❯](fg:cyan)\"\nerror_symbol = \"[✗](fg:purple)\"\n";
    character_success_bold_fg_prefix: Scenario::Character(0), "[character]\nsuccess_symbol = \"[❯](bold fg:cyan)\"\nerror_symbol = \"[✗](bold fg:purple)\"\n";
    character_error_fg_prefix: Scenario::Character(1), "[character]\nsuccess_symbol = \"[❯](bold fg:cyan)\"\nerror_symbol = \"[✗](bold fg:purple)\"\n";
    character_success_fg_and_bg: Scenario::Character(0), "[character]\nsuccess_symbol = \"[❯](bg:blue fg:white)\"\nerror_symbol = \"[✗](bg:blue fg:red)\"\n";
    character_success_palette_alias: Scenario::Character(0), "palette = \"p\"\n[character]\nsuccess_symbol = \"[❯](bold peach)\"\n[palettes.p]\npeach = \"#fab387\"\n";
    character_error_palette_alias_fg_prefix: Scenario::Character(1), "palette = \"p\"\n[character]\nsuccess_symbol = \"[❯](bold peach)\"\nerror_symbol = \"[✗](bold fg:peach)\"\n[palettes.p]\npeach = \"#fab387\"\n";
    // #738: `add_newline` and `$line_break` drive the prompt's layout.
    layout_default: Scenario::Layout, "[character]\n";
    layout_add_newline_false: Scenario::Layout, "add_newline = false\n";
    layout_format_with_line_break: Scenario::Layout, "format = \"$directory$line_break$character\"\n";
    layout_format_with_line_break_no_newline: Scenario::Layout, "add_newline = false\nformat = \"$directory$git_branch$cmd_duration$line_break$character\"\n";
    layout_format_without_line_break: Scenario::Layout, "format = \"$directory$character\"\n";
    layout_format_without_line_break_no_newline: Scenario::Layout, "add_newline = false\nformat = \"$directory$character\"\n";
    // #795: `$git_branch$git_status` joins with nothing between the two.
    git_default_formats_without_symbol: Scenario::Git, "[git_branch]\nformat = \"on [$branch]($style) \"\n[git_status]\n";
    git_pure_preset_format: Scenario::Git, "[git_branch]\nformat = \"[$branch]($style)\"\nstyle = \"bold purple\"\n[git_status]\nformat = \"[$all_status]($style)\"\nstyle = \"bold red\"\n";
    git_powerline_format: Scenario::Git, "[git_branch]\nstyle = \"bg:#394260\"\nformat = '[[ $branch ](fg:#769ff0 bg:#394260)]($style)'\n[git_status]\nstyle = \"bg:#394260\"\nformat = '[[($all_status$ahead_behind )](fg:#769ff0 bg:#394260)]($style)'\n";
}
