//! `hyprlayer self-update` dispatch and direct binary replacement.

use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::agents;
use crate::cli::SelfUpdateArgs;
use crate::http;
use crate::integrity;
use crate::version::{self, InstallMethod, UpdateInfo};
use crate::version_source;

/// Hard ceiling on the downloaded binary. Real builds sit ~25–35 MB; 200 MB
/// is plenty of headroom and stops a hostile/misconfigured source from
/// streaming gigabytes into the temp dir before we verify the digest.
const MAX_BINARY_BYTES: u64 = 200 * 1024 * 1024;

pub fn run(args: SelfUpdateArgs) -> Result<()> {
    let method = InstallMethod::detect();

    if args.check {
        report_update_status(method, args.force)?;
        return Ok(());
    }

    match method {
        InstallMethod::Homebrew => dispatch_pm("brew", &["upgrade", "hyprlayer"]),
        InstallMethod::Cargo => {
            let Some(info) = preflight_update_info(method, args.force)? else {
                return Ok(());
            };
            run_cargo(&info)
        }
        InstallMethod::Winget => dispatch_pm("winget", &["upgrade", "BrightBlock.Hyprlayer"]),
        InstallMethod::Scoop => dispatch_pm("scoop", &["update", "hyprlayer"]),
        InstallMethod::Aur => dispatch_aur(),
        InstallMethod::WindowsInstaller | InstallMethod::Unknown => {
            let Some(info) = preflight_update_info(method, args.force)? else {
                return Ok(());
            };
            direct_update(&info)?;
            println!("hyprlayer updated to {}.", info.latest);
            Ok(())
        }
    }
}

/// Startup auto-update path; callers fall back to notification on failure.
pub fn run_silent(info: &UpdateInfo) -> Result<()> {
    direct_update(info)
}

fn report_update_status(method: InstallMethod, force: bool) -> Result<()> {
    if let Some(info) = latest_update_info(method, force)? {
        println!(
            "Update available: {} → {} ({})",
            info.current,
            info.latest,
            method.upgrade_hint()
        );
    }
    Ok(())
}

fn latest_update_info(method: InstallMethod, force: bool) -> Result<Option<UpdateInfo>> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let latest = version_source::latest_available_for(&method).ok_or_else(|| {
        anyhow!(
            "Unable to determine the latest installable version for this install method ({method:?}). \
             The release source may be unreachable, or the package manager may not yet have this package."
        )
    })?;

    if !force && !version::is_newer_version(&latest, &current) {
        println!("hyprlayer is up to date ({}).", current);
        return Ok(None);
    }

    Ok(Some(UpdateInfo {
        current,
        latest,
        install_method: method,
    }))
}

fn preflight_update_info(method: InstallMethod, force: bool) -> Result<Option<UpdateInfo>> {
    debug_assert!(requires_release_preflight(method));
    latest_update_info(method, force)
}

fn requires_release_preflight(method: InstallMethod) -> bool {
    matches!(
        method,
        InstallMethod::Cargo | InstallMethod::WindowsInstaller | InstallMethod::Unknown
    )
}

fn run_cargo(info: &UpdateInfo) -> Result<()> {
    let tag = format!("v{}", info.latest);
    let repo_url = format!("https://github.com/{}", agents::REPO);
    let cmd = [
        "install",
        "--git",
        repo_url.as_str(),
        "--tag",
        tag.as_str(),
        "--force",
    ];
    dispatch_pm("cargo", &cmd)
}

fn dispatch_pm(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch `{program}`; is it on PATH?"))?;

    if !status.success() {
        let rendered = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        bail!("`{rendered}` exited with status {status}");
    }
    Ok(())
}

