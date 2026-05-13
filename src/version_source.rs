//! Per-install-method version sources. Returns whatever the user's channel
//! actually has — notifying a Winget user about an unmerged tag points them
//! at a version they can't install. `None` means "source unreachable or PM
//! hasn't caught up"; callers suppress the notification.

use crate::agents;
use crate::http;
use crate::version::InstallMethod;
use serde::Deserialize;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const VERSION_SOURCE_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-PM probe cap. Wide enough for a warm `brew outdated` / `winget upgrade`,
/// tight enough that a hung child can't extend startup beyond ~10s.
const PACKAGE_MANAGER_TIMEOUT: Duration = Duration::from_secs(10);

pub fn latest_available_for(method: &InstallMethod) -> Option<String> {
    match method {
        InstallMethod::Homebrew => homebrew_cli_version(),
        InstallMethod::Scoop => scoop_cli_version(),
        InstallMethod::Aur => aur_helper_version(),
        InstallMethod::Cargo => github_release_version(),
        InstallMethod::Winget => winget_cli_version(),
        InstallMethod::WindowsInstaller | InstallMethod::Unknown => github_release_version(),
    }
}

/// Resolve the URL for the GitHub releases-latest API endpoint.
fn github_releases_latest_url() -> String {
    format!("{}/releases/latest", agents::github_api_repo_url())
}

/// Bounded by `PACKAGE_MANAGER_TIMEOUT` — a hung `brew` / `winget` can't
/// stall startup. stderr/stdin are detached so misbehaving PMs can't write
/// to the user's terminal mid-startup.
fn package_manager_stdout(program: &str, args: &[&str], envs: &[(&str, &str)]) -> Option<String> {
    let mut stdout_file = tempfile::tempfile().ok()?;
    let stdout_for_child = stdout_file.try_clone().ok()?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::from(stdout_for_child))
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().ok()?;

    let status = match child.wait_timeout(PACKAGE_MANAGER_TIMEOUT).ok()? {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    let mut stdout_bytes = Vec::new();
    stdout_file.seek(SeekFrom::Start(0)).ok()?;
    stdout_file.read_to_end(&mut stdout_bytes).ok()?;
    if !status.success() && stdout_bytes.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&stdout_bytes).to_string())
}

fn homebrew_cli_version() -> Option<String> {
    let body = package_manager_stdout(
        "brew",
        &["outdated", "--json=v2", "hyprlayer"],
        &[("HOMEBREW_NO_AUTO_UPDATE", "1")],
    )?;
    parse_homebrew_outdated(&body)
        .map(|v| v.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()))
}

#[derive(Deserialize)]
struct HomebrewOutdated {
    #[serde(default)]
    formulae: Vec<HomebrewFormula>,
}

#[derive(Deserialize)]
struct HomebrewFormula {
    name: String,
    current_version: Option<String>,
}

fn parse_homebrew_outdated(body: &str) -> Option<Option<String>> {
    let outdated: HomebrewOutdated = serde_json::from_str(body).ok()?;
    let latest = outdated
        .formulae
        .into_iter()
        .find(|f| f.name == "hyprlayer" || f.name.ends_with("/hyprlayer"))
        .and_then(|f| f.current_version);
    match latest {
        Some(v) if is_semverish(&v) => Some(Some(v)),
        Some(_) => None,
        None => Some(None),
    }
}

fn scoop_cli_version() -> Option<String> {
    let body = package_manager_stdout("scoop", &["cat", "hyprlayer"], &[])?;
    parse_scoop_manifest(&body)
}

#[derive(Deserialize)]
struct ScoopManifest {
    version: String,
}

fn parse_scoop_manifest(body: &str) -> Option<String> {
    let manifest: ScoopManifest = serde_json::from_str(body).ok()?;
    if is_semverish(&manifest.version) {
        Some(manifest.version)
    } else {
        None
    }
}

fn aur_helper_version() -> Option<String> {
    for helper in ["paru", "yay"] {
        let Some(body) = package_manager_stdout(helper, &["-Qua", "hyprlayer-bin"], &[]) else {
            continue;
        };
        return Some(
            parse_aur_helper_outdated(&body)
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        );
    }
    None
}

fn parse_aur_helper_outdated(body: &str) -> Option<String> {
    body.lines()
        .filter(|line| line.contains("hyprlayer-bin"))
        .find_map(|line| line.split_once("->").map(|(_, after)| after.trim()))
        .and_then(|after| after.split_whitespace().next())
        .and_then(strip_aur_pkgrel)
}

fn strip_aur_pkgrel(raw: &str) -> Option<String> {
    let base = match raw.rsplit_once('-') {
        Some((v, _)) => v,
        None => raw,
    };
    if is_semverish(base) {
        Some(base.to_string())
    } else {
        None
    }
}

fn github_release_version() -> Option<String> {
    let body = http::get_text(&github_releases_latest_url(), VERSION_SOURCE_TIMEOUT).ok()?;
    parse_github_release(&body)
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

fn parse_github_release(body: &str) -> Option<String> {
    let release: GitHubRelease = serde_json::from_str(body).ok()?;
    let v = release.tag_name.trim_start_matches('v').to_string();
    if is_semverish(&v) { Some(v) } else { None }
}

fn winget_cli_version() -> Option<String> {
    let body = package_manager_stdout(
        "winget",
        &[
            "upgrade",
            "--id",
            "BrightBlock.Hyprlayer",
            "--exact",
            "--disable-interactivity",
            "--accept-source-agreements",
        ],
        &[],
    )?;
    Some(parse_winget_upgrade(&body).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()))
}

