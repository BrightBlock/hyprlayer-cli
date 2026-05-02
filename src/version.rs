//! Version checking and update notification.

use anyhow::Result;
use serde::Deserialize;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agents;
use crate::config;

/// Throttle interval shared between the GitHub release check and the agent
/// auto-reinstall check.
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn should_skip_due_to_throttle(last_check: i64, now: i64) -> bool {
    now - last_check < CHECK_INTERVAL_SECS
}

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
    Winget,
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

        // WinGet: %LOCALAPPDATA%\Microsoft\WinGet\Packages\
        if path_str.contains("WinGet\\Packages") || path_str.contains("WinGet/Packages") {
            return Self::Winget;
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
            Self::Winget => "Run 'winget upgrade BrightBlock.Hyprlayer' to upgrade",
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

/// Equality on the cached vs upstream SHA. `None` (no SHA cached yet —
/// first run after the field was added) always counts as stale.
fn should_reinstall(installed_sha: Option<&str>, latest_sha: &str) -> bool {
    installed_sha != Some(latest_sha)
}

/// Run all once-per-invocation startup checks (release notification + agent
/// bundle auto-reinstall). Loads the config once and saves it at most once,
/// only when a check actually mutated something.
///
/// `config_path = None` means "use the default path." The caller passes
/// the parsed `--config-file` value when present so a user with a custom
/// config (and their custom `disableUpdateCheck` setting) gets the
/// expected startup behavior.
pub fn run_startup_checks(config_path: Option<&std::path::Path>) {
    let default_path;
    let config_path = match config_path {
        Some(p) => p,
        None => match config::get_default_config_path() {
            Ok(p) => {
                default_path = p;
                &default_path
            }
            Err(_) => return,
        },
    };
    let Ok(mut cfg) = config::HyprlayerConfig::load(config_path) else {
        return;
    };
    if cfg.disable_update_check {
        return;
    }

    let now = unix_now();
    let release_changed = check_release_in(&mut cfg, now);
    let agents_changed = reinstall_agents_in(&mut cfg, now);

    if release_changed || agents_changed {
        let _ = cfg.save(config_path);
    }
}

fn check_release_in(cfg: &mut config::HyprlayerConfig, now: i64) -> bool {
    if should_skip_due_to_throttle(cfg.last_version_check.unwrap_or(0), now) {
        return false;
    }
    if let Some(update_info) = check_for_updates() {
        print_update_notification(&update_info);
    }
    cfg.last_version_check = Some(now);
    true
}

fn reinstall_agents_in(cfg: &mut config::HyprlayerConfig, now: i64) -> bool {
    // Auto-reinstall only refreshes an existing install — it never bootstraps
    // a new one for a user who has not run `hyprlayer ai configure`.
    let Some(ai) = cfg.ai.as_ref() else {
        return false;
    };
    let Some(tool) = ai.agent_tool else {
        return false;
    };
    // `has_existing_install` (looser than `is_installed`) is correct here:
    // the strict sentinel check rejects exactly the stale installs that
    // most need refreshing.
    if !tool.has_existing_install() {
        return false;
    }
    let opencode_provider = ai.opencode_provider.clone();

    if should_skip_due_to_throttle(cfg.last_agent_check.unwrap_or(0), now) {
        return false;
    }
    cfg.last_agent_check = Some(now);

    let Ok(latest_sha) = agents::fetch_repo_dir_sha(tool.repo_dir()) else {
        return true;
    };
    if !should_reinstall(cfg.agents_installed_sha.as_deref(), &latest_sha) {
        return true;
    }

    eprintln!("Updating agent files for {}…", tool);
    match tool.install(opencode_provider.as_ref(), true) {
        Ok(sha) => {
            if sha.is_some() {
                cfg.agents_installed_sha = sha;
            }
        }
        Err(e) => eprintln!(
            "Failed to update agent files: {}. Run 'hyprlayer ai reinstall' to retry.",
            e
        ),
    }
    true
}

/// Print update notification with install-method-specific hint.
///
/// Writes to stderr so it never pollutes stdout-piped output (e.g.
/// `codex exec ... --json | hyprlayer codex stream`).
fn print_update_notification(info: &UpdateInfo) {
    use colored::Colorize;

    let hint = info.install_method.upgrade_hint();
    eprintln!(
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
    fn should_reinstall_truth_table() {
        assert!(should_reinstall(None, "abc"));
        assert!(!should_reinstall(Some("abc"), "abc"));
        assert!(should_reinstall(Some("abc"), "def"));
        assert!(should_reinstall(Some(""), "abc"));
    }

    #[test]
    fn check_interval_is_one_day() {
        assert_eq!(CHECK_INTERVAL_SECS, 86_400);
    }

    #[test]
    fn throttle_math() {
        let now: i64 = 2_000_000_000;
        assert!(should_skip_due_to_throttle(now, now));
        assert!(should_skip_due_to_throttle(now - 1, now));
        assert!(should_skip_due_to_throttle(
            now - (CHECK_INTERVAL_SECS - 1),
            now
        ));
        assert!(!should_skip_due_to_throttle(now - CHECK_INTERVAL_SECS, now));
        assert!(!should_skip_due_to_throttle(
            now - (CHECK_INTERVAL_SECS + 1),
            now
        ));
        assert!(!should_skip_due_to_throttle(0, now));
        // Clock skew: last_check in the future → still skip (negative
        // delta is < interval). Conservative: avoids hammering on a
        // misconfigured clock that's about to be fixed.
        assert!(should_skip_due_to_throttle(now + 5, now));
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
            InstallMethod::Winget.upgrade_hint(),
            "Run 'winget upgrade BrightBlock.Hyprlayer' to upgrade"
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
