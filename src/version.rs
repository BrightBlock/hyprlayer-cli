//! Version checking and update notification.

use anyhow::Result;
use serde::Deserialize;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agents;
use crate::config;

/// GitHub Release API response (minimal fields needed)
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

/// How hyprlayer was installed - determines upgrade instructions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    WindowsInstaller,
    Unknown,
}

impl InstallMethod {
    /// Detect installation method based on executable path
    pub fn detect() -> Self {
        let exe_path = match env::current_exe() {
            Ok(p) => p,
            Err(_) => return Self::Unknown,
        };

        let path_str = exe_path.to_string_lossy();

        // Homebrew: /opt/homebrew/Cellar/... or /usr/local/Cellar/...
        if path_str.contains("/homebrew/") || path_str.contains("/Cellar/") {
            return Self::Homebrew;
        }

        // Cargo: ~/.cargo/bin/hyprlayer
        if path_str.contains(".cargo/bin") || path_str.contains(".cargo\\bin") {
            return Self::Cargo;
        }

        // Windows installer: %USERPROFILE%\.hyprlayer\bin
        if path_str.contains(".hyprlayer\\bin") || path_str.contains(".hyprlayer/bin") {
            return Self::WindowsInstaller;
        }

        Self::Unknown
    }

    /// Get the upgrade command for this installation method
    pub fn upgrade_hint(&self) -> &'static str {
        match self {
            Self::Homebrew => "Run 'brew upgrade hyprlayer' to upgrade",
            Self::Cargo => "Run 'cargo install hyprlayer' to upgrade",
            Self::WindowsInstaller => "Re-run the install script to upgrade",
            Self::Unknown => "Download the latest release from GitHub",
        }
    }
}

/// Result of checking for updates
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    #[allow(dead_code)]
    pub download_url: String,
    pub install_method: InstallMethod,
}

/// Check GitHub for the latest release version.
/// Returns Some(UpdateInfo) if a newer version is available, None otherwise.
/// Returns Ok(None) on any error (network, parse, etc.) - fails silently.
pub fn check_for_updates() -> Option<UpdateInfo> {
    check_for_updates_inner().ok().flatten()
}

fn check_for_updates_inner() -> Result<Option<UpdateInfo>> {
    let current = env!("CARGO_PKG_VERSION");

    // Fetch latest release from GitHub
    let url = "https://api.github.com/repos/BrightBlock/hyprlayer-cli/releases/latest";
    let json = agents::curl_get_json(url, Some(5))?;

    let release: GitHubRelease = serde_json::from_str(&json)?;

    // Strip 'v' prefix if present (e.g., "v1.5.0" -> "1.5.0")
    let latest = release.tag_name.trim_start_matches('v');

    if is_newer_version(latest, current) {
        Ok(Some(UpdateInfo {
            current: current.to_string(),
            latest: latest.to_string(),
            download_url: release.html_url,
            install_method: InstallMethod::detect(),
        }))
    } else {
        Ok(None)
    }
}

/// Compare two semver version strings numerically.
/// Returns true if `a` is newer than `b`.
/// Pre-release suffixes (e.g., "-beta.1") are stripped before comparison.
fn is_newer_version(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        // Strip pre-release suffix: "1.5.0-beta.1" -> "1.5.0"
        let base = v.split('-').next().unwrap_or(v);
        base.split('.').filter_map(|s| s.parse().ok()).collect()
    };
    parse(a) > parse(b)
}

/// Check for updates if enough time has passed since last check.
/// Runs silently on any error - never blocks command execution.
pub fn maybe_check_for_updates() {
    // Try to load config; if it doesn't exist, skip update check
    let config_path = match config::get_default_config_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    if !config_path.exists() {
        return;
    }

    let mut thoughts_config = match config::ThoughtsConfig::load(&config_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Check if updates are disabled
    if thoughts_config.disable_update_check {
        return;
    }

    // Check if 24 hours have passed since last check
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let last_check = thoughts_config.last_version_check.unwrap_or(0);
    let one_day_seconds = 24 * 60 * 60;

    if now - last_check < one_day_seconds {
        return;
    }

    // Perform the check
    if let Some(update_info) = check_for_updates() {
        print_update_notification(&update_info);
    }

    // Update last check timestamp (ignore save errors)
    thoughts_config.last_version_check = Some(now);
    let _ = thoughts_config.save(&config_path);
}

/// Print update notification with install-method-specific hint
fn print_update_notification(info: &UpdateInfo) {
    use colored::Colorize;

    let hint = info.install_method.upgrade_hint();
    println!(
        "\n{} {} → {} ({})\n",
        "Update available:".yellow(),
        info.current,
        info.latest.green(),
        hint
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_works() {
        assert!(is_newer_version("1.5.0", "1.4.0"));
        assert!(is_newer_version("1.4.1", "1.4.0"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(is_newer_version("1.10.0", "1.9.0")); // double-digit segment
        assert!(!is_newer_version("1.4.0", "1.4.0")); // equal
        assert!(!is_newer_version("1.3.0", "1.4.0")); // older
    }

    #[test]
    fn version_comparison_prerelease() {
        // Pre-release of same version is not newer
        assert!(!is_newer_version("1.5.0-beta.1", "1.5.0"));
        // Pre-release of newer version is still newer
        assert!(is_newer_version("1.6.0-rc.1", "1.5.0"));
        // Two pre-releases of same version are equal (suffix stripped)
        assert!(!is_newer_version("1.5.0-beta.1", "1.5.0-beta.2"));
    }

    #[test]
    fn version_comparison_mismatched_segments() {
        // Shorter version treated as less if it's a prefix
        assert!(is_newer_version("1.4.1", "1.4"));
        assert!(!is_newer_version("1.4", "1.4.0"));
        // Two-segment vs three-segment
        assert!(is_newer_version("1.5", "1.4.9"));
    }

    #[test]
    fn version_comparison_empty_and_malformed() {
        assert!(!is_newer_version("", "1.0.0"));
        assert!(!is_newer_version("", ""));
        assert!(is_newer_version("1.0.0", ""));
        assert!(!is_newer_version("nightly", "1.0.0"));
    }

    #[test]
    fn strip_v_prefix() {
        let tag = "v1.5.0";
        let version = tag.trim_start_matches('v');
        assert_eq!(version, "1.5.0");
    }

    #[test]
    fn install_method_upgrade_hints() {
        assert_eq!(
            InstallMethod::Homebrew.upgrade_hint(),
            "Run 'brew upgrade hyprlayer' to upgrade"
        );
        assert_eq!(
            InstallMethod::Cargo.upgrade_hint(),
            "Run 'cargo install hyprlayer' to upgrade"
        );
        assert_eq!(
            InstallMethod::WindowsInstaller.upgrade_hint(),
            "Re-run the install script to upgrade"
        );
        assert_eq!(
            InstallMethod::Unknown.upgrade_hint(),
            "Download the latest release from GitHub"
        );
    }
}
