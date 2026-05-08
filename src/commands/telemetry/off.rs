use anyhow::Result;

use crate::cli::TelemetryOffArgs;
use crate::commands::telemetry::{hook, opencode_plugin};
use crate::config::{HyprlayerConfig, TelemetryMode};

pub fn off(args: TelemetryOffArgs) -> Result<()> {
    let config_path = args.config.path()?;
    if !config_path.exists() {
        return Ok(());
    }
    let mut cfg = HyprlayerConfig::load(&config_path)?;
    cfg.telemetry.mode = TelemetryMode::Off;
    cfg.save(&config_path)?;

    // Best-effort: remove the Stop hook so opted-out users don't pay
    // the per-turn process spawn that would no-op on `is_recording()`.
    if let Ok(path) = hook::settings_path() {
        let _ = hook::uninstall_at(&path);
    }
    // Same reasoning for the opencode plugin: leaving it on disk would
    // fire `execFile hyprlayer telemetry skill-end` per turn just to
    // bail on `is_recording()`. Removing it stops the spawn cost; the
    // next `telemetry on` re-fetches via `download_repo_file`.
    if let Ok(path) = opencode_plugin::install_path() {
        let _ = opencode_plugin::uninstall_at(&path);
    }
    Ok(())
}
