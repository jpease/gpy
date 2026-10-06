//! Pure path resolution for GPY's XDG-style roots (#477).
//!
//! GPY has three independent roots, each with its own precedence chain over
//! a handful of environment variables:
//!
//! - **runtime** (`XDG_RUNTIME_DIR` -> `XDG_CACHE_HOME` -> `HOME/.cache/gpy`
//!   -> `/tmp/gpy`): the socket, PID file, and shell registry. Kept separate
//!   from cache because `XDG_RUNTIME_DIR` is a 0700 tmpfs cleared at logout;
//!   collapsing it onto `~/.cache` would leave stale sockets across reboots.
//! - **cache** (`XDG_CACHE_HOME` -> `HOME/.cache/gpy`): the instant-prompt
//!   cache.
//! - **config** (`XDG_CONFIG_HOME/gpy` -> `HOME/.config/gpy`): user themes,
//!   user plugins, and `config.toml`.
//!
//! Before #477, each root's precedence was re-derived independently at every
//! call site (ten-plus copies for the runtime root alone), so a fix to one
//! copy -- like #475, which found a resolver that silently skipped the
//! `XDG_CACHE_HOME` step -- did not propagate to the others. This module is
//! the single definition of each root's precedence; every call site routes
//! its path *value* through the matching function here while keeping its own
//! I/O, error handling, and suffix logic (directory creation, `AlreadyExists`
//! tolerance, the `#426` `/tmp` symlink hardening, etc.) exactly as before.
//!
//! Each function is pure -- no ambient `std::env` reads -- so the precedence
//! can be table-tested. `std::env::set_var` requires `unsafe` under the 2024
//! edition, and this crate forbids unsafe code (`#![forbid(unsafe_code)]`),
//! so a synthetic environment map passed as parameters is the only way to
//! exercise every branch of the matrix in a unit test; real env-mutating
//! tests live in `gpy-agent/tests/` integration binaries instead, which are
//! compiled as separate crates and are not bound by the lib's `forbid`.
//!
//! ## Windows (#478)
//!
//! Only the **cache** root gets a native-Windows branch, via the [`Os`] enum
//! and [`cache_root_for`]'s third parameter. The **runtime** root does not:
//! IPC (the socket/PID-file consumer of `runtime_root_for`) is Unix-socket
//! based and unimplemented on native Windows (see
//! [`crate::ipc::native_windows_unsupported`]), so a Windows runtime-root
//! branch would resolve a path nothing ever creates. The **config** root
//! is out of #478's scope too (not mentioned in that issue's Scope section).
//!
//! Native-Windows GPY can therefore resolve `%LOCALAPPDATA%\gpy` correctly
//! (this module) and run the CLI standalone, but has **no shell
//! integration** -- there is no Fish/Bash/Zsh process on native Windows in
//! this scoping, only under WSL, which presents as Unix and uses the
//! unchanged [`Os::Unix`] path unconditionally. Install and use GPY's prompt
//! integration under WSL; native Windows is CLI-only until named pipes (a
//! separate, unscoped piece of work) replace the Unix-socket transport.
//!
//! The `Os` value is decided by the *impure* call site (`#[cfg(windows)]` in
//! `cache::instant_prompt::get_gpy_cache_dir`), never inside this module's
//! functions themselves (no `cfg!(windows)` in a function body) -- that is
//! what lets every branch, including the Windows one, be table-tested from
//! this Unix development machine.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The directory `gpy-agent start` ran in, recorded before it forks (#724).
///
/// The daemon `chdir`s to `/` so it never keeps a mount busy, yet relative
/// paths the user supplied (`--socket`, `GPY_AGENT_SOCKET_PATH`,
/// `GPY_CONFIG_PATH`, `GPY_DEBUG_LOG`, `GPY_BUNDLED_PLUGIN_DIR`) must keep
/// resolving against the launch directory. The forked daemon inherits this
/// value; every other command leaves it unset.
pub(crate) static LAUNCH_DIR: OnceLock<PathBuf> = OnceLock::new();

