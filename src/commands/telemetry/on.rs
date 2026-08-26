use anyhow::Result;

use crate::cli::TelemetryOnArgs;
use crate::commands::ai::install_claude_hook_if_applicable;
use crate::telemetry::lifecycle;

pub fn on(args: TelemetryOnArgs) -> Result<()> {
    let config_path = args.config.path()?;
    let mut cfg = args.config.load_or_default()?;

    lifecycle::enroll(&mut cfg, &config_path)?;

    // Claude is always provisioned, so restore its Stop hook on the
    // off → on round trip without consulting a selected harness.
    install_claude_hook_if_applicable(&cfg);
    Ok(())
}
