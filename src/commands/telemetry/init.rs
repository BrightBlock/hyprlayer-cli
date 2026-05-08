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

    // Manual overrides land first so the org refresh sees the
    // post-override key source (Manual is sticky against auto-pull).
    if let Some(key) = api_key {
        cfg.telemetry.api_key = Some(key);
        cfg.telemetry.api_key_source = KeySource::Manual;
    }
    if let Some(org) = org_id {
        cfg.telemetry.org_id = Some(org);
    }

    // `init` takes an explicit `--mode`, so we skip `enroll`'s auto-elevate
    // path: we don't want a corporate cwd to silently flip a user-pinned
    // `--mode anonymous` to identified (and spool an identify event the
    // user didn't ask for). Pull the org-managed key, then pin the mode.
    let was_first_run = lifecycle::provision_identity(&mut cfg);
    let _ = lifecycle::refresh_org_config(&mut cfg);
    cfg.telemetry.mode = new_mode;
    cfg.save(&config_path)?;

    lifecycle::maybe_print_disclosure(&cfg, was_first_run);

    if new_mode == TelemetryMode::Identified && prior_mode != TelemetryMode::Identified {
        let _ = identify::record_identify(&cfg);
    }

    Ok(())
}
