//! Version checking and update notification.

use std::env;
use std::path::{Path, PathBuf};

use crate::agents;
use crate::config;
use crate::telemetry;
use crate::version_source;

const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

const TELEMETRY_FLUSH_INTERVAL_SECS: i64 = 5 * 60;
const INSTALL_METHOD_ENV: &str = "HYPRLAYER_INSTALL_METHOD";
const INSTALL_METHOD_MARKER: &str = "hyprlayer.install-method";

fn unix_now() -> i64 {
    telemetry::unix_now() as i64
}

fn should_skip_due_to_throttle(last_check: i64, now: i64, interval_secs: i64) -> bool {
    now - last_check < interval_secs
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    Winget,
    WindowsInstaller,
    Scoop,
    Aur,
    Unknown,
}

impl InstallMethod {
    pub fn detect() -> Self {
        if let Ok(raw) = env::var(INSTALL_METHOD_ENV)
            && let Some(method) = Self::parse(&raw)
        {
            return method;
        }

        let exe_path = match env::current_exe() {
            Ok(p) => p,
            Err(_) => return Self::Unknown,
        };
        if let Some(method) = detect_from_marker(&exe_path) {
            return method;
        }

        let path_str = exe_path.to_string_lossy();
        classify_path(&path_str, pacman_has_hyprlayer_bin)
    }

    pub fn can_auto_update(&self) -> bool {
        matches!(self, Self::WindowsInstaller)
    }

    /// Render the upgrade command for this method, interpolating `version`
    /// where the install path needs it (currently only Cargo). Pass the
    /// `latest` we just resolved so the printed command is copy-pasteable.
    pub fn upgrade_hint(&self, version: &str) -> String {
        match self {
            Self::Homebrew => "brew upgrade hyprlayer".to_string(),
            Self::Cargo => format!(
                "cargo install --git https://github.com/{} --tag v{} --force",
                agents::REPO,
                version
            ),
            Self::Winget => "winget upgrade BrightBlock.Hyprlayer".to_string(),
            Self::Scoop => "scoop update hyprlayer".to_string(),
            Self::Aur => "yay -S hyprlayer-bin   # or your AUR helper".to_string(),
            Self::WindowsInstaller => "Re-run the install script".to_string(),
            Self::Unknown => "Download the latest release from GitHub".to_string(),
        }
    }

    /// Canonical marker-file / env-var values, accepted case-insensitively
    /// with `_` normalized to `-`. Aliases are intentionally not supported:
    /// the marker file is a stable ABI written by our own installers, and
    /// the env var is a debug/test affordance — both have a single correct
    /// spelling per variant.
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "homebrew" => Some(Self::Homebrew),
            "cargo" => Some(Self::Cargo),
            "winget" => Some(Self::Winget),
            "windows-installer" => Some(Self::WindowsInstaller),
            "scoop" => Some(Self::Scoop),
            "aur" => Some(Self::Aur),
            _ => None,
        }
    }
}

const PATH_RULES: &[(InstallMethod, &[&str])] = &[
    (InstallMethod::Homebrew, &["/homebrew/", "/Cellar/"]),
    (InstallMethod::Cargo, &[".cargo/bin", ".cargo\\bin"]),
    (
        InstallMethod::Winget,
        &["WinGet\\Packages", "WinGet/Packages"],
    ),
    (
        InstallMethod::Scoop,
        &[
            "\\scoop\\apps\\",
            "\\scoop\\shims\\",
            "/scoop/apps/",
            "/scoop/shims/",
        ],
    ),
    (
        InstallMethod::WindowsInstaller,
        &[".hyprlayer\\bin", ".hyprlayer/bin"],
    ),
];

fn detect_from_marker(exe_path: &Path) -> Option<InstallMethod> {
    install_method_marker_candidates(exe_path)
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .find_map(|raw| InstallMethod::parse(&raw))
}

fn install_method_marker_candidates(exe_path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(bin_dir) = exe_path.parent() {
        candidates.push(bin_dir.join(INSTALL_METHOD_MARKER));
        if let Some(install_dir) = bin_dir.parent() {
            candidates.push(install_dir.join(INSTALL_METHOD_MARKER));
        }
    }
    candidates
}

