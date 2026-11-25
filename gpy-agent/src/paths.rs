//! Path resolution for GPY's runtime, cache, and config roots and the agent socket. Nothing here creates directories.
//!
//! The `*_for` resolvers take environment values as parameters (no ambient
//! reads) so the precedence can be table-tested without `unsafe` `set_var`.
//! This module is the only reader of `HOME`, `USER`, `XDG_*` and
//! `LOCALAPPDATA`. Every lookup is `Path`-based, so a non-UTF-8 value resolves
//! like any other. The runtime root stays separate from cache because
//! `XDG_RUNTIME_DIR` is a 0700 tmpfs cleared at logout. Native Windows gets a
//! cache branch only: IPC is Unix-socket based, so Windows is CLI-only and
//! shell integration runs under WSL.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// `path` made absolute against the current directory.
#[must_use]
pub fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() || path.is_empty() {
        return path.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |dir| dir.join(path))
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

/// The runtime root: `$XDG_RUNTIME_DIR/gpy`, then the cache root
/// ([`cache_root_for`]: `$XDG_CACHE_HOME/gpy`, then `$HOME/.cache/gpy` on Unix
/// or `%LOCALAPPDATA%\gpy` on Windows), each `XDG_*` value absolute per `os`.
/// `None` when none is usable (no `/tmp` fallback: a shared directory would
/// let another user squat the socket). The shells' `__gpy_runtime_root`
/// copies (Unix rules) are pinned by `xdg_absoluteness_shared_vectors` and
/// `path_parity`, and disable GPY for the session on `None`.
#[must_use]
pub fn runtime_root_for(
    xdg_runtime_dir: Option<&Path>,
    xdg_cache_home: Option<&Path>,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    os: Os,
) -> Option<PathBuf> {
    xdg_path(xdg_runtime_dir, os)
        .map(|root| root.join("gpy"))
        .or_else(|| cache_root_for(xdg_cache_home, home, local_app_data, os))
}

/// The one-line reason [`runtime_dir`] fails, naming the variables to set.
pub const NO_RUNTIME_DIR: &str = if cfg!(windows) {
    "cannot locate a runtime directory: set XDG_RUNTIME_DIR, XDG_CACHE_HOME or LOCALAPPDATA to an absolute path"
} else {
    "cannot locate a runtime directory: set XDG_RUNTIME_DIR, XDG_CACHE_HOME or HOME to an absolute path"
};

/// The one-line reason [`config_root`] fails, naming the variables to set.
pub const NO_CONFIG_DIR: &str =
    "cannot locate a config directory: set XDG_CONFIG_HOME or HOME to an absolute path";

/// Why there are no bundled plugins without `GPY_BUNDLED_PLUGIN_DIR`: no data
/// root ([`data_root`]), naming the variables to set.
pub const NO_DATA_DIR: &str =
    "cannot locate a data directory: set XDG_DATA_HOME or HOME to an absolute path";

/// [`runtime_root_for`] for this process's environment: where the agent keeps
/// its socket and the shells' PID files.
/// Never creates it; `gpy-agent start` does (`lifecycle::ensure_runtime_dir`).
///
/// # Errors
/// Returns [`NO_RUNTIME_DIR`] when no absolute `XDG_RUNTIME_DIR`,
/// `XDG_CACHE_HOME` or home directory (`%LOCALAPPDATA%` on Windows) is
/// available.
pub fn runtime_dir() -> crate::Result<PathBuf> {
    runtime_root_for(
        xdg_root("XDG_RUNTIME_DIR").as_deref(),
        xdg_root("XDG_CACHE_HOME").as_deref(),
        home_path().as_deref(),
        local_app_data().as_deref(),
        Os::host(),
    )
    .ok_or_else(|| crate::Error::config(NO_RUNTIME_DIR))
}

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
/// `<runtime_dir>/gpy.sock`; `None` when only the default applies and there is
/// no runtime directory. Pure, so the precedence is table-tested without
/// touching the process environment; [`SocketPath::resolve`] feeds it the
/// real ones.
#[must_use]
pub fn socket_path_for(
    flag: Option<&Path>,
    env: Option<&OsStr>,
    runtime_dir: Option<&Path>,
) -> Option<SocketPath> {
    if let Some(given) = flag {
        return Some(SocketPath {
            path: given.to_path_buf(),
            overridden: true,
        });
    }
    if let Some(value) = env.filter(|value| !value.is_empty()) {
        return Some(SocketPath {
            path: PathBuf::from(value),
            overridden: true,
        });
    }
    runtime_dir.map(|dir| SocketPath {
        path: dir.join("gpy.sock"),
        overridden: false,
    })
}

