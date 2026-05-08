//! Manages `~/.config/opencode/plugins/hyprlayer-telemetry.ts`.
//! Installed on opencode + telemetry-recording configurations; removed
//! on opt-out so opted-out users don't pay the per-turn execFile cost
//! the plugin would otherwise no-op on `is_recording()`.
//!
//! The plugin source is fetched from the repo (master) on demand via
//! `agents::download_repo_file` — same mechanism `ai configure` uses
//! for the rest of the opencode bundle. No binary-embedded copy.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::agents::{AgentTool, download_repo_file};

const REPO_PATH: &str = "opencode/plugins/hyprlayer-telemetry.ts";
const PLUGIN_FILENAME: &str = "hyprlayer-telemetry.ts";

pub fn install_path() -> Result<PathBuf> {
    Ok(AgentTool::OpenCode
        .dest_dir()?
        .join("plugins")
        .join(PLUGIN_FILENAME))
}

pub fn install_at(path: &Path) -> Result<()> {
    download_repo_file(REPO_PATH, path)
}

pub fn uninstall_at(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

pub fn is_installed_at(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn uninstall_at_removes_plugin_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hyprlayer-telemetry.ts");
        std::fs::write(&path, "stub").unwrap();
        assert!(path.exists());
        uninstall_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn uninstall_at_on_missing_file_is_noop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.ts");
        uninstall_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn is_installed_at_returns_true_when_file_present() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plugin.ts");
        std::fs::write(&path, "stub").unwrap();
        assert!(is_installed_at(&path));
    }

    #[test]
    fn is_installed_at_returns_false_when_absent() {
        let dir = tempdir().unwrap();
        assert!(!is_installed_at(&dir.path().join("nope.ts")));
    }

    #[test]
    fn is_installed_at_returns_false_when_directory_at_path() {
        // A directory shadowing the plugin filename — `.is_file()`
        // must reject this, not return true on its existence.
        let dir = tempdir().unwrap();
        let path = dir.path().join("hyprlayer-telemetry.ts");
        std::fs::create_dir(&path).unwrap();
        assert!(!is_installed_at(&path));
    }

    #[test]
    fn install_path_includes_opencode_plugins_segment() {
        let path = install_path().unwrap();
        let s = path.to_string_lossy();
        assert!(s.contains(".config"), "expected .config in {s}");
        assert!(s.contains("opencode"), "expected opencode in {s}");
        assert!(s.contains("plugins"), "expected plugins in {s}");
        assert!(
            s.ends_with(PLUGIN_FILENAME),
            "expected to end with {PLUGIN_FILENAME}: {s}"
        );
    }
}
