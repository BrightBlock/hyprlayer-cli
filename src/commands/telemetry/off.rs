use anyhow::Result;

use crate::cli::TelemetryOffArgs;
use crate::commands::telemetry::hook;
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
    Ok(())
}
