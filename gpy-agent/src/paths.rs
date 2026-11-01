//! Path resolution for GPY's runtime, cache, and config roots and the agent socket. Nothing here creates directories.
//!
//! The `*_for` resolvers take environment values as parameters (no ambient
//! reads) so the precedence can be table-tested without `unsafe` `set_var`;
//! `runtime_dir`, `config_root`, `cache_root`, `SocketPath::resolve`,
//! `home_path`, `xdg_config_home`, `xdg_data_home`, `user_name` and
//! `local_app_data` are the entry points that
//! read the environment, and the only place the crate reads `HOME`, `USER`,
//! `XDG_*` or `LOCALAPPDATA`. Every lookup is `Path`-based, so a non-UTF-8
//! value resolves like any other. The runtime root
//! stays separate from cache because `XDG_RUNTIME_DIR` is a 0700 tmpfs cleared
//! at logout. Native Windows gets a cache branch only: IPC is Unix-socket
//! based, so Windows is CLI-only and shell integration runs under WSL.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directory `gpy-agent start` ran in, recorded before the daemon `chdir`s to
/// `/`, so user-supplied relative paths keep resolving against it.
pub(crate) static LAUNCH_DIR: OnceLock<PathBuf> = OnceLock::new();

/// `path` made absolute against [`LAUNCH_DIR`], else the current directory.
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

/// Bytes in `sockaddr_un.sun_path`, NUL included (hardcoded: crate forbids `unsafe`).
#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )
))]
pub const SUN_PATH_CAPACITY: usize = 104;

/// Bytes in `sockaddr_un.sun_path`, NUL included.
#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
pub const SUN_PATH_CAPACITY: usize = 108;

/// Why `path` cannot fit a `sun_path` of `capacity` bytes, or `None` when it fits.
#[must_use]
pub fn socket_path_problem(path: &Path, capacity: usize) -> Option<String> {
    let len = path.as_os_str().len();
    let max = capacity.saturating_sub(1);
    if len <= max {
        return None;
    }
    Some(format!(
        "socket path '{}' is {len} bytes; this system allows at most {max}\n  \
         Remediation: point XDG_RUNTIME_DIR at a shorter directory, or set \
         GPY_AGENT_SOCKET_PATH to a short path in a private directory \
         (mkdir -m 700 /tmp/gpy-$USER; GPY_AGENT_SOCKET_PATH=/tmp/gpy-$USER/gpy.sock)",
        path.display()
    ))
}

