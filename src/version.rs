//! Version checking and update notification.

use anyhow::Result;
use serde::Deserialize;
use std::env;

use crate::agents;
use crate::config;
use crate::telemetry;

/// Throttle interval shared between the GitHub release check and the agent
/// auto-reinstall check. Both poll a rate-limited external API.
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Telemetry flush throttle. The flush hits our own ingest endpoint, so a
/// much shorter cadence is fine and keeps spool size + data-loss window
/// bounded for active users.
const TELEMETRY_FLUSH_INTERVAL_SECS: i64 = 5 * 60;

fn unix_now() -> i64 {
    telemetry::unix_now() as i64
}

fn should_skip_due_to_throttle(last_check: i64, now: i64, interval_secs: i64) -> bool {
    now - last_check < interval_secs
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
///
/// `allow_background_flush = false` suppresses the background telemetry
/// flush spawn. Telemetry subcommands set this so `telemetry off` can't
/// flush queued events before disabling and `telemetry flush` can't
/// recursively spawn another flush.
pub fn run_startup_checks(config_path: Option<&std::path::Path>, allow_background_flush: bool) {
    let resolved = config_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| config::get_default_config_path().ok());
    let Some(config_path) = resolved else {
        return;
    };
    let Ok(mut cfg) = config::HyprlayerConfig::load(&config_path) else {
        return;
    };
    if cfg.disable_update_check {
        return;
    }

    let now = unix_now();
    let release_changed = check_release_in(&mut cfg, now);
    let (agents_changed, mode_elevated) = reinstall_agents_in(&mut cfg, now);
    let telemetry_changed =
        allow_background_flush && maybe_flush_telemetry_in(&mut cfg, now, &config_path);

    if release_changed || agents_changed || telemetry_changed {
        // Spool the `$identify` only after the save succeeds — a failed
        // save must not leave behind an identify event that contradicts
        // the on-disk telemetry mode.
        if cfg.save(&config_path).is_ok() && mode_elevated {
            let _ = telemetry::identify::record_identify(&cfg);
        }
    }
}

/// Spawn a detached `hyprlayer telemetry flush` if
/// `TELEMETRY_FLUSH_INTERVAL_SECS` have passed since the last flush and
/// telemetry is enabled. Passes through `--config-file` so the child reads
/// the same config as the parent. Returns true if last_flush was updated
/// so the caller persists it.
///
/// Bumps `last_flush` unconditionally on a spawn attempt — even if `spawn`
/// fails — so a broken transport can't cause us to hammer the process
/// table. The child is `Telemetry::Flush`, which sets
/// `allows_background_flush = false`, so it cannot recursively re-enter
/// this function.
fn maybe_flush_telemetry_in(
    cfg: &mut config::HyprlayerConfig,
    now: i64,
    config_path: &std::path::Path,
) -> bool {
    if cfg.telemetry.mode == config::TelemetryMode::Off {
        return false;
    }
    let last = cfg.telemetry.last_flush as i64;
    if should_skip_due_to_throttle(last, now, TELEMETRY_FLUSH_INTERVAL_SECS) {
        return false;
    }
    if let Ok(exe) = env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("telemetry").arg("flush");
        if let Ok(default) = config::get_default_config_path()
            && config_path != default
        {
            cmd.arg("--config-file").arg(config_path);
        }
        let _ = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn();
    }
    cfg.telemetry.last_flush = now as u64;
    true
}

fn check_release_in(cfg: &mut config::HyprlayerConfig, now: i64) -> bool {
    if should_skip_due_to_throttle(
        cfg.last_version_check.unwrap_or(0),
        now,
        CHECK_INTERVAL_SECS,
    ) {
        return false;
    }
    if let Some(update_info) = check_for_updates() {
        print_update_notification(&update_info);
    }
    cfg.last_version_check = Some(now);
    true
}