impl SocketPath {
    /// Resolve the socket from `flag`, `GPY_AGENT_SOCKET_PATH` and the runtime
    /// directory, made absolute against the launch directory so a daemon that
    /// runs in `/` binds the path the user meant.
    ///
    /// # Errors
    /// Returns an error if no runtime directory exists for the default socket
    /// (see [`runtime_dir`]) or the path exceeds the platform's Unix socket
    /// path limit.
    pub fn resolve(flag: Option<&Path>) -> crate::Result<Self> {
        let socket = Self::resolve_unchecked(flag)?;
        check_socket_path_len(&socket.path)?;
        Ok(socket)
    }

    /// [`Self::resolve`] without the length check, for diagnostics that must
    /// print a path `resolve` rejects.
    ///
    /// # Errors
    /// Returns [`NO_RUNTIME_DIR`] when neither `flag` nor
    /// `GPY_AGENT_SOCKET_PATH` is set and there is no runtime directory.
    pub fn resolve_unchecked(flag: Option<&Path>) -> crate::Result<Self> {
        let env = crate::env_config::AGENT_SOCKET_PATH.path();
        let runtime = runtime_dir().ok();
        let mut socket = socket_path_for(
            flag,
            env.as_deref().map(Path::as_os_str),
            runtime.as_deref(),
        )
        .ok_or_else(|| crate::Error::config(NO_RUNTIME_DIR))?;
        socket.path = absolutize(&socket.path);
        Ok(socket)
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

/// GPY base cache directory (`~/.cache/gpy`, XDG, or `%LOCALAPPDATA%\gpy`).
///
/// # Errors
///
/// Returns an error if neither `XDG_CACHE_HOME` nor `HOME`/`LOCALAPPDATA` is set.
pub fn gpy_cache_dir() -> crate::Result<PathBuf> {
    cache_root().ok_or_else(|| {
        crate::Error::config("No cache directory found (XDG_CACHE_HOME or HOME not set)")
    })
}

/// The data root, where `gpy setup` installs the shell trees and bundled
/// plugins: `$XDG_DATA_HOME/gpy` (absolute per `os`), then
/// `$HOME/.local/share/gpy` on Unix or `%LOCALAPPDATA%/gpy` on Windows.
#[must_use]
pub fn data_root_for(
    xdg_data_home: Option<&Path>,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    os: Os,
) -> Option<PathBuf> {
    if let Some(xdg_data) = xdg_path(xdg_data_home, os) {
        return Some(xdg_data.join("gpy"));
    }
    match os {
        Os::Unix => xdg_path(home, Os::Unix)
            .map(|home_dir| home_dir.join(".local").join("share").join("gpy")),
        Os::Windows => local_app_data.map(|lad| lad.join("gpy")),
    }
}

/// [`data_root_for`] for this process's environment and OS.
#[must_use]
pub fn data_root() -> Option<PathBuf> {
    data_root_for(
        xdg_data_home().as_deref(),
        home_path().as_deref(),
        local_app_data().as_deref(),
        Os::host(),
    )
}

/// The config root: `$XDG_CONFIG_HOME/gpy` (absolute per `os`), then
/// `$HOME/.config/gpy`; `None` when neither is usable (never a relative
/// `.config/gpy` under the current directory).
#[must_use]
pub fn config_root_for(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
    os: Os,
) -> Option<PathBuf> {
    xdg_path(xdg_config_home, os)
        .map(|xdg_config| xdg_config.join("gpy"))
        .or_else(|| xdg_path(home, os).map(|home_dir| home_dir.join(".config").join("gpy")))
}

/// [`config_root_for`] for this process's environment and OS: where user
/// themes, palettes and plugins live.
///
/// # Errors
/// Returns [`NO_CONFIG_DIR`] when neither an absolute `XDG_CONFIG_HOME` nor a
/// home directory is available.
pub fn config_root() -> crate::Result<PathBuf> {
    config_root_for(
        xdg_config_home().as_deref(),
        home_path().as_deref(),
        Os::host(),
    )
    .ok_or_else(|| crate::Error::config(NO_CONFIG_DIR))
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
        Option<&'static str>,
    );

    /// `XDG_CACHE_HOME` must win when `XDG_RUNTIME_DIR` is unset (macOS
    /// default); with nothing usable there is no root (never `/tmp/gpy`).
    #[test]
    fn runtime_root_precedence() {
        let cases: [RuntimeCase; 14] = [
            (
                Some("/run/user/1000"),
                Some("/cache"),
                Some("/home/u"),
                Some("/run/user/1000/gpy"),
            ),
            (None, Some("/cache"), Some("/home/u"), Some("/cache/gpy")),
            (None, None, Some("/home/u"), Some("/home/u/.cache/gpy")),
            (None, None, None, None),
            // Empty or relative $XDG_* values are unset.
            (
                Some(""),
                Some("/cache"),
                Some("/home/u"),
                Some("/cache/gpy"),
            ),
            (
                Some(""),
                Some(""),
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            (Some(""), Some(""), None, None),
            (
                Some("run"),
                Some("/cache"),
                Some("/home/u"),
                Some("/cache/gpy"),
            ),
            (
                Some("run"),
                Some("cache"),
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            // Windows-shaped values are relative on Unix and ignored.
            (
                Some("C:/x"),
                None,
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            (
                Some(r"C:\x"),
                None,
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            (
                Some(r"\\srv\x"),
                None,
                Some("/home/u"),
                Some("/home/u/.cache/gpy"),
            ),
            // An empty or relative HOME is unset, as in the shells' __gpy_home.
            (None, None, Some("relhome"), None),
            (Some("run"), Some("cache"), Some(""), None),
        ];

        // A `%LOCALAPPDATA%` inherited through WSL never counts under Unix rules.
        let local_app_data = Some(Path::new(r"C:\Users\u\AppData\Local"));
        for (xdg_runtime, xdg_cache, home, expected) in cases {
            assert_eq!(
                runtime_root_for(
                    xdg_runtime.map(Path::new),
                    xdg_cache.map(Path::new),
                    home.map(Path::new),
                    local_app_data,
                    Os::Unix,
                ),
                expected.map(PathBuf::from),
                "runtime root for (XDG_RUNTIME_DIR={xdg_runtime:?}, XDG_CACHE_HOME={xdg_cache:?}, HOME={home:?})"
            );
        }
    }

    /// Native Windows (CLI only): Windows-absolute `XDG_*` values count, then
    /// `%LOCALAPPDATA%\gpy` as for the cache root; HOME is never consulted.
    #[test]
    fn runtime_root_precedence_windows() {
        type WindowsRuntimeCase = (
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
        );
        let local = r"C:\Users\u\AppData\Local";
        // `join` uses the host separator, hence `/gpy` on a Unix runner.
        let cases: [WindowsRuntimeCase; 7] = [
            (
                Some(r"C:\run"),
                Some(r"C:\cache"),
                Some(local),
                Some(r"C:\run/gpy"),
            ),
            (None, Some(r"C:\cache"), Some(local), Some(r"C:\cache/gpy")),
            (
                None,
                None,
                Some(local),
                Some(r"C:\Users\u\AppData\Local/gpy"),
            ),
            (
                Some("run"),
                Some("cache"),
                Some(local),
                Some(r"C:\Users\u\AppData\Local/gpy"),
            ),
            (
                Some(""),
                Some(""),
                Some(local),
                Some(r"C:\Users\u\AppData\Local/gpy"),
            ),
            (None, Some("/cache"), None, Some("/cache/gpy")),
            (Some("run"), Some("cache"), None, None),
        ];
        for (xdg_runtime, xdg_cache, lad, expected) in cases {
            assert_eq!(
                runtime_root_for(
                    xdg_runtime.map(Path::new),
                    xdg_cache.map(Path::new),
                    Some(Path::new(r"C:\Users\u")),
                    lad.map(Path::new),
                    Os::Windows,
                ),
                expected.map(PathBuf::from),
                "runtime root for (XDG_RUNTIME_DIR={xdg_runtime:?}, XDG_CACHE_HOME={xdg_cache:?}, LOCALAPPDATA={lad:?}, Os::Windows)"
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
    fn data_root_precedence() {
        type DataCase = (
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
        );
        let cases: [DataCase; 8] = [
            (Some("/xdgdata"), Some("/home/u"), Some("/xdgdata/gpy")),
            (None, Some("/home/u"), Some("/home/u/.local/share/gpy")),
            (Some(""), Some("/home/u"), Some("/home/u/.local/share/gpy")),
            (
                Some("data"),
                Some("/home/u"),
                Some("/home/u/.local/share/gpy"),
            ),
            (
                Some("C:/x"),
                Some("/home/u"),
                Some("/home/u/.local/share/gpy"),
            ),
            (Some("/xdgdata"), None, Some("/xdgdata/gpy")),
            (None, Some("relhome"), None),
            (None, None, None),
        ];

        for (xdg_data, home, expected) in cases {
            assert_eq!(
                data_root_for(xdg_data.map(Path::new), home.map(Path::new), None, Os::Unix),
                expected.map(PathBuf::from),
                "data root for (XDG_DATA_HOME={xdg_data:?}, HOME={home:?}, Os::Unix)"
            );
        }
    }

    #[test]
    fn data_root_precedence_windows() {
        let local = Some(Path::new(r"C:\Users\u\AppData\Local"));
        assert_eq!(
            data_root_for(Some(Path::new("/data")), None, local, Os::Windows),
            Some(PathBuf::from("/data/gpy"))
        );
        // `join` uses the host separator, hence `/gpy` on this runner.
        assert_eq!(
            data_root_for(Some(Path::new("data")), None, local, Os::Windows),
            Some(PathBuf::from("C:\\Users\\u\\AppData\\Local/gpy"))
        );
        assert_eq!(data_root_for(None, None, None, Os::Windows), None);
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
        assert_eq!(
            runtime_root_for(Some(odd), None, None, None, Os::Unix),
            Some(odd.join("gpy"))
        );
        assert_eq!(
            cache_root_for(Some(odd), None, None, Os::Unix),
            Some(odd.join("gpy"))
        );
        assert_eq!(
            cache_root_for(None, Some(odd), None, Os::Unix),
            Some(odd.join(".cache").join("gpy"))
        );
        assert_eq!(
            config_root_for(Some(odd), None, Os::Unix),
            Some(odd.join("gpy"))
        );
        assert_eq!(
            config_root_for(None, Some(odd), Os::Unix),
            Some(odd.join(".config").join("gpy"))
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

    type ConfigCase = (
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    /// With nothing usable there is no root (never a relative `./.config/gpy`).
    #[test]
    fn config_root_precedence() {
        let cases: [ConfigCase; 8] = [
            (Some("/xdgcfg"), Some("/home/u"), Some("/xdgcfg/gpy")),
            (None, Some("/home/u"), Some("/home/u/.config/gpy")),
            (None, None, None),
            // Empty or relative values are unset.
            (Some(""), Some("/home/u"), Some("/home/u/.config/gpy")),
            (Some(""), None, None),
            (Some("cfg"), Some("/home/u"), Some("/home/u/.config/gpy")),
            (None, Some("relhome"), None),
            (Some("cfg"), Some(""), None),
        ];

        for (xdg_config, home, expected) in cases {
            assert_eq!(
                config_root_for(xdg_config.map(Path::new), home.map(Path::new), Os::Unix),
                expected.map(PathBuf::from),
                "config root for (XDG_CONFIG_HOME={xdg_config:?}, HOME={home:?})"
            );
        }
    }

    /// The fail-closed errors name every variable that would fix them.
    #[test]
    fn missing_root_errors_name_the_variables() {
        for (message, variables) in [
            (
                NO_RUNTIME_DIR,
                &["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"][..],
            ),
            (NO_CONFIG_DIR, &["XDG_CONFIG_HOME", "HOME"][..]),
        ] {
            assert!(!message.contains('\n'), "one line: {message}");
            for variable in variables {
                assert!(message.contains(variable), "{variable} in: {message}");
            }
        }
    }

    /// Windows-absolute `XDG_CONFIG_HOME` is honoured only under `Os::Windows`.
    #[test]
    fn config_root_windows_xdg_override() {
        let xdg = Some(Path::new(r"C:\Users\u\cfg"));
        let home = Some(Path::new("/home/u"));
        assert_eq!(
            config_root_for(xdg, home, Os::Windows),
            Some(PathBuf::from(r"C:\Users\u\cfg").join("gpy")),
        );
        assert_eq!(
            config_root_for(xdg, home, Os::Unix),
            Some(PathBuf::from("/home/u/.config/gpy")),
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

    /// `--socket` beats `GPY_AGENT_SOCKET_PATH`, which beats the runtime
    /// directory; without one, only an override resolves.
    #[test]
    fn socket_path_precedence_is_flag_then_env_then_default() {
        type SocketCase<'a> = (
            Option<&'a Path>,
            Option<&'a OsStr>,
            Option<&'a Path>,
            Option<SocketPath>,
        );
        let run_dir = Some(Path::new("/run/gpy"));
        let flag_path = Path::new("/flag.sock");
        let env_value = OsStr::new("/env.sock");
        let cases: [SocketCase<'_>; 7] = [
            (
                Some(flag_path),
                Some(env_value),
                run_dir,
                Some(socket_at("/flag.sock", true)),
            ),
            (
                Some(flag_path),
                None,
                run_dir,
                Some(socket_at("/flag.sock", true)),
            ),
            (
                None,
                Some(env_value),
                run_dir,
                Some(socket_at("/env.sock", true)),
            ),
            (
                None,
                None,
                run_dir,
                Some(socket_at("/run/gpy/gpy.sock", false)),
            ),
            (
                Some(flag_path),
                None,
                None,
                Some(socket_at("/flag.sock", true)),
            ),
            (
                None,
                Some(env_value),
                None,
                Some(socket_at("/env.sock", true)),
            ),
            (None, None, None, None),
        ];
        for (flag, env, runtime, expected) in cases {
            assert_eq!(
                socket_path_for(flag, env, runtime),
                expected,
                "flag {flag:?}, env {env:?}, runtime {runtime:?}"
            );
        }
    }

    /// An empty `GPY_AGENT_SOCKET_PATH` falls through to the default, as in the shells.
    #[test]
    fn empty_socket_env_counts_as_unset() {
        let resolved = socket_path_for(None, Some(OsStr::new("")), Some(Path::new("/run/gpy")));
        assert_eq!(resolved, Some(socket_at("/run/gpy/gpy.sock", false)));
        assert!(!resolved.is_some_and(|socket| socket.is_override()));
        assert_eq!(socket_path_for(None, Some(OsStr::new("")), None), None);
    }

    /// Every row of `xdg_absoluteness_vectors.tsv`: an `XDG_*` value is honoured
    /// on Unix iff absolute; otherwise each root falls through as if unset, to
    /// the home fallback or, with no usable home, to no root at all.
    #[test]
    fn xdg_absoluteness_shared_vectors() {
        let home = Path::new("/home/u");
        let mut checked = 0_u32;
        for row in crate::vector_cells::rows("xdg_absoluteness_vectors.tsv") {
            let [raw, absolute, ..] = row.as_slice() else {
                panic!("short row {row:?}");
            };
            let value = crate::vector_cells::cell(raw);
            let given = Some(Path::new(&value));
            let honoured = match absolute.as_str() {
                "yes" => true,
                "no" => false,
                other => panic!("absolute must be yes or no, got {other:?} in {row:?}"),
            };
            assert_eq!(xdg_path(given, Os::Unix).is_some(), honoured, "row {row:?}");
            let rooted = honoured.then(|| Path::new(&value).join("gpy"));
            let home_or = |fallback: PathBuf| rooted.clone().or(Some(fallback));
            assert_eq!(
                runtime_root_for(given, None, Some(home), None, Os::Unix),
                home_or(home.join(".cache").join("gpy")),
                "runtime root for {row:?}"
            );
            assert_eq!(
                cache_root_for(given, Some(home), None, Os::Unix),
                home_or(home.join(".cache").join("gpy")),
                "cache root for {row:?}"
            );
            assert_eq!(
                config_root_for(given, Some(home), Os::Unix),
                home_or(home.join(".config").join("gpy")),
                "config root for {row:?}"
            );
            // Nothing usable: an unhonoured value leaves no root to fall back on.
            assert_eq!(
                runtime_root_for(given, None, None, None, Os::Unix),
                rooted,
                "{row:?}"
            );
            assert_eq!(
                cache_root_for(given, None, None, Os::Unix),
                rooted,
                "{row:?}"
            );
            assert_eq!(config_root_for(given, None, Os::Unix), rooted, "{row:?}");
            checked = checked.saturating_add(1);
        }
        assert!(checked > 0, "no XDG absoluteness vectors were checked");
    }
}