/// Refuse a socket path the kernel would reject (macOS allows 104 bytes,
/// Linux 108) with an actionable message instead of `bind`'s bare error.
///
/// # Errors
/// Returns a config error when `path` is longer than the platform allows.
pub fn check_socket_path_len(path: &Path) -> crate::Result<()> {
    #[cfg(unix)]
    {
        socket_path_problem(path, SUN_PATH_CAPACITY)
            .map_or(Ok(()), |problem| Err(crate::Error::config(problem)))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Treat empty or non-absolute `$XDG_*` values as unset, per the XDG Base
/// Directory spec; the shells apply the same rule. On [`Os::Unix`] only a
/// leading `/` counts (`C:\x` from `WSLENV` is relative there).
#[must_use]
pub fn xdg_path(value: Option<&Path>, os: Os) -> Option<&Path> {
    value.filter(|path| match os {
        Os::Unix => path.as_os_str().as_encoded_bytes().first() == Some(&b'/'),
        Os::Windows => is_absolute_windows(&path.to_string_lossy()),
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

/// The user's home directory. Uses [`std::env::home_dir`] (passwd fallback,
/// like Fish/Zsh) so agent and shells agree when `HOME` is unset; a relative
/// `HOME` counts as none.
#[must_use]
pub fn home_path() -> Option<PathBuf> {
    ambient_home().filter(|path| path.is_absolute())
}

#[cfg(not(test))]
fn ambient_home() -> Option<PathBuf> {
    std::env::home_dir()
}

/// The [`root_var_os`] test seam's stand-in for HOME.
#[cfg(test)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "mirrors the non-test variant's signature, which can return None"
)]
fn ambient_home() -> Option<PathBuf> {
    Some(test_root().join("home"))
}

/// `std::env::var_os` for the root-locating variables. Under `cfg(test)` the
/// XDG variables point into a per-process temp root so lib tests never touch
/// the developer's real config or cache.
#[must_use]
pub fn root_var_os(key: &str) -> Option<std::ffi::OsString> {
    #[cfg(test)]
    {
        let subdir = match key {
            "XDG_CONFIG_HOME" => "config",
            "XDG_CACHE_HOME" => "cache",
            "XDG_RUNTIME_DIR" => "run",
            _ => return None,
        };
        Some(test_root().join(subdir).into_os_string())
    }
    #[cfg(not(test))]
    std::env::var_os(key)
}

/// `%LOCALAPPDATA%`, when set. Only [`Os::Windows`] resolvers use it: WSL may
/// inherit a `C:\...` value that means nothing there.
#[must_use]
pub fn local_app_data() -> Option<PathBuf> {
    root_var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// The login name from `$USER`, else `$USERNAME` (Windows); non-UTF-8 bytes
/// are replaced, as the name is only displayed.
#[must_use]
pub fn user_name() -> Option<String> {
    std::env::var_os("USER")
        .or_else(|| std::env::var_os("USERNAME"))
        .map(|name| name.to_string_lossy().into_owned())
}

/// Per-process stand-in for HOME and the XDG roots, emptied on first use.
/// Lives at `$TMPDIR/gpy-lib-test/<pid>`; creating it also removes the
/// siblings of processes that have exited, so runs do not accumulate roots.
#[cfg(test)]
static TEST_ROOT: std::sync::LazyLock<PathBuf> = std::sync::LazyLock::new(|| {
    let parent = std::env::temp_dir().join("gpy-lib-test");
    let pid = std::process::id();
    if let Ok(entries) = std::fs::read_dir(&parent) {
        for entry in entries.flatten() {
            let stale = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
                .is_some_and(|other| {
                    other != pid && !crate::ipc::ClientDirectory::is_client_alive(other)
                });
            if stale {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let root = parent.join(pid.to_string());
    let _ = std::fs::remove_dir_all(&root);
    root
});

#[cfg(test)]
fn test_root() -> &'static Path {
    &TEST_ROOT
}

/// Which OS's rules a pure resolver applies; chosen by the impure caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux, macOS, and WSL.
    Unix,
    /// A native Windows compilation target (`#[cfg(windows)]`).
    Windows,
}

impl Os {
    /// The OS this binary was compiled for.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// The runtime root: `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy`,
/// `$HOME/.cache/gpy`, then `/tmp/gpy`. Matches Fish `__gpy_runtime_root`.
#[must_use]
pub fn runtime_root_for(
    xdg_runtime_dir: Option<&Path>,
    xdg_cache_home: Option<&Path>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(xdg_runtime) = xdg_path(xdg_runtime_dir, Os::Unix) {
        return xdg_runtime.join("gpy");
    }
    if let Some(xdg_cache) = xdg_path(xdg_cache_home, Os::Unix) {
        return xdg_cache.join("gpy");
    }
    if let Some(home_dir) = xdg_path(home, Os::Unix) {
        return home_dir.join(".cache").join("gpy");
    }
    PathBuf::from("/tmp/gpy")
}

/// Runtime directory for agent files: `$XDG_RUNTIME_DIR/gpy`, then
/// `$XDG_CACHE_HOME/gpy`, `$HOME/.cache/gpy`, `/tmp/gpy` (matches the shells).
/// Never creates it; `gpy-agent start` does (`lifecycle::ensure_runtime_dir`).
#[must_use]
pub fn runtime_dir() -> PathBuf {
    runtime_root_for(
        xdg_root("XDG_RUNTIME_DIR").as_deref(),
        xdg_root("XDG_CACHE_HOME").as_deref(),
        home_path().as_deref(),
    )
}

/// Environment variable that points the agent socket somewhere else.
const SOCKET_PATH_ENV: &str = "GPY_AGENT_SOCKET_PATH";

/// The agent socket a process acts on, resolved once from the `--socket`
/// flag, then `GPY_AGENT_SOCKET_PATH`, then `<runtime_dir>/gpy.sock`.
///
/// Carried as a value (never read from process-global state) so the forked
/// daemon binds the path `start` resolved. Read the file path with
/// [`Self::path`]; `&SocketPath` also satisfies `AsRef<Path>` for `connect`
/// and friends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPath {
    path: PathBuf,
    overridden: bool,
}

impl AsRef<Path> for SocketPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// The socket for these inputs: `flag`, else a non-empty `env` value, else
/// `<runtime_dir>/gpy.sock`. Pure, so the precedence is table-tested without
/// touching the process environment; [`SocketPath::resolve`] feeds it the
/// real ones.
#[must_use]
pub fn socket_path_for(flag: Option<&Path>, env: Option<&OsStr>, runtime_dir: &Path) -> SocketPath {
    if let Some(given) = flag {
        return SocketPath {
            path: given.to_path_buf(),
            overridden: true,
        };
    }
    if let Some(value) = env.filter(|value| !value.is_empty()) {
        return SocketPath {
            path: PathBuf::from(value),
            overridden: true,
        };
    }
    SocketPath {
        path: runtime_dir.join("gpy.sock"),
        overridden: false,
    }
}

impl SocketPath {
    /// Resolve the socket from `flag`, `GPY_AGENT_SOCKET_PATH` and the runtime
    /// directory, made absolute against the launch directory so a daemon that
    /// runs in `/` binds the path the user meant.
    ///
    /// # Errors
    /// Returns an error if the path exceeds the platform's Unix socket path limit.
    pub fn resolve(flag: Option<&Path>) -> crate::Result<Self> {
        let socket = Self::resolve_unchecked(flag);
        check_socket_path_len(&socket.path)?;
        Ok(socket)
    }

    /// [`Self::resolve`] without the length check, for diagnostics that must
    /// print a path `resolve` rejects.
    #[must_use]
    pub fn resolve_unchecked(flag: Option<&Path>) -> Self {
        let env = std::env::var_os(SOCKET_PATH_ENV);
        let mut socket = socket_path_for(flag, env.as_deref(), &runtime_dir());
        socket.path = absolutize(&socket.path);
        socket
    }

    /// The socket's file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The socket's file path, by value.
    #[must_use]
    pub fn into_path(self) -> PathBuf {
        self.path
    }

    /// Whether the path came from `--socket` or `GPY_AGENT_SOCKET_PATH` rather
    /// than the default runtime directory.
    #[must_use]
    pub const fn is_override(&self) -> bool {
        self.overridden
    }
}

/// The cache root: `$XDG_CACHE_HOME/gpy`, then `$HOME/.cache/gpy` (Unix) or
/// `%LOCALAPPDATA%\gpy` (Windows; Local because caches must not roam).
/// Returns `None` when neither applies; callers build the error.
#[must_use]
pub fn cache_root_for(
    xdg_cache_home: Option<&Path>,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    os: Os,
) -> Option<PathBuf> {
    if let Some(xdg_cache) = xdg_path(xdg_cache_home, os) {
        return Some(xdg_cache.join("gpy"));
    }
    match os {
        Os::Unix => xdg_path(home, Os::Unix).map(|home_dir| home_dir.join(".cache").join("gpy")),
        Os::Windows => local_app_data.map(|lad| lad.join("gpy")),
    }
}

/// [`cache_root_for`] for this process's environment and OS.
#[must_use]
pub fn cache_root() -> Option<PathBuf> {
    cache_root_for(
        xdg_root("XDG_CACHE_HOME").as_deref(),
        home_path().as_deref(),
        local_app_data().as_deref(),
        Os::host(),
    )
}

/// The config root: `$XDG_CONFIG_HOME/gpy` (absolute per `os`), then
/// `$HOME/.config/gpy`, else relative `.config/gpy`.
#[must_use]
pub fn config_root_for(xdg_config_home: Option<&Path>, home: Option<&Path>, os: Os) -> PathBuf {
    if let Some(xdg_config) = xdg_path(xdg_config_home, os) {
        return xdg_config.join("gpy");
    }
    let home_dir = xdg_path(home, os).unwrap_or_else(|| Path::new("."));
    home_dir.join(".config").join("gpy")
}

/// [`config_root_for`] for this process's environment and OS.
#[must_use]
pub fn config_root() -> PathBuf {
    config_root_for(
        xdg_config_home().as_deref(),
        home_path().as_deref(),
        Os::host(),
    )
}

/// `$XDG_CONFIG_HOME` as set, before the absoluteness rule of [`xdg_path`].
#[must_use]
pub fn xdg_config_home() -> Option<PathBuf> {
    xdg_root("XDG_CONFIG_HOME")
}

/// `$XDG_DATA_HOME` as set, before the absoluteness rule of [`xdg_path`].
#[must_use]
pub fn xdg_data_home() -> Option<PathBuf> {
    xdg_root("XDG_DATA_HOME")
}

fn xdg_root(key: &str) -> Option<PathBuf> {
    root_var_os(key).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    type RuntimeCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        &'static str,
    );

    /// `XDG_CACHE_HOME` must win when `XDG_RUNTIME_DIR` is unset (macOS default).
    #[test]
    fn runtime_root_precedence() {
        let cases: [RuntimeCase; 14] = [
            (
                Some("/run/user/1000"),
                Some("/cache"),
                Some("/home/u"),
                "/run/user/1000/gpy",
            ),
            (None, Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (None, None, Some("/home/u"), "/home/u/.cache/gpy"),
            (None, None, None, "/tmp/gpy"),
            // Empty or relative $XDG_* values are unset.
            (Some(""), Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (Some(""), Some(""), Some("/home/u"), "/home/u/.cache/gpy"),
            (Some(""), Some(""), None, "/tmp/gpy"),
            (Some("run"), Some("/cache"), Some("/home/u"), "/cache/gpy"),
            (
                Some("run"),
                Some("cache"),
                Some("/home/u"),
                "/home/u/.cache/gpy",
            ),
            // Windows-shaped values are relative on Unix and ignored.
            (Some("C:/x"), None, Some("/home/u"), "/home/u/.cache/gpy"),
            (Some(r"C:\x"), None, Some("/home/u"), "/home/u/.cache/gpy"),
            (
                Some(r"\\srv\x"),
                None,
                Some("/home/u"),
                "/home/u/.cache/gpy",
            ),
            // An empty or relative HOME is unset, as in the shells' __gpy_home.
            (None, None, Some("relhome"), "/tmp/gpy"),
            (None, None, Some(""), "/tmp/gpy"),
        ];

        for (xdg_runtime, xdg_cache, home, expected) in cases {
            assert_eq!(
                runtime_root_for(
                    xdg_runtime.map(Path::new),
                    xdg_cache.map(Path::new),
                    home.map(Path::new),
                ),
                PathBuf::from(expected),
                "runtime root for (XDG_RUNTIME_DIR={xdg_runtime:?}, XDG_CACHE_HOME={xdg_cache:?}, HOME={home:?})"
            );
        }
    }

    type CacheCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    #[test]
    fn cache_root_precedence() {
        let cases: [CacheCase; 11] = [
            (Some("/cache"), Some("/home/u"), Some("/cache/gpy")),
            (None, Some("/home/u"), Some("/home/u/.cache/gpy")),
            (None, None, None),
            (Some(""), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some("cache"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some(""), None, None),
            (Some("C:/x"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (Some(r"C:\x"), Some("/home/u"), Some("/home/u/.cache/gpy")),
            (
                Some(r"\\srv\x"),
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            // Empty or relative HOME is unset, as in the shells' __gpy_home.
            (None, Some("relhome"), None),
            (None, Some(""), None),
        ];

        for (xdg_cache, home, expected) in cases {
            assert_eq!(
                cache_root_for(
                    xdg_cache.map(Path::new),
                    home.map(Path::new),
                    None,
                    Os::Unix
                ),
                expected.map(PathBuf::from),
                "cache root for (XDG_CACHE_HOME={xdg_cache:?}, HOME={home:?}, Os::Unix)"
            );
        }
    }

    /// `Os::Windows` rows, testable from a Unix host.
    type WindowsCacheCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    #[test]
    fn cache_root_precedence_windows() {
        let cases: [WindowsCacheCase; 3] = [
            (
                Some("/cache"),
                Some(r"C:\Users\u\AppData\Local"),
                Some("/cache/gpy"),
            ),
            // `join` uses the host separator, hence `/gpy` on this runner.
            (
                None,
                Some(r"C:\Users\u\AppData\Local"),
                Some("C:\\Users\\u\\AppData\\Local/gpy"),
            ),
            (None, None, None),
        ];

        for (xdg_cache, local_app_data, expected) in cases {
            assert_eq!(
                cache_root_for(
                    xdg_cache.map(Path::new),
                    None,
                    local_app_data.map(Path::new),
                    Os::Windows,
                ),
                expected.map(PathBuf::from),
                "cache root for (XDG_CACHE_HOME={xdg_cache:?}, LOCALAPPDATA={local_app_data:?}, Os::Windows)"
            );
        }
    }

    #[test]
    fn xdg_path_unix_keeps_only_slash_rooted_values() {
        for accepted in ["/run/user/1000", "/"] {
            assert_eq!(
                xdg_path(Some(Path::new(accepted)), Os::Unix),
                Some(Path::new(accepted)),
                "{accepted:?} is absolute and must be kept"
            );
        }

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
                xdg_path(Some(Path::new(rejected)), Os::Unix),
                None,
                "{rejected:?} is empty or relative on Unix and must be treated as unset"
            );
        }

        assert_eq!(xdg_path(None, Os::Unix), None);
    }

    #[test]
    fn xdg_path_windows_keeps_windows_and_posix_absolute_values() {
        for accepted in [
            "/run/user/1000",
            "/",
            r"C:\Users\u\AppData\Local",
            "c:/users/u",
            r"\server\share",
        ] {
            assert_eq!(
                xdg_path(Some(Path::new(accepted)), Os::Windows),
                Some(Path::new(accepted)),
                "{accepted:?} is absolute and must be kept"
            );
        }

        for rejected in ["", "cache", "./cache", "~/cache", "C:relative", "1:/x"] {
            assert_eq!(
                xdg_path(Some(Path::new(rejected)), Os::Windows),
                None,
                "{rejected:?} is empty or relative and must be treated as unset"
            );
        }

        assert_eq!(xdg_path(None, Os::Windows), None);
    }

    /// A root that is not UTF-8 resolves byte for byte in every resolver.
    #[cfg(unix)]
    #[test]
    fn resolvers_keep_non_utf8_roots_intact() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let odd = Path::new(OsStr::from_bytes(b"/root-\xff"));
        assert_eq!(runtime_root_for(Some(odd), None, None), odd.join("gpy"));
        assert_eq!(
            cache_root_for(Some(odd), None, None, Os::Unix),
            Some(odd.join("gpy"))
        );
        assert_eq!(
            cache_root_for(None, Some(odd), None, Os::Unix),
            Some(odd.join(".cache").join("gpy"))
        );
        assert_eq!(config_root_for(Some(odd), None, Os::Unix), odd.join("gpy"));
        assert_eq!(
            config_root_for(None, Some(odd), Os::Unix),
            odd.join(".config").join("gpy")
        );
    }

    /// Ambient wrapper; precedence is covered by the pure tables.
    #[test]
    fn home_path_resolves_on_this_machine() {
        let home = home_path().expect("a home directory must be resolvable");
        assert!(
            home.is_absolute(),
            "home directory should be absolute, got {home:?}"
        );
    }

    type ConfigCase = (Option<&'static str>, Option<&'static str>, &'static str);

    #[test]
    fn config_root_precedence() {
        let cases: [ConfigCase; 8] = [
            (Some("/xdgcfg"), Some("/home/u"), "/xdgcfg/gpy"),
            (None, Some("/home/u"), "/home/u/.config/gpy"),
            (None, None, "./.config/gpy"),
            // Empty or relative values are unset.
            (Some(""), Some("/home/u"), "/home/u/.config/gpy"),
            (Some(""), None, "./.config/gpy"),
            (Some("cfg"), Some("/home/u"), "/home/u/.config/gpy"),
            (None, Some("relhome"), "./.config/gpy"),
            (None, Some(""), "./.config/gpy"),
        ];

        for (xdg_config, home, expected) in cases {
            assert_eq!(
                config_root_for(xdg_config.map(Path::new), home.map(Path::new), Os::Unix),
                PathBuf::from(expected),
                "config root for (XDG_CONFIG_HOME={xdg_config:?}, HOME={home:?})"
            );
        }
    }

    /// Windows-absolute `XDG_CONFIG_HOME` is honoured only under `Os::Windows`.
    #[test]
    fn config_root_windows_xdg_override() {
        let xdg = Some(Path::new(r"C:\Users\u\cfg"));
        let home = Some(Path::new("/home/u"));
        assert_eq!(
            config_root_for(xdg, home, Os::Windows),
            PathBuf::from(r"C:\Users\u\cfg").join("gpy"),
        );
        assert_eq!(
            config_root_for(xdg, home, Os::Unix),
            PathBuf::from("/home/u/.config/gpy"),
        );
    }

    /// The NUL takes the last byte, so the boundary is `capacity - 1`.
    #[test]
    fn socket_path_problem_boundary_and_message() {
        for capacity in [104_usize, 108_usize] {
            let fits = format!("/{}", "a".repeat(capacity - 2));
            assert_eq!(fits.len(), capacity - 1);
            assert_eq!(
                socket_path_problem(Path::new(&fits), capacity),
                None,
                "{} bytes fit a {capacity}-byte sun_path",
                fits.len()
            );

            let too_long = format!("/{}", "a".repeat(capacity - 1));
            let problem = socket_path_problem(Path::new(&too_long), capacity)
                .expect("one byte too many is refused");
            for needle in [
                format!("is {capacity} bytes"),
                format!("at most {}", capacity - 1),
                "GPY_AGENT_SOCKET_PATH".to_owned(),
                "XDG_RUNTIME_DIR".to_owned(),
            ] {
                assert!(problem.contains(&needle), "`{needle}` in: {problem}");
            }
            let mut lines = problem.lines();
            assert!(
                lines.next().is_some_and(|first| first.contains("bytes")),
                "the first line names the length: {problem}"
            );
            assert!(
                lines
                    .next()
                    .is_some_and(|second| second.starts_with("  Remediation:")),
                "the second line is the remediation: {problem}"
            );
        }
    }

    /// The constant matches what std/the kernel accept.
    #[cfg(unix)]
    #[test]
    fn sun_path_capacity_matches_what_the_kernel_accepts() {
        let longest = format!("/{}", "a".repeat(SUN_PATH_CAPACITY - 2));
        assert!(
            std::os::unix::net::SocketAddr::from_pathname(&longest).is_ok(),
            "{} bytes should fit sun_path on this platform",
            longest.len()
        );
        assert_eq!(check_socket_path_len(Path::new(&longest)).ok(), Some(()));

        let too_long = format!("{longest}a");
        assert!(
            std::os::unix::net::SocketAddr::from_pathname(&too_long).is_err(),
            "{} bytes should not fit sun_path on this platform",
            too_long.len()
        );
        let error = check_socket_path_len(Path::new(&too_long))
            .expect_err("one byte over the limit is refused")
            .to_string();
        assert!(
            error.contains(&format!("is {} bytes", too_long.len())),
            "{error}"
        );
    }

    fn socket_at(path: &str, overridden: bool) -> SocketPath {
        SocketPath {
            path: PathBuf::from(path),
            overridden,
        }
    }

    /// `--socket` beats `GPY_AGENT_SOCKET_PATH`, which beats the runtime directory.
    #[test]
    fn socket_path_precedence_is_flag_then_env_then_default() {
        let runtime = Path::new("/run/gpy");
        let flag_path = Path::new("/flag.sock");
        let env_value = OsStr::new("/env.sock");
        let cases: [(Option<&Path>, Option<&OsStr>, SocketPath); 4] = [
            (
                Some(flag_path),
                Some(env_value),
                socket_at("/flag.sock", true),
            ),
            (Some(flag_path), None, socket_at("/flag.sock", true)),
            (None, Some(env_value), socket_at("/env.sock", true)),
            (None, None, socket_at("/run/gpy/gpy.sock", false)),
        ];
        for (flag, env, expected) in cases {
            assert_eq!(
                socket_path_for(flag, env, runtime),
                expected,
                "flag {flag:?}, env {env:?}"
            );
        }
    }

    /// An empty `GPY_AGENT_SOCKET_PATH` falls through to the default, as in the shells.
    #[test]
    fn empty_socket_env_counts_as_unset() {
        let resolved = socket_path_for(None, Some(OsStr::new("")), Path::new("/run/gpy"));
        assert_eq!(resolved, socket_at("/run/gpy/gpy.sock", false));
        assert!(!resolved.is_override());
    }
}