fn classify_path<F: FnOnce() -> bool>(path: &str, aur_pacman_present: F) -> InstallMethod {
    if let Some((method, _)) = PATH_RULES
        .iter()
        .find(|(_, needles)| needles.iter().any(|needle| path.contains(needle)))
    {
        return *method;
    }
    // pacman probe runs only if no path rule matched and we're on Linux at /usr/bin/.
    if cfg!(target_os = "linux") && path.starts_with("/usr/bin/") && aur_pacman_present() {
        return InstallMethod::Aur;
    }
    InstallMethod::Unknown
}

#[cfg(target_os = "linux")]
fn pacman_has_hyprlayer_bin() -> bool {
    let Ok(entries) = std::fs::read_dir("/var/lib/pacman/local") else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("hyprlayer-bin-")
    })
}

#[cfg(not(target_os = "linux"))]
fn pacman_has_hyprlayer_bin() -> bool {
    false
}

pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub install_method: InstallMethod,
}

/// Pre-release suffixes ("-beta.1") and build metadata ("+build.3") are
/// stripped before comparison, so only the `major.minor.patch` core is compared.
pub(crate) fn is_newer_version(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        let base = v.split(['-', '+']).next().unwrap_or(v);
        base.split('.').filter_map(|s| s.parse().ok()).collect()
    };
    parse(a) > parse(b)
}

/// Whether the assets on disk need replacing, decided purely by comparing
/// the version the last install recorded against the one we want.
///
/// Takes both versions as arguments and touches nothing else, so the
/// settled case — the overwhelmingly common one — reaches no network at
/// all. This replaces the pre-1.6.0 shape, which resolved `master` HEAD
/// over the wire before it could answer.
///
/// `None` (a config written before 1.6.0, or an install that never
/// recorded one) always counts as stale, which is what makes the first
/// 1.6.0 run install exactly once.
pub(crate) fn should_reinstall(installed_version: Option<&str>, desired_version: &str) -> bool {
    installed_version != Some(desired_version)
}

/// What `check_release_in` did, so `run_startup_checks` can persist state
/// once and the caller can short-circuit the rest of the CLI on auto-update.
#[derive(Debug, PartialEq)]
enum ReleaseCheckOutcome {
    /// Within the throttle window — nothing changed.
    Throttled,
    /// Network attempt occurred; `last_version_check` must be persisted.
    Checked,
    /// Auto-update succeeded; binary was replaced. Caller exits after save.
    AutoUpdated { new_version: String },
}

/// Run startup checks using the same config path as the selected command.
/// Exits the process with status 0 if a silent auto-update succeeded.
pub fn run_startup_checks(
    config_path: Option<&std::path::Path>,
    allow_background_flush: bool,
    allow_agent_provision: bool,
) {
    let resolved = config_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| config::get_default_config_path().ok());
    let Some(config_path) = resolved else {
        return;
    };
    let mut cfg = if config_path.exists() {
        let Ok(cfg) = config::HyprlayerConfig::load(&config_path) else {
            return;
        };
        cfg
    } else {
        config::HyprlayerConfig::default()
    };

    let now = unix_now();

    // MUST run before `disable_update_check`: that flag suppresses
    // update notifications, not corporate telemetry policy.
    let allow_shellout = should_run_discovery(cfg.telemetry.last_enrollment_check, now);
    let _ = telemetry::lifecycle::auto_enroll_and_enforce(&mut cfg, &config_path, allow_shellout);

    // Agent provisioning is a local runtime invariant, not an update
    // notification. It runs even when release checks are disabled and can
    // bootstrap a machine with no prior agent installation or config file.
    let agents_changed = allow_agent_provision && ensure_bundle_set_in(&mut cfg, now);

    if cfg.disable_update_check {
        if agents_changed {
            let _ = cfg.save(&config_path);
        }
        return;
    }

    let release_outcome = check_release_in(&mut cfg, now);

    // Exit BEFORE agent reinstall / telemetry flush: the executable has
    // been replaced, but this process is still running the old binary
    // in memory. Continuing would (a) re-exec the new binary as a child
    // for telemetry flush — risking schema mismatch — and (b) print
    // post-update startup chatter after the "please re-run" line.
    if let ReleaseCheckOutcome::AutoUpdated { new_version } = release_outcome {
        let _ = cfg.save(&config_path);
        println!("hyprlayer updated to {new_version}, please re-run your command.");
        std::process::exit(0);
    }

    let release_changed = !matches!(release_outcome, ReleaseCheckOutcome::Throttled);
    let telemetry_changed =
        allow_background_flush && maybe_flush_telemetry_in(&mut cfg, now, &config_path);

    if release_changed || agents_changed || telemetry_changed {
        let _ = cfg.save(&config_path);
    }
}

