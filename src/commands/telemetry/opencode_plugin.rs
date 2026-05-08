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
const BEACON_START: &str = "<!-- hyprlayer:telemetry-beacon -->";
const BEACON_END: &str = "<!-- /hyprlayer:telemetry-beacon -->";

pub fn install_path() -> Result<PathBuf> {
    Ok(AgentTool::OpenCode
        .dest_dir()?
        .join("plugins")
        .join(PLUGIN_FILENAME))
}

/// `~/.config/opencode/commands/` — the directory the legacy-beacon
/// migration scrubs in-place during plugin install.
fn commands_dir() -> Result<PathBuf> {
    Ok(AgentTool::OpenCode.dest_dir()?.join("commands"))
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

/// Strip pre-1.5.4 inline beacon blocks from `~/.config/opencode/
/// commands/*.md`. Pre-1.5.4 bundles asked the model to invoke
/// `hyprlayer telemetry skill-start/skill-end` inline; now that the TS
/// plugin owns that emission, leaving those blocks on disk would
/// double-fire on platforms where the model can shell out (Linux,
/// macOS terminal-launched opencode). Idempotent: a no-op for users
/// whose bundle has already been refreshed past the strip.
///
/// Best-effort: a perms error or a malformed file logs to stderr and
/// is otherwise silently skipped. The plugin install never fails on a
/// migration error — we'd rather have telemetry running with stale
/// beacons than no plugin at all.
pub fn strip_legacy_beacons_in_commands_dir() {
    let Ok(dir) = commands_dir() else {
        return;
    };
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(stripped) = strip_beacon_blocks(&contents)
            && let Err(e) = std::fs::write(&path, stripped)
        {
            eprintln!(
                "warning: could not strip legacy telemetry beacon in {}: {}",
                path.display(),
                e
            );
        }
    }
}

/// Pure helper for `strip_legacy_beacons_in_commands_dir`. Returns
/// `Some(new_contents)` when at least one beacon block was found and
/// removed, `None` when the input was already clean (so the caller
/// can skip the disk write). Handles multiple blocks per file and
/// drops up to two trailing newlines after each block (matching the
/// `perl -0pe 's/.../\n\n?//gs'` strip we shipped in the bundle).
fn strip_beacon_blocks(contents: &str) -> Option<String> {
    if !contents.contains(BEACON_START) {
        return None;
    }
    let mut out = String::with_capacity(contents.len());
    let mut remaining = contents;
    let mut changed = false;
    while let Some(start_idx) = remaining.find(BEACON_START) {
        let Some(end_offset) = remaining[start_idx..].find(BEACON_END) else {
            // Unmatched start — leave the rest verbatim. The next
            // bundle pull will reconcile.
            break;
        };
        let mut after_end = start_idx + end_offset + BEACON_END.len();
        if remaining[after_end..].starts_with('\n') {
            after_end += 1;
        }
        if remaining[after_end..].starts_with('\n') {
            after_end += 1;
        }
        out.push_str(&remaining[..start_idx]);
        remaining = &remaining[after_end..];
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(remaining);
    Some(out)
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

    #[test]
    fn strip_beacon_blocks_returns_none_when_clean() {
        // Already-stripped commands hit this every install — must
        // return None so the caller skips the disk write.
        let input = "---\nfoo: bar\n---\n# Title\n\nbody\n";
        assert_eq!(strip_beacon_blocks(input), None);
    }

    #[test]
    fn strip_beacon_blocks_removes_single_block_with_trailing_blank() {
        let input = "---\ndescription: x\n---\n\n<!-- hyprlayer:telemetry-beacon -->\nrun skill-start...\n<!-- /hyprlayer:telemetry-beacon -->\n\n# Title\n\nbody\n";
        let stripped = strip_beacon_blocks(input).expect("should strip");
        assert!(!stripped.contains("hyprlayer:telemetry-beacon"));
        assert!(stripped.contains("# Title"));
        assert!(stripped.contains("description: x"));
    }

    #[test]
    fn strip_beacon_blocks_removes_multiple_blocks() {
        // Defensive: a corrupted bundle could plausibly have stacked
        // copies. Strip all of them, not just the first.
        let input = "---\nfoo\n---\n<!-- hyprlayer:telemetry-beacon -->\na\n<!-- /hyprlayer:telemetry-beacon -->\n\nmiddle\n\n<!-- hyprlayer:telemetry-beacon -->\nb\n<!-- /hyprlayer:telemetry-beacon -->\n\nend\n";
        let stripped = strip_beacon_blocks(input).expect("should strip");
        assert!(!stripped.contains("hyprlayer:telemetry-beacon"));
        assert!(stripped.contains("middle"));
        assert!(stripped.contains("end"));
    }

    #[test]
    fn strip_beacon_blocks_handles_unmatched_start() {
        // A truncated/corrupted file with only a start marker — leave
        // the rest verbatim; the next bundle pull reconciles.
        let input = "---\n---\n<!-- hyprlayer:telemetry-beacon -->\nno end marker\n# Title\n";
        assert_eq!(strip_beacon_blocks(input), None);
    }

    #[test]
    fn strip_beacon_blocks_preserves_surrounding_content() {
        let input = "BEFORE\n<!-- hyprlayer:telemetry-beacon -->\nbeacon body\n<!-- /hyprlayer:telemetry-beacon -->\nAFTER\n";
        let stripped = strip_beacon_blocks(input).expect("should strip");
        assert!(stripped.starts_with("BEFORE"));
        assert!(stripped.ends_with("AFTER\n"));
        assert!(!stripped.contains("beacon body"));
    }

    #[test]
    fn strip_beacon_blocks_handles_no_trailing_newline() {
        // Block at exact end of file with no trailing newline. Must
        // not panic; output must drop the whole block.
        let input = "BEFORE\n<!-- hyprlayer:telemetry-beacon -->\nbody\n<!-- /hyprlayer:telemetry-beacon -->";
        let stripped = strip_beacon_blocks(input).expect("should strip");
        assert_eq!(stripped, "BEFORE\n");
    }

    #[test]
    fn strip_legacy_beacons_in_commands_dir_skips_when_dir_missing() {
        // The function itself uses the user's $HOME, so we can't
        // sandbox it cleanly without env mutation. Verify it returns
        // (no panic) on systems where ~/.config/opencode/commands/
        // doesn't exist — the most common state for non-opencode users.
        // The is_dir() guard short-circuits.
        strip_legacy_beacons_in_commands_dir();
    }
}
