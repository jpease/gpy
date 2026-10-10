//! Windows Subsystem for Linux detection (#849).
//!
//! A WSL distribution is a Linux system in every way the agent can observe
//! except three: the terminal (and so the fonts) lives on the Windows side,
//! `/mnt/<drive>` is a Windows filesystem, and the desktop opener `xdg-open`
//! is usually missing. Callers ask [`is_wsl`] before choosing a default that
//! would otherwise be wrong there. Nothing in the agent *requires* the answer:
//! a false negative only means a Linux default is used on WSL.

/// Whether the kernel release string (`/proc/sys/kernel/osrelease`) names WSL.
///
/// WSL 1 reports e.g. `4.4.0-19041-Microsoft`, WSL 2 e.g.
/// `5.15.133.1-microsoft-standard-WSL2`.
#[must_use]
pub fn kernel_release_is_wsl(release: &str) -> bool {
    let lower = release.to_ascii_lowercase();
    lower.contains("microsoft") || lower.contains("wsl")
}

/// Pure WSL decision from the two signals WSL provides.
///
/// `distro` is `WSL_DISTRO_NAME`, which WSL sets in every distribution's
/// environment; it covers a custom WSL 2 kernel whose release string omits
/// `microsoft`. An empty value is not a signal.
#[must_use]
pub fn detect(kernel_release: Option<&str>, distro: Option<&str>) -> bool {
    kernel_release.is_some_and(kernel_release_is_wsl)
        || distro.is_some_and(|d| !d.trim().is_empty())
}

/// Whether this process runs inside WSL. Resolved once; always `false` off
/// Linux, where neither signal exists.
#[must_use]
pub fn is_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        static IS_WSL: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok();
            let distro = std::env::var("WSL_DISTRO_NAME").ok();
            detect(release.as_deref(), distro.as_deref())
        });
        *IS_WSL
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::{detect, kernel_release_is_wsl};

    #[test]
    fn recognises_wsl1_and_wsl2_kernel_releases() {
        assert!(kernel_release_is_wsl("4.4.0-19041-Microsoft"));
        assert!(kernel_release_is_wsl(
            "5.15.133.1-microsoft-standard-WSL2\n"
        ));
    }

    #[test]
    fn ordinary_linux_kernel_releases_are_not_wsl() {
        assert!(!kernel_release_is_wsl("6.8.0-45-generic"));
        assert!(!kernel_release_is_wsl("6.1.0-13-amd64"));
    }

    #[test]
    fn distro_name_covers_a_custom_kernel_but_an_empty_one_is_no_signal() {
        assert!(detect(Some("6.6.0-custom"), Some("Ubuntu-24.04")));
        assert!(!detect(Some("6.6.0-custom"), Some("  ")));
        assert!(!detect(None, None));
    }
}