/// `path` made absolute against [`LAUNCH_DIR`].
///
/// Falls back to the current directory when `LAUNCH_DIR` is unset (every
/// non-daemon command). Absolute and empty paths, and any path when no
/// directory is known, are returned unchanged.
#[must_use]
pub fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() || path.as_os_str().is_empty() {
        return path.to_path_buf();
    }
    LAUNCH_DIR
        .get()
        .cloned()
        .or_else(|| std::env::current_dir().ok())
        .map_or_else(|| path.to_path_buf(), |dir| dir.join(path))
}

/// Normalise an `$XDG_*` environment value to the spec's notion of "set".
///
/// The XDG Base Directory specification says an environment variable that is
/// "either not set or empty" must be treated as unset, and that a value which
/// "is not an absolute path" is invalid "and should be ignored". Every
/// resolver in this module runs its `XDG_*` inputs through this function
/// (#626), so `XDG_RUNTIME_DIR=` no longer makes the agent join a *relative*
/// `gpy/gpy.sock` that no shell ever looks for, and a relative
/// `XDG_CACHE_HOME=cache` falls through to `$HOME/.cache` instead of writing
/// a cache next to whatever directory the process happened to start in.
///
/// The Fish/Bash/Zsh resolvers apply the same rule with
/// `test -n "$X"; and string match -q '/*' -- $X` (and the `-n`/`[[ $X == /* ]]`
/// equivalents), which is what makes the four implementations agree under the
/// cross-shell parity harness.
///
/// "Absolute" depends on the [`Os`] whose rules apply (#774). On
/// [`Os::Unix`] (Linux, macOS, WSL) only a leading `/` counts, exactly like the
/// shells' `'/*'` checks: `C:\x`, `C:/x`, `\\server\share` and `~/x` are
/// *relative* paths there, and honouring them would bind sockets and write
/// caches under the process's current directory (the WSL `WSLENV` hazard from
/// #478). On [`Os::Windows`] the spellings `/...`, `\...`, `C:\...`,
/// `C:/...` and UNC all count, so the `XDG_CACHE_HOME` override that #478
/// gives native-Windows users keeps working. The function stays pure:
/// the caller's `os`, not `cfg!(windows)`, decides.
#[must_use]
pub fn xdg_value(value: Option<&str>, os: Os) -> Option<&str> {
    value.filter(|raw| match os {
        Os::Unix => raw.starts_with('/'),
        Os::Windows => is_absolute_windows(raw),
    })
}

/// Whether `value` is absolute under Windows rules (POSIX-style `/` included).
fn is_absolute_windows(value: &str) -> bool {
    if value.starts_with('/') || value.starts_with('\\') {
        return true;
    }
    let mut chars = value.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(drive), Some(':'), Some('/' | '\\')) if drive.is_ascii_alphabetic()
    )
}

/// The current user's home directory, as a string.
///
/// The single source of `HOME` for every resolver in this crate (#626).
/// [`std::env::home_dir`] was un-deprecated in Rust 1.85 once its Unix
/// implementation was fixed: it reads `$HOME`, treats an empty value as
/// unset, and falls back to the passwd database (`getpwuid_r`) -- which is
/// exactly what Fish and Zsh do when the environment does not carry `HOME`.
/// Reading the variable directly instead is what made the agent resolve
/// `/tmp/gpy` (and Bash the filesystem-root `/.cache/gpy`) while the shells
/// resolved the real home, so a shell and the agent used different sockets in
/// any `HOME`-less environment (cron, some systemd units, `env -i`).
///
/// Returns `None` only when there is no home directory at all, which is what
/// keeps `/tmp/gpy` alive as [`runtime_root_for`]'s last resort.
#[must_use]
pub fn home_dir() -> Option<String> {
    std::env::home_dir().map(|path| path.to_string_lossy().into_owned())
}