fn parse_winget_upgrade(body: &str) -> Option<String> {
    body.lines()
        .find(|line| line.contains("BrightBlock.Hyprlayer"))
        .and_then(|line| line.split_whitespace().rfind(|token| is_semverish(token)))
        .map(str::to_string)
}

/// Two-or-more dot-separated all-digit segments, optional `-suffix`.
fn is_semverish(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let base = s.split('-').next().unwrap_or(s);
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_homebrew_outdated_json() {
        let body = r#"{
            "formulae": [
                { "name": "hyprlayer", "installed_versions": ["1.5.5"], "current_version": "1.6.0" }
            ],
            "casks": []
        }"#;
        assert_eq!(
            parse_homebrew_outdated(body),
            Some(Some("1.6.0".to_string()))
        );
    }

    #[test]
    fn homebrew_empty_outdated_json_means_no_update() {
        assert_eq!(
            parse_homebrew_outdated(r#"{ "formulae": [], "casks": [] }"#),
            Some(None)
        );
    }

    #[test]
    fn homebrew_malformed_outdated_json_returns_none() {
        assert_eq!(parse_homebrew_outdated(""), None);
        assert_eq!(parse_homebrew_outdated("not json"), None);
    }

    #[test]
    fn parses_scoop_manifest() {
        let body = r#"{ "version": "1.6.0", "url": "https://example.com/hyprlayer.zip" }"#;
        assert_eq!(parse_scoop_manifest(body), Some("1.6.0".to_string()));
    }

    #[test]
    fn scoop_malformed_input_returns_none() {
        assert_eq!(parse_scoop_manifest(""), None);
        assert_eq!(parse_scoop_manifest("not json"), None);
        assert_eq!(parse_scoop_manifest(r#"{ "url": "x" }"#), None);
        assert_eq!(parse_scoop_manifest(r#"{ "version": "HEAD" }"#), None);
    }

    #[test]
    fn parses_aur_helper_outdated_output() {
        let body = "aur/hyprlayer-bin 1.5.5-1 -> 1.6.0-1\n";
        assert_eq!(parse_aur_helper_outdated(body), Some("1.6.0".to_string()));
    }

    #[test]
    fn aur_strips_only_trailing_pkgrel() {
        assert_eq!(
            strip_aur_pkgrel("1.6.0-rc1-2"),
            Some("1.6.0-rc1".to_string())
        );
    }

    #[test]
    fn aur_helper_empty_or_malformed_output_returns_none() {
        assert_eq!(parse_aur_helper_outdated(""), None);
        assert_eq!(parse_aur_helper_outdated("other-package 1 -> 2"), None);
        assert_eq!(parse_aur_helper_outdated("hyprlayer-bin garbage"), None);
    }

    #[test]
    fn parses_github_release() {
        let body = r#"{ "tag_name": "v1.6.0", "name": "v1.6.0" }"#;
        assert_eq!(parse_github_release(body), Some("1.6.0".to_string()));
    }

    #[test]
    fn github_release_accepts_no_v_prefix() {
        let body = r#"{ "tag_name": "1.6.0" }"#;
        assert_eq!(parse_github_release(body), Some("1.6.0".to_string()));
    }

    #[test]
    fn github_release_malformed_input_returns_none() {
        assert_eq!(parse_github_release(""), None);
        assert_eq!(parse_github_release("not json"), None);
        assert_eq!(parse_github_release(r#"{ "tag_name": "" }"#), None);
        assert_eq!(parse_github_release(r#"{ "tag_name": "nightly" }"#), None);
    }

    #[test]
    fn parses_winget_upgrade_table() {
        let body = r#"
Name      Id                    Version Available Source
--------------------------------------------------------
Hyprlayer BrightBlock.Hyprlayer 1.5.5   1.6.0     winget
"#;
        assert_eq!(parse_winget_upgrade(body), Some("1.6.0".to_string()));
    }

    #[test]
    fn winget_no_update_output_returns_none() {
        assert_eq!(parse_winget_upgrade("No available upgrade found."), None);
        assert_eq!(
            parse_winget_upgrade("Other App Other.App 1.0.0 2.0.0 winget"),
            None
        );
    }

    #[test]
    fn cargo_uses_github_release_parser() {
        assert_eq!(
            parse_github_release(r#"{ "tag_name": "v1.6.0" }"#),
            Some("1.6.0".to_string())
        );
    }

    #[test]
    fn is_semverish_truth_table() {
        assert!(is_semverish("1.6.0"));
        assert!(is_semverish("1.6"));
        assert!(is_semverish("10.20.30"));
        assert!(is_semverish("1.6.0-rc1"));
        assert!(!is_semverish(""));
        assert!(!is_semverish("HEAD"));
        assert!(!is_semverish("1"));
        assert!(!is_semverish("1."));
        assert!(!is_semverish(".1"));
        assert!(!is_semverish("1.x"));
        assert!(!is_semverish("v1.6.0"));
    }

    #[cfg(unix)]
    #[test]
    fn package_manager_stdout_handles_large_output_without_pipe_deadlock() {
        let body = package_manager_stdout(
            "sh",
            &[
                "-c",
                "i=0; while [ \"$i\" -lt 30000 ]; do printf xxxxxxxxxx; i=$((i + 1)); done",
            ],
            &[],
        )
        .unwrap();
        assert_eq!(body.len(), 300_000);
    }
}