/// Returns `(mutated, mode_elevated)`. The caller is responsible for
/// persisting `mutated == true` and only spooling `$identify` after a
/// successful save when `mode_elevated == true`.
fn reinstall_agents_in(cfg: &mut config::HyprlayerConfig, now: i64) -> (bool, bool) {
    // Auto-reinstall only refreshes an existing install — it never
    // bootstraps a new one for a user who has not run `ai configure`.
    let Some(ai) = cfg.ai.as_ref() else {
        return (false, false);
    };
    let Some(tool) = ai.agent_tool else {
        return (false, false);
    };
    if !tool.has_existing_install() {
        return (false, false);
    }
    let opencode_provider = ai.opencode_provider.clone();

    if should_skip_due_to_throttle(cfg.last_agent_check.unwrap_or(0), now, CHECK_INTERVAL_SECS) {
        return (false, false);
    }
    cfg.last_agent_check = Some(now);

    try_refresh_agents(cfg, tool, opencode_provider.as_ref());

    // No-op for opted-out users; only refreshes the org-managed key for
    // users who already ran `telemetry on`. The auto-elevate then flips
    // anonymous → identified once an org key has been picked up, so
    // existing corporate users move off the community key on the next
    // 24h cycle without a manual `telemetry init --mode identified`.
    telemetry::lifecycle::refresh_org_config(cfg);
    let mode_elevated = telemetry::lifecycle::auto_elevate_if_org_keyed(cfg);

    (true, mode_elevated)
}

/// Compare cached SHA to upstream; if stale, run the install. Failure is
/// logged once; the next 24h cycle retries.
fn try_refresh_agents(
    cfg: &mut config::HyprlayerConfig,
    tool: agents::AgentTool,
    opencode_provider: Option<&agents::OpenCodeProvider>,
) {
    let Ok(latest_sha) = agents::fetch_repo_dir_sha(tool.repo_dir()) else {
        return;
    };
    if !should_reinstall(cfg.agents_installed_sha.as_deref(), &latest_sha) {
        return;
    }
    eprintln!("Updating agent files for {}…", tool);
    match tool.install(opencode_provider, true) {
        Ok(Some(sha)) => cfg.agents_installed_sha = Some(sha),
        Ok(None) => {}
        Err(e) => eprintln!(
            "Failed to update agent files: {}. Run 'hyprlayer ai reinstall' to retry.",
            e
        ),
    }
    crate::commands::ai::install_claude_hook_if_applicable(tool, cfg);
    // The 24h auto-reinstall has just re-pulled the bundle, which
    // includes `opencode/plugins/hyprlayer-telemetry.ts` for opencode
    // users. Run the orchestrator so opted-out users get the freshly-
    // landed plugin removed, and so the in-place legacy-beacon strip
    // catches any pre-1.5.4 bundle that hadn't been refreshed yet.
    // For telemetry-on opencode users this is a no-op via the
    // `is_installed_at` short-circuit.
    crate::commands::ai::install_opencode_plugin_if_applicable(tool, cfg);
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
    fn should_reinstall_truth_table() {
        assert!(should_reinstall(None, "abc"));
        assert!(!should_reinstall(Some("abc"), "abc"));
        assert!(should_reinstall(Some("abc"), "def"));
        assert!(should_reinstall(Some(""), "abc"));
    }

    #[test]
    fn throttle_math() {
        let now: i64 = 2_000_000_000;
        let i = CHECK_INTERVAL_SECS;
        assert!(should_skip_due_to_throttle(now, now, i));
        assert!(should_skip_due_to_throttle(now - 1, now, i));
        assert!(should_skip_due_to_throttle(now - (i - 1), now, i));
        assert!(!should_skip_due_to_throttle(now - i, now, i));
        assert!(!should_skip_due_to_throttle(now - (i + 1), now, i));
        assert!(!should_skip_due_to_throttle(0, now, i));
        // Clock skew: last_check in the future → still skip (negative
        // delta is < interval). Conservative: avoids hammering on a
        // misconfigured clock that's about to be fixed.
        assert!(should_skip_due_to_throttle(now + 5, now, i));

        // Telemetry uses a much shorter interval so a stale spool drains
        // within minutes, not the next day.
        let t = TELEMETRY_FLUSH_INTERVAL_SECS;
        assert!(t < i);
        assert!(should_skip_due_to_throttle(now - (t - 1), now, t));
        assert!(!should_skip_due_to_throttle(now - t, now, t));
    }
}