/// Which OS's precedence rules a pure resolver in this module should apply.
///
/// The *value* is decided by the impure caller -- typically via
/// `#[cfg(windows)]` at the call site, see
/// `cache::instant_prompt::get_gpy_cache_dir` -- and passed in. This type
/// itself carries no platform-conditional compilation, which is exactly what
/// lets every branch be exercised from any host in a table-driven test
/// (#478).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux, macOS, and WSL. WSL presents as a Unix environment, so it uses
    /// this variant even though the host kernel is Windows.
    Unix,
    /// A native Windows compilation target (`#[cfg(windows)]`).
    Windows,
}

/// The runtime root for a given environment, with no I/O and no ambient
/// reads.
///
/// Precedence (matches Fish `__gpy_runtime_root` and the production socket
/// resolver):
/// 1. `$XDG_RUNTIME_DIR/gpy`
/// 2. `$XDG_CACHE_HOME/gpy`
/// 3. `$HOME/.cache/gpy`
/// 4. `/tmp/gpy`
///
/// Moved here from `agent::lifecycle` (where it landed for #475) as the
/// shared definition every runtime-root consumer routes through: the four
/// wrappers in `agent::lifecycle::{mod, start}`, `ipc::server::handle`,
/// `ipc::client`, and `ipc::transport` each call this for the path *value*
/// and keep their own I/O and error handling.
#[must_use]
pub fn runtime_root_for(
    xdg_runtime_dir: Option<&str>,
    xdg_cache_home: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    if let Some(xdg_runtime) = xdg_value(xdg_runtime_dir, Os::Unix) {
        return PathBuf::from(xdg_runtime).join("gpy");
    }
    if let Some(xdg_cache) = xdg_value(xdg_cache_home, Os::Unix) {
        return PathBuf::from(xdg_cache).join("gpy");
    }
    if let Some(home_dir) = home {
        return PathBuf::from(home_dir).join(".cache").join("gpy");
    }
    PathBuf::from("/tmp/gpy")
}

/// The cache root for a given environment, with no I/O and no ambient reads.
///
/// Precedence for [`Os::Unix`] (Linux, macOS, WSL):
/// 1. `$XDG_CACHE_HOME/gpy`
/// 2. `$HOME/.cache/gpy`
///
/// Precedence for [`Os::Windows`] (#478):
/// 1. `$XDG_CACHE_HOME/gpy` -- an explicitly-set override still wins, even on
///    Windows
/// 2. `%LOCALAPPDATA%\gpy` -- Local, not Roaming: a cache and a socket must
///    not sync across machines
///
/// Deliberately has **no** `%USERPROFILE%` fallback for the Windows arm: if
/// `LOCALAPPDATA` is unset the environment is broken enough that
/// `USERPROFILE` likely is too, and a silent fallback there would be worse
/// than the hard, explicit error this returning `None` produces.
///
/// `home` and `local_app_data` are two independent optional inputs (rather
/// than one dual-purpose slot) so the call site can pass `None` for whichever
/// one its `os` branch does not use -- see
/// `cache::instant_prompt::get_gpy_cache_dir` for the impure caller that
/// decides `os` via `#[cfg(windows)]` and reads only the matching variable.
///
/// Returns `None` when neither applicable variable is set, mirroring
/// `cache::instant_prompt`'s historical hard-error behaviour for that case --
/// the pure resolver has no business owning the error message text, so
/// callers construct their own [`crate::Error`] when this returns `None`.
#[must_use]
pub fn cache_root_for(
    xdg_cache_home: Option<&str>,
    home: Option<&str>,
    local_app_data: Option<&str>,
    os: Os,
) -> Option<PathBuf> {
    if let Some(xdg_cache) = xdg_value(xdg_cache_home, os) {
        return Some(PathBuf::from(xdg_cache).join("gpy"));
    }
    match os {
        Os::Unix => home.map(|home_dir| PathBuf::from(home_dir).join(".cache").join("gpy")),
        Os::Windows => local_app_data.map(|lad| PathBuf::from(lad).join("gpy")),
    }
}