/// Try `paru` then `yay`; `pacman -U` would require building locally and is
/// the wrong fit for a "self-update" UX.
fn dispatch_aur() -> Result<()> {
    for helper in ["paru", "yay"] {
        if which(helper).is_some() {
            return dispatch_pm(helper, &["-S", "hyprlayer-bin"]);
        }
    }
    bail!(
        "No AUR helper found (looked for `paru`, `yay`). \
         Install via your AUR helper: `<helper> -S hyprlayer-bin`."
    );
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn direct_update(info: &UpdateInfo) -> Result<()> {
    let target = target_triple().ok_or_else(|| {
        anyhow!(
            "No precompiled binary is published for this target. \
             Build from source: https://github.com/{}",
            agents::REPO
        )
    })?;
    let asset = asset_name(target);
    let tag = format!("v{}", info.latest);
    let bin_url = format!("{}/{tag}/{asset}", agents::github_release_download_base());

    let api_url = format!("{}/releases/tags/{tag}", agents::github_api_repo_url());
    let release_body = http::get_text(&api_url, Duration::from_secs(15))
        .context("Unable to fetch release metadata from GitHub")?;
    let digests = integrity::digests_from_release_json(&release_body);
    let expected = digests.get(&asset).ok_or_else(|| {
        anyhow!(
            "GitHub release `{tag}` exposes no SHA256 digest for asset `{asset}`. \
             Refusing to swap an unverified binary."
        )
    })?;

    let tmp = tempfile::Builder::new()
        .prefix("hyprlayer-update-")
        .tempdir()
        .context("failed to create temp dir for download")?;
    let bin_path = tmp.path().join(&asset);

    eprintln!("Downloading {asset}…");
    http::download_file_capped(
        &bin_url,
        &bin_path,
        Duration::from_secs(120),
        Some(MAX_BINARY_BYTES),
    )
    .with_context(|| format!("Download failed: {bin_url}"))?;

    integrity::verify_sha256(&bin_path, expected)
        .with_context(|| format!("Integrity check failed for `{asset}`"))?;

    replace_current_exe(&bin_path).context("Atomic swap failed")?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_current_exe(bin_path: &Path) -> Result<()> {
    self_replace::self_replace(bin_path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_current_exe(bin_path: &Path) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?;
    let backup = dir.join(format!(
        ".hyprlayer-{}-old.exe",
        uuid::Uuid::new_v4().simple()
    ));
    let staged = dir.join(format!(
        ".hyprlayer-{}-new.exe",
        uuid::Uuid::new_v4().simple()
    ));

    std::fs::copy(bin_path, &staged)
        .with_context(|| format!("failed to stage replacement beside `{}`", exe.display()))?;

    if let Err(e) = std::fs::rename(&exe, &backup) {
        let _ = std::fs::remove_file(&staged);
        return Err(e)
            .with_context(|| format!("failed to preserve old executable `{}`", exe.display()));
    }

    if let Err(replace_err) = std::fs::rename(&staged, &exe) {
        let rollback = std::fs::rename(&backup, &exe);
        let _ = std::fs::remove_file(&staged);
        return match rollback {
            Ok(()) => Err(replace_err)
                .with_context(|| format!("failed to move replacement into `{}`", exe.display())),
            Err(rollback_err) => Err(anyhow!(
                "failed to move replacement into `{}` ({replace_err}); rollback also failed ({rollback_err}); old executable remains at `{}`",
                exe.display(),
                backup.display()
            )),
        };
    }

    let _ = self_replace::self_delete_at(&backup);
    Ok(())
}

fn target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn asset_name(target: &str) -> String {
    if target.contains("windows") {
        format!("hyprlayer-{target}.exe")
    } else {
        format!("hyprlayer-{target}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_matches_release_yml_matrix() {
        assert_eq!(
            asset_name("x86_64-unknown-linux-gnu"),
            "hyprlayer-x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            asset_name("aarch64-unknown-linux-gnu"),
            "hyprlayer-aarch64-unknown-linux-gnu"
        );
        assert_eq!(
            asset_name("aarch64-apple-darwin"),
            "hyprlayer-aarch64-apple-darwin"
        );
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc"),
            "hyprlayer-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn target_triple_matches_host_target() {
        let got = target_triple();
        if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            assert!(got.is_none(), "Intel macOS is unsupported");
        } else if cfg!(target_os = "linux")
            || cfg!(target_os = "macos")
            || cfg!(target_os = "windows")
        {
            assert!(got.is_some(), "must publish for this host's target");
        }
    }

    #[test]
    fn which_finds_a_known_binary() {
        #[cfg(unix)]
        {
            assert!(which("sh").is_some());
        }
        assert!(which("nonexistent-binary-that-no-one-installs-zzzz").is_none());
    }

    #[test]
    fn only_direct_release_methods_preflight_version_metadata() {
        assert!(requires_release_preflight(InstallMethod::Cargo));
        assert!(requires_release_preflight(InstallMethod::WindowsInstaller));
        assert!(requires_release_preflight(InstallMethod::Unknown));

        assert!(!requires_release_preflight(InstallMethod::Homebrew));
        assert!(!requires_release_preflight(InstallMethod::Winget));
        assert!(!requires_release_preflight(InstallMethod::Scoop));
        assert!(!requires_release_preflight(InstallMethod::Aur));
    }
}
