use anyhow::Result;

use crate::cli::TelemetryInitArgs;
use crate::config::{KeySource, TelemetryMode};
use crate::telemetry::{identify, lifecycle};

pub fn init(args: TelemetryInitArgs) -> Result<()> {
    let TelemetryInitArgs {
        mode,
        api_key,
        org_id,
        config,
    } = args;
    let config_path = config.path()?;

    let mut cfg = config.load_or_default()?;
    let prior_mode = cfg.telemetry.mode;
    let new_mode: TelemetryMode = mode.into();

    // Overrides must land before enroll so the disclosure check sees the
    // post-override key source.
    if let Some(key) = api_key {
        cfg.telemetry.api_key = Some(key);
        cfg.telemetry.api_key_source = KeySource::Manual;
    }
    if let Some(org) = org_id {
        cfg.telemetry.org_id = Some(org);
    }

    lifecycle::enroll(&mut cfg);
    cfg.telemetry.mode = new_mode;
    cfg.save(&config_path)?;

    if new_mode == TelemetryMode::Identified && prior_mode != TelemetryMode::Identified {
        let _ = identify::record_identify(&cfg);
    }

    Ok(())
}