/// Treats future and `u64`-overflowing anchors as "no anchor" so a
/// poisoned `lastEnrollmentCheck` can't permanently suppress discovery.
fn should_run_discovery(last_enrollment_check: u64, now: i64) -> bool {
    let last = i64::try_from(last_enrollment_check).unwrap_or(i64::MAX);
    if last > now {
        return true;
    }
    !should_skip_due_to_throttle(last, now, CHECK_INTERVAL_SECS)
}

/// Spawn a detached telemetry flush and bump the throttle on any spawn attempt.
fn maybe_flush_telemetry_in(
    cfg: &mut config::HyprlayerConfig,
    now: i64,
    config_path: &std::path::Path,
) -> bool {
    // Escape hatch for the integration suite, which runs the real binary
    // as a subprocess and asserts on the spool's contents. A detached
    // `telemetry flush` child would race those assertions — claiming and
    // draining the spool mid-read — and would POST the suite's events to
    // PostHog. This disables only the *background* flush; an explicit
    // `telemetry flush` is unaffected. Never set in production.
    if env::var_os("HYPRLAYER_DISABLE_BACKGROUND_FLUSH").is_some() {
        return false;
    }
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

/// Bump `last_version_check` on every network attempt.
fn check_release_in(cfg: &mut config::HyprlayerConfig, now: i64) -> ReleaseCheckOutcome {
    let current = env!("CARGO_PKG_VERSION");
    let last_check = cfg.last_version_check.unwrap_or(0);
    if should_skip_due_to_throttle(last_check, now, CHECK_INTERVAL_SECS) {
        return ReleaseCheckOutcome::Throttled;
    }
    // Bump the throttle *before* attempting any network/PM work; a flaky
    // source mustn't cause us to retry on every startup until the next window.
    cfg.last_version_check = Some(now);

    let method = InstallMethod::detect();
    let latest = version_source::latest_available_for(&method);
    if !should_notify(current, latest.as_deref()) {
        return ReleaseCheckOutcome::Checked;
    }
    let info = UpdateInfo {
        current: current.to_string(),
        latest: latest.unwrap(),
        install_method: method,
    };
    if should_auto_update(cfg.auto_update, &method) {
        match crate::commands::self_update::run_silent(&info) {
            Ok(()) => {
                return ReleaseCheckOutcome::AutoUpdated {
                    new_version: info.latest,
                };
            }
            Err(e) => eprintln!("Auto-update failed ({e}); falling back to notification."),
        }
    }
    print_update_notification(&info);
    ReleaseCheckOutcome::Checked
}

fn should_auto_update(auto_update_flag: bool, method: &InstallMethod) -> bool {
    auto_update_flag && method.can_auto_update()
}

fn should_notify(current: &str, latest_for_method: Option<&str>) -> bool {
    match latest_for_method {
        Some(latest) => is_newer_version(latest, current),
        None => false,
    }
}

/// Startup ensure for the atomic Claude + Codex bundle set.
///
/// Freshness is a version comparison against what the last install recorded
/// (`assets_need_refresh`), not a timer and not a `master` HEAD lookup: an
/// already-current install costs no network I/O, and a binary upgrade — or a
/// newly set pin — refreshes on the very next run instead of waiting out a
/// 24h window.
///
/// `last_agent_check` survives as the backoff for a refresh that *failed*.
/// Without it, an unreachable GitHub would mean a download attempt on every
/// single command; with it, a failure costs one attempt per day. A
/// successful refresh clears the anchor (`record_assets_version`), so it
/// never delays the next legitimately-stale refresh.
fn ensure_bundle_set_in(cfg: &mut config::HyprlayerConfig, now: i64) -> bool {
    let desired = cfg.desired_assets_version().to_string();
    if !cfg.assets_need_refresh() && agents::bundle_set_is_installed(&desired) {
        return false;
    }
    let setup_backed_off = agent_setup_is_backed_off(cfg.last_agent_check, now);

    // A complete generation can repair a missing/repointed link farm without
    // touching the network. This also adopts a store installed before the
    // config existed.
    match agents::repair_bundle_set_links(&desired) {
        Ok(Some(changed)) => {
            if agents::bundle_set_is_installed(&desired) {
                if changed > 0 {
                    eprintln!("Repaired Claude + Codex agent links ({changed} links).")
                }
                cfg.record_assets_version(&desired);
                crate::commands::ai::install_claude_hook_if_applicable(cfg);
                return true;
            }

            // A complete local generation plus unhealthy live farms means a
            // user-owned collision was deliberately preserved (including a
            // real ~/.agents/skills). Re-downloading identical bytes cannot
            // resolve that, and retrying on every command would be noisy.
            if setup_backed_off {
                return false;
            }
            cfg.last_agent_check = Some(now);
            eprintln!(
                "Claude + Codex agent setup remains incomplete because an existing path was left untouched. \
                 Run 'hyprlayer ai status' for details, then 'hyprlayer ai reinstall' after resolving the collision."
            );
            return true;
        }
        Ok(None) => {}
        Err(error) => {
            if setup_backed_off {
                return false;
            }
            eprintln!(
                "Failed to repair Claude + Codex agent links: {error}. \
                 Run 'hyprlayer ai reinstall' to retry."
            );
        }
    }

    if setup_backed_off {
        return false;
    }

    // Bump before the attempt so an offline machine retries at most daily.
    cfg.last_agent_check = Some(now);
    let had_install = agents::bundle_set_has_existing_install();
    match agents::install_bundle_set(cfg.agents_pinned_version.as_deref(), true) {
        Ok(outcome) => {
            if let Some(sha) = outcome.sha {
                cfg.agents_installed_sha = Some(sha);
            }
            if agents::bundle_set_is_installed(&desired) {
                if outcome.changed > 0 {
                    eprintln!(
                        "{} Claude + Codex agent files ({} links).",
                        if had_install { "Updated" } else { "Installed" },
                        outcome.changed
                    );
                }
                cfg.record_assets_version(&desired);
            } else {
                eprintln!(
                    "Claude + Codex bundles were downloaded, but an existing path prevented the full link set. \
                     Run 'hyprlayer ai status' for details."
                );
            }
        }
        Err(e) => eprintln!(
            "Failed to provision Claude + Codex agent files: {}. \
             Run 'hyprlayer ai reinstall' to retry.",
            e
        ),
    }
    crate::commands::ai::install_claude_hook_if_applicable(cfg);
    true
}

fn agent_setup_is_backed_off(last_agent_check: Option<i64>, now: i64) -> bool {
    should_skip_due_to_throttle(last_agent_check.unwrap_or(0), now, CHECK_INTERVAL_SECS)
}

/// Writes to stderr so it never pollutes stdout-piped output.
fn print_update_notification(info: &UpdateInfo) {
    use colored::Colorize;

    let hint = info.install_method.upgrade_hint(&info.latest);
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
        assert!(is_newer_version("1.10.0", "1.9.0"));
        assert!(!is_newer_version("1.4.0", "1.4.0"));
        assert!(!is_newer_version("1.3.0", "1.4.0"));
    }

    #[test]
    fn version_comparison_prerelease() {
        assert!(!is_newer_version("1.5.0-beta.1", "1.5.0"));
        assert!(is_newer_version("1.6.0-rc.1", "1.5.0"));
        assert!(!is_newer_version("1.5.0-beta.1", "1.5.0-beta.2"));
    }

    #[test]
    fn version_comparison_build_metadata() {
        // Build metadata ("+...") must be stripped, like pre-release suffixes.
        assert!(!is_newer_version("1.5.0+build.3", "1.5.0"));
        assert!(!is_newer_version("1.5.0", "1.5.0+build.3"));
        assert!(is_newer_version("1.6.0+build.1", "1.5.0"));
        assert!(!is_newer_version("1.5.0-rc.1+build.7", "1.5.0"));
    }

    #[test]
    fn version_comparison_mismatched_segments() {
        assert!(is_newer_version("1.4.1", "1.4"));
        assert!(!is_newer_version("1.4", "1.4.0"));
        assert!(is_newer_version("1.5", "1.4.9"));
    }

    #[test]
    fn version_comparison_empty_and_malformed() {
        assert!(!is_newer_version("", "1.0.0"));
        assert!(!is_newer_version("", ""));
        assert!(is_newer_version("1.0.0", ""));
        assert!(!is_newer_version("nightly", "1.0.0"));
    }

    /// The freshness truth table, now over bundle versions rather than
    /// `master` SHAs. Note what is *not* here: any source of truth beyond
    /// the two arguments. The comparison cannot reach the network, which is
    /// what makes the settled case free.
    #[test]
    fn should_reinstall_truth_table() {
        // Mismatch reinstalls, match no-ops.
        assert!(should_reinstall(Some("1.6.0"), "1.6.1"));
        assert!(!should_reinstall(Some("1.6.0"), "1.6.0"));
        // Downgrades are mismatches too — pinning back to an older bundle
        // has to install it, not decide it is already new enough.
        assert!(should_reinstall(Some("1.6.1"), "1.6.0"));
        // Exact strings, not semver cores: a prerelease pin is a different
        // bundle from the release of the same version and must be fetched.
        assert!(should_reinstall(Some("1.6.0-rc.1"), "1.6.0"));

        // Unknown — a config written before 1.6.0, or an install that never
        // recorded — is always stale, which is the migration trigger.
        assert!(should_reinstall(None, "1.6.0"));
        assert!(should_reinstall(Some(""), "1.6.0"));
    }

    /// A binary upgrade must move the skills on the *next* run, not a day
    /// later: with freshness decided by version, `last_agent_check` is only
    /// a backoff for a failed refresh, and a successful one clears it.
    #[test]
    fn a_binary_upgrade_refreshes_without_waiting_out_the_throttle() {
        let now: i64 = 2_000_000_000;

        // A refresh that succeeded an hour ago — well inside the window.
        let mut cfg = config::HyprlayerConfig {
            last_agent_check: Some(now - 3600),
            ..Default::default()
        };
        cfg.record_assets_version("1.6.0");

        assert!(
            !should_reinstall(cfg.agents_installed_version.as_deref(), "1.6.0"),
            "same version: nothing to do, and nothing fetched to find that out"
        );
        assert!(
            should_reinstall(cfg.agents_installed_version.as_deref(), "1.6.1"),
            "the upgraded binary wants its own bundle"
        );
        assert!(
            !should_skip_due_to_throttle(
                cfg.last_agent_check.unwrap_or(0),
                now,
                CHECK_INTERVAL_SECS
            ),
            "a successful refresh must clear the anchor, or the upgrade waits 24h"
        );
    }

    /// The other half: a refresh that *failed* leaves the anchor set and no
    /// recorded version, so it stays stale but retries once a day instead of
    /// on every command.
    #[test]
    fn a_failed_refresh_backs_off_for_a_day() {
        let now: i64 = 2_000_000_000;

        // `ensure_bundle_set_in` bumps the anchor before attempting; the
        // install then errored, so no version was recorded.
        let cfg = config::HyprlayerConfig {
            last_agent_check: Some(now),
            ..Default::default()
        };

        assert!(should_reinstall(
            cfg.agents_installed_version.as_deref(),
            "1.6.0"
        ));
        assert!(
            should_skip_due_to_throttle(
                cfg.last_agent_check.unwrap(),
                now + 60,
                CHECK_INTERVAL_SECS
            ),
            "a minute later: no retry"
        );
        assert!(
            !should_skip_due_to_throttle(
                cfg.last_agent_check.unwrap(),
                now + CHECK_INTERVAL_SECS,
                CHECK_INTERVAL_SECS
            ),
            "a day later: retry"
        );
    }

    #[test]
    fn preserved_agent_collision_warning_obeys_the_refresh_backoff() {
        let now = 2_000_000_000;
        assert!(
            !agent_setup_is_backed_off(None, now),
            "the first incomplete local repair should be reported"
        );
        assert!(
            agent_setup_is_backed_off(Some(now), now + 60),
            "the same preserved collision must not warn on every command"
        );
        assert!(
            !agent_setup_is_backed_off(Some(now), now + CHECK_INTERVAL_SECS),
            "the user gets another actionable reminder after the backoff"
        );
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
        assert!(should_skip_due_to_throttle(now + 5, now, i));

        let t = TELEMETRY_FLUSH_INTERVAL_SECS;
        assert!(t < i);
        assert!(should_skip_due_to_throttle(now - (t - 1), now, t));
        assert!(!should_skip_due_to_throttle(now - t, now, t));
    }

    #[test]
    fn enrollment_throttle_gates_shellout() {
        let now: i64 = 2_000_000_000;
        let i = CHECK_INTERVAL_SECS;
        let allow = |last: i64| !should_skip_due_to_throttle(last, now, i);

        assert!(!allow(now - 100), "100s ago: still inside the throttle");
        assert!(
            !allow(now - (i - 1)),
            "1s before window edge: still skipped"
        );
        assert!(allow(now - i), "at window edge: shellout allowed");
        assert!(
            allow(now - 100_000),
            "well outside window: shellout allowed"
        );
        assert!(allow(0), "fresh install (zero anchor): shellout allowed");
    }

    #[test]
    fn classify_path_recognizes_every_variant() {
        use InstallMethod::*;
        let no_pacman = || false;
        let with_pacman = || true;

        assert_eq!(
            classify_path(
                "/opt/homebrew/Cellar/hyprlayer/1.5.5/bin/hyprlayer",
                no_pacman
            ),
            Homebrew
        );
        assert_eq!(
            classify_path("/usr/local/Cellar/hyprlayer/1.5.5/bin/hyprlayer", no_pacman),
            Homebrew
        );

        assert_eq!(
            classify_path("/home/jt/.cargo/bin/hyprlayer", no_pacman),
            Cargo
        );
        assert_eq!(
            classify_path(r"C:\Users\jt\.cargo\bin\hyprlayer.exe", no_pacman),
            Cargo
        );

        assert_eq!(
            classify_path(
                r"C:\Users\jt\AppData\Local\Microsoft\WinGet\Packages\BrightBlock.Hyprlayer_..\hyprlayer.exe",
                no_pacman,
            ),
            Winget
        );

        assert_eq!(
            classify_path(
                r"C:\Users\jt\scoop\apps\hyprlayer\current\hyprlayer.exe",
                no_pacman,
            ),
            Scoop
        );
        assert_eq!(
            classify_path(r"C:\Users\jt\scoop\shims\hyprlayer.exe", no_pacman),
            Scoop
        );

        assert_eq!(
            classify_path(r"C:\Users\jt\.hyprlayer\bin\hyprlayer.exe", no_pacman),
            WindowsInstaller
        );

        if cfg!(target_os = "linux") {
            assert_eq!(classify_path("/usr/bin/hyprlayer", with_pacman), Aur);
            assert_eq!(classify_path("/usr/bin/hyprlayer", no_pacman), Unknown);
        }

        assert_eq!(classify_path("/opt/random/hyprlayer", no_pacman), Unknown);
    }

    #[test]
    fn install_method_parser_accepts_canonical_names_only() {
        assert_eq!(
            InstallMethod::parse("windows-installer"),
            Some(InstallMethod::WindowsInstaller)
        );
        assert_eq!(
            InstallMethod::parse("WINDOWS_INSTALLER"),
            Some(InstallMethod::WindowsInstaller)
        );
        assert_eq!(
            InstallMethod::parse("homebrew"),
            Some(InstallMethod::Homebrew)
        );
        assert_eq!(InstallMethod::parse("aur"), Some(InstallMethod::Aur));

        // Aliases that used to parse are now rejected so the marker-file
        // ABI is unambiguous.
        assert_eq!(InstallMethod::parse("brew"), None);
        assert_eq!(InstallMethod::parse("windows"), None);
        assert_eq!(InstallMethod::parse("installer"), None);
        assert_eq!(InstallMethod::parse("hyprlayer-bin"), None);

        assert_eq!(InstallMethod::parse("garbage"), None);
    }

    #[test]
    fn install_method_marker_beats_legacy_path_detection() {
        let temp = tempfile::tempdir().unwrap();
        let bin_dir = temp.path().join(".cargo").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("hyprlayer");
        std::fs::write(&exe, b"").unwrap();
        std::fs::write(bin_dir.join(INSTALL_METHOD_MARKER), "windows-installer").unwrap();

        assert_eq!(
            detect_from_marker(&exe),
            Some(InstallMethod::WindowsInstaller)
        );
        assert_eq!(
            classify_path(&exe.to_string_lossy(), || false),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn install_method_marker_can_live_in_install_root() {
        let temp = tempfile::tempdir().unwrap();
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("hyprlayer");
        std::fs::write(&exe, b"").unwrap();
        std::fs::write(temp.path().join(INSTALL_METHOD_MARKER), "scoop").unwrap();

        assert_eq!(detect_from_marker(&exe), Some(InstallMethod::Scoop));
    }

    #[test]
    fn upgrade_hint_covers_every_variant() {
        for m in [
            InstallMethod::Homebrew,
            InstallMethod::Cargo,
            InstallMethod::Winget,
            InstallMethod::WindowsInstaller,
            InstallMethod::Scoop,
            InstallMethod::Aur,
            InstallMethod::Unknown,
        ] {
            let hint = m.upgrade_hint("1.6.0");
            assert!(!hint.is_empty(), "empty hint for {m:?}");
            assert!(
                !hint.contains("<VERSION>"),
                "unsubstituted placeholder in hint for {m:?}: {hint}"
            );
            if matches!(m, InstallMethod::Cargo) {
                assert!(hint.contains("v1.6.0"), "version not interpolated: {hint}");
            }
        }
    }

    #[test]
    fn should_notify_truth_table() {
        assert!(should_notify("1.5.5", Some("1.6.0")));
        assert!(should_notify("1.5.5", Some("1.10.0")));
        assert!(!should_notify("1.6.0", Some("1.6.0")));
        assert!(!should_notify("1.6.0", Some("1.5.5")));
        assert!(!should_notify("1.5.5", None));
        assert!(!should_notify("1.6.0", Some("1.6.0-rc1")));
        assert!(should_notify("1.5.5", Some("1.6.0-rc1")));
    }

    #[test]
    fn can_auto_update_only_for_direct_swap_methods() {
        assert!(InstallMethod::WindowsInstaller.can_auto_update());

        assert!(!InstallMethod::Unknown.can_auto_update());
        assert!(!InstallMethod::Homebrew.can_auto_update());
        assert!(!InstallMethod::Cargo.can_auto_update());
        assert!(!InstallMethod::Winget.can_auto_update());
        assert!(!InstallMethod::Scoop.can_auto_update());
        assert!(!InstallMethod::Aur.can_auto_update());
    }

    #[test]
    fn should_auto_update_requires_flag_and_capable_method() {
        assert!(!should_auto_update(false, &InstallMethod::Unknown));
        assert!(!should_auto_update(false, &InstallMethod::WindowsInstaller));
        assert!(!should_auto_update(false, &InstallMethod::Homebrew));

        assert!(should_auto_update(true, &InstallMethod::WindowsInstaller));

        assert!(!should_auto_update(true, &InstallMethod::Unknown));
        assert!(!should_auto_update(true, &InstallMethod::Homebrew));
        assert!(!should_auto_update(true, &InstallMethod::Cargo));
        assert!(!should_auto_update(true, &InstallMethod::Winget));
        assert!(!should_auto_update(true, &InstallMethod::Scoop));
        assert!(!should_auto_update(true, &InstallMethod::Aur));
    }

    #[test]
    fn should_run_discovery_rejects_future_anchors() {
        let now: i64 = 2_000_000_000;
        assert!(
            !should_run_discovery((now - 100) as u64, now),
            "recent past anchor stays throttled"
        );
        assert!(
            should_run_discovery((now - CHECK_INTERVAL_SECS) as u64, now),
            "lapsed window: allowed"
        );
        assert!(
            should_run_discovery((now + 10) as u64, now),
            "10s in the future: clamp treats as no anchor"
        );
        assert!(
            should_run_discovery(u64::MAX, now),
            "u64::MAX poisoned anchor: clamp treats as no anchor"
        );
        assert!(should_run_discovery(0, now), "fresh install anchor");
    }
}