/// The config root for a given environment, with no I/O and no ambient
/// reads.
///
/// Unix rules only (#774): a Windows-shaped `XDG_CONFIG_HOME` is relative and
/// ignored; native-Windows config resolution is out of scope.
///
/// Precedence:
/// 1. `$XDG_CONFIG_HOME/gpy`
/// 2. `$HOME/.config/gpy`, falling back to a bare `.config/gpy` relative path
///    when `HOME` is also unset.
///
/// Theme-name resolution (`theme::manager::ThemeManager::resolve_path`, used
/// by both the daemon and CLI oneshot rendering, #572) routes through this
/// function via `ThemeManager::user_theme_path`. It previously had its own
/// hand-rolled, no-`./`-prefix fallback for the no-`XDG_CONFIG_HOME`
/// no-`HOME` case (`config::loader::get_theme_path`, since removed); that
/// divergence was deliberately preserved once (#477) but is gone now that
/// theme resolution unified onto this function.
#[must_use]
pub fn config_root_for(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(xdg_config) = xdg_value(xdg_config_home, Os::Unix) {
        return PathBuf::from(xdg_config).join("gpy");
    }
    let home_dir = home.unwrap_or(".");
    PathBuf::from(home_dir).join(".config").join("gpy")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row: two or three `Option<&str>` env inputs, then the expected
    /// output.
    type RuntimeCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
    );

    /// Regression test for #475, moved here from `agent::lifecycle::runtime_dir_tests` as part
    /// of #477's centralization.
    ///
    /// The runtime root's precedence is the contract every socket-path consumer must honour,
    /// and a second resolver in `commands::utils` used to omit the `XDG_CACHE_HOME` step -- so
    /// with `XDG_CACHE_HOME` set and `XDG_RUNTIME_DIR` unset (the conventional macOS setup,
    /// and what every test fixture uses) the CLI connected to a socket nothing was listening
    /// on, and `gpy config set` silently failed to reload a running agent.
    ///
    /// Expressed against the pure resolver because the environment cannot be
    /// mutated in-process: `std::env::set_var` is unavailable under
    /// `#![forbid(unsafe_code)]`.
    ///
    /// # Panics
    ///
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn runtime_root_precedence() {
        let cases: [RuntimeCase; 12] = [
            (
                Some("/run/user/1000"),
                Some("/cache"),
                Some("/home/u"),
                "/run/user/1000/gpy",
            ),
            // The step the duplicate resolver was missing.
            (None, Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (None, None, Some("/home/u"), "/home/u/.cache/gpy"),
            (None, None, None, "/tmp/gpy"),
            // #626: an empty $XDG_* value is "not set" per the XDG spec. This
            // used to join onto the empty string and return the RELATIVE
            // "gpy", a path no shell ever looked for.
            (Some(""), Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (Some(""), Some(""), Some("/home/u"), "/home/u/.cache/gpy"),
            (Some(""), Some(""), None, "/tmp/gpy"),
            // #626: a relative $XDG_* value is invalid and ignored.
            (Some("run"), Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (
                Some("run"),
                Some("cache"),
                Some("/home/u"),
                "/home/u/.cache/gpy",
            ),
            // #774: Windows-shaped values are relative on Unix and ignored.
            (Some("C:/x"), None, Some("/home/u"), "/home/u/.cache/gpy"),
            (Some(r"C:\x"), None, Some("/home/u"), "/home/u/.cache/gpy"),
            (
                Some(r"\\srv\x"),
                None,
                Some("/home/u"),
                "/home/u/.cache/gpy",
            ),
        ];

        for (xdg_runtime, xdg_cache, home, expected) in cases {
            assert_eq!(
                runtime_root_for(xdg_runtime, xdg_cache, home),
                PathBuf::from(expected),
                "runtime root for (XDG_RUNTIME_DIR={xdg_runtime:?}, XDG_CACHE_HOME={xdg_cache:?}, HOME={home:?})"
            );
        }
    }

    /// One row: two `Option<&str>` env inputs, then the expected output (`None` when neither
    /// is set).
    ///
    /// Unix-only (`Os::Unix` is fixed for every row); the Windows arm has its own table in
    /// `cache_root_precedence_windows`.
    type CacheCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    /// # Panics
    ///
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn cache_root_precedence() {
        let cases: [CacheCase; 9] = [
            (Some("/cache"), Some("/home/u"), Some("/cache/gpy")),
            (None, Some("/home/u"), Some("/home/u/.cache/gpy")),
            (None, None, None),
            // #626: empty and relative XDG_CACHE_HOME both fall through.
            (Some(""), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some("cache"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some(""), None, None),
            // #774: Windows-shaped values are relative on Unix and ignored.
            (Some("C:/x"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some(r"C:\x"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (
                Some(r"\\srv\x"),
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
        ];

        for (xdg_cache, home, expected) in cases {
            assert_eq!(
                cache_root_for(xdg_cache, home, None, Os::Unix),
                expected.map(PathBuf::from),
                "cache root for (XDG_CACHE_HOME={xdg_cache:?}, HOME={home:?}, Os::Unix)"
            );
        }
    }

    /// One row: `XDG_CACHE_HOME`, then `LOCALAPPDATA`, then the expected output.
    ///
    /// `None` when neither is set -- exercised with `Os::Windows` from this Unix test runner,
    /// per #478's acceptance criterion that the Windows env matrix is table-tested without a
    /// Windows host.
    ///
    type WindowsCacheCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    /// # Panics
    ///
    /// Panics if the resolver disagrees with the documented Windows
    /// precedence: an explicitly-set `XDG_CACHE_HOME` still wins over
    /// `%LOCALAPPDATA%`, `LOCALAPPDATA` alone resolves to `%LOCALAPPDATA%\gpy`,
    /// and both absent is a hard error (`None`), never a `%USERPROFILE%`
    /// fallback.
    #[test]
    fn cache_root_precedence_windows() {
        let cases: [WindowsCacheCase; 3] = [
            // XDG_CACHE_HOME set: wins even though LOCALAPPDATA is also set.
            (
                Some("/cache"),
                Some(r"C:\Users\u\AppData\Local"),
                Some("/cache/gpy"),
            ),
            // XDG_CACHE_HOME absent, LOCALAPPDATA set. `PathBuf::join` uses
            // the *host* separator, so on this Unix test runner the joined
            // "gpy" component is appended with `/`, not `\` -- that's a test
            // harness artifact, not a claim about real Windows path
            // rendering (which `PathBuf` would join with `\` when actually
            // compiled and run on Windows).
            (
                None,
                Some(r"C:\Users\u\AppData\Local"),
                Some("C:\\Users\\u\\AppData\\Local/gpy"),
            ),
            // Both absent: hard error, no %USERPROFILE% fallback.
            (None, None, None),
        ];

        for (xdg_cache, local_app_data, expected) in cases {
            assert_eq!(
                cache_root_for(xdg_cache, None, local_app_data, Os::Windows),
                expected.map(PathBuf::from),
                "cache root for (XDG_CACHE_HOME={xdg_cache:?}, LOCALAPPDATA={local_app_data:?}, Os::Windows)"
            );
        }
    }

    /// # Panics
    ///
    /// Panics if the XDG spec's "empty or relative is not set" rule is not
    /// applied on Unix, or if an absolute value is rejected.
    #[test]
    fn xdg_value_unix_keeps_only_slash_rooted_values() {
        for accepted in ["/run/user/1000", "/"] {
            assert_eq!(
                xdg_value(Some(accepted), Os::Unix),
                Some(accepted),
                "{accepted:?} is absolute and must be kept"
            );
        }

        // #774: Windows spellings are relative on Unix.
        for rejected in [
            "",
            "cache",
            "./cache",
            "~/cache",
            "C:relative",
            "1:/x",
            r"C:\Users\u\AppData\Local",
            "c:/users/u",
            r"\server\share",
            r"\x",
            " /lead-space",
        ] {
            assert_eq!(
                xdg_value(Some(rejected), Os::Unix),
                None,
                "{rejected:?} is empty or relative on Unix and must be treated as unset"
            );
        }

        assert_eq!(xdg_value(None, Os::Unix), None);
    }

    /// # Panics
    ///
    /// Panics if a Windows-absolute spelling is rejected on Windows, or a
    /// relative one accepted (#478 behaviour).
    #[test]
    fn xdg_value_windows_keeps_windows_and_posix_absolute_values() {
        for accepted in [
            "/run/user/1000",
            "/",
            r"C:\Users\u\AppData\Local",
            "c:/users/u",
            r"\server\share",
        ] {
            assert_eq!(
                xdg_value(Some(accepted), Os::Windows),
                Some(accepted),
                "{accepted:?} is absolute and must be kept"
            );
        }

        for rejected in ["", "cache", "./cache", "~/cache", "C:relative", "1:/x"] {
            assert_eq!(
                xdg_value(Some(rejected), Os::Windows),
                None,
                "{rejected:?} is empty or relative and must be treated as unset"
            );
        }

        assert_eq!(xdg_value(None, Os::Windows), None);
    }

    /// A home directory is always found on a machine with a user account.
    ///
    /// The wrapper is not table-testable -- it reads the ambient environment
    /// on purpose -- so this asserts only that property, which is what makes
    /// `/tmp/gpy` a last resort rather than the routine answer for a
    /// `HOME`-less environment. The *precedence* that consumes it is covered
    /// by the pure tables above.
    ///
    /// # Panics
    ///
    /// Panics if no home directory can be resolved at all.
    #[test]
    fn home_dir_resolves_on_this_machine() {
        let home = home_dir().expect("a home directory must be resolvable");
        assert!(
            home.starts_with('/'),
            "home directory should be absolute, got {home:?}"
        );
    }

    /// One row: two `Option<&str>` env inputs, then the expected output.
    type ConfigCase = (Option<&'static str>, Option<&'static str>, &'static str);

    /// # Panics
    ///
    /// Panics if the resolver disagrees with the documented precedence.
    #[test]
    fn config_root_precedence() {
        let cases: [ConfigCase; 6] = [
            (Some("/xdgcfg"), Some("/home/u"), "/xdgcfg/gpy"),
            (None, Some("/home/u"), "/home/u/.config/gpy"),
            // No leading `./` guaranteed here. The one caller that historically
            // needed a no-slash string for this branch (config::loader's old,
            // now-removed get_theme_path/theme_path_for, #477) has since
            // unified onto this function (#572), so nothing depends on the
            // no-`./` form anymore.
            (None, None, "./.config/gpy"),
            // #626 flips this row: an empty XDG_CONFIG_HOME used to produce
            // the relative "gpy", which is what made the agent report
            // "/gpy/config.toml" (the filesystem root) as its config
            // candidate. It now falls through to $HOME like every other
            // resolver.
            (Some(""), Some("/home/u"), "/home/u/.config/gpy"),
            (Some(""), None, "./.config/gpy"),
            // Relative values are invalid per the spec and ignored too.
            (Some("cfg"), Some("/home/u"), "/home/u/.config/gpy"),
        ];

        for (xdg_config, home, expected) in cases {
            assert_eq!(
                config_root_for(xdg_config, home),
                PathBuf::from(expected),
                "config root for (XDG_CONFIG_HOME={xdg_config:?}, HOME={home:?})"
            );
        }
    }
}
