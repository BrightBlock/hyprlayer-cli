//! Manages `~/.config/opencode/plugins/hyprlayer-telemetry.ts`.
//! Installed on opencode + telemetry-recording configurations; removed
//! on opt-out so opted-out users don't pay the per-turn execFile cost
//! the plugin would otherwise no-op on `is_recording()`.
//!
//! The plugin source is fetched on demand rather than embedded in the
//! binary: from the release-asset bundle the installed opencode files came
//! from when there is one, and otherwise from the frozen `opencode/` tree
//! on master — the same two sources `ai configure` installs the rest of the
//! bundle from.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::agents::{AgentTool, download_repo_file, install_bundled_file, installed_manifest};

/// Where the plugin lives inside a release-asset bundle, relative to the
/// harness root — which is also where it lands under the opencode config
/// dir, so a normal `ai configure` installs it as part of the sync.
const BUNDLE_PATH: &str = "plugins/hyprlayer-telemetry.ts";
/// The same file in the frozen legacy tree, for installs that came from
/// there and so have no bundle to read.
const REPO_PATH: &str = "opencode/plugins/hyprlayer-telemetry.ts";
const PLUGIN_FILENAME: &str = "hyprlayer-telemetry.ts";

pub fn install_path() -> Result<PathBuf> {
    Ok(AgentTool::OpenCode
        .dest_dir()?
        .join("plugins")
        .join(PLUGIN_FILENAME))
}

/// Restore the plugin file. The plugin talks to the CLI and to the skills
/// around it, so it has to be the copy from the *installed* bundle: pulling
/// it from `master` would hand a pinned or version-matched install a plugin
/// from whatever the branch happens to hold today.
///
/// Only an install with no manifest — from the frozen legacy tree, or by a
/// pre-manifest CLI — falls back to `master`, where the frozen
/// `opencode/plugins/` copy is exactly what that install received. A bundle
/// install whose fetch fails errors instead: a mismatched plugin is worse
/// than a missing one, which the next `ai configure` restores.
pub fn install_at(path: &Path) -> Result<()> {
    match installed_manifest(AgentTool::OpenCode) {
        Some(manifest) => install_bundled_file(AgentTool::OpenCode, &manifest, BUNDLE_PATH, path),
        None => download_repo_file(REPO_PATH, path),
    }
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

    /// `install_at` looks this path up in the installed bundle's manifest,
    /// so a move inside `assets/opencode/` would only show up as a failed
    /// plugin restore at runtime.
    #[test]
    fn bundle_path_names_a_file_the_live_opencode_tree_ships() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/opencode")
            .join(BUNDLE_PATH);
        assert!(path.is_file(), "no live plugin at {}", path.display());
        assert!(BUNDLE_PATH.ends_with(PLUGIN_FILENAME));
    }

    /// The fallback source has to be the same file in the frozen tree —
    /// that is what a legacy install actually received.
    #[test]
    fn repo_path_is_the_frozen_trees_copy_of_the_bundle_path() {
        assert_eq!(REPO_PATH, format!("opencode/{BUNDLE_PATH}"));
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(REPO_PATH);
        assert!(path.is_file(), "no frozen plugin at {}", path.display());
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
