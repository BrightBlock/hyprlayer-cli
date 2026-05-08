use anyhow::Result;

use crate::cli::TelemetryPurgeArgs;
use crate::telemetry::spool;

pub fn purge(_args: TelemetryPurgeArgs) -> Result<()> {
    spool::purge()?;
    Ok(())
}
