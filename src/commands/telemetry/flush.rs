use anyhow::Result;

use crate::cli::TelemetryFlushArgs;
use crate::config::HyprlayerConfig;
use crate::telemetry::{flush, unix_now};

pub fn flush_cmd(args: TelemetryFlushArgs) -> Result<()> {
    let config_path = args.config.path()?;
    let Some(cfg) = args.config.load_if_exists()? else {
        return Ok(());
    };

    match flush::flush(&cfg) {
        Ok(count) => {
            // Surface success before the throttle update — the events are
            // already gone from the spool, so a save failure below is not
            // a flush failure.
            if count > 0 {
                eprintln!("flushed {count} event(s)");
            }
            // Reload to narrow the clobber window: the cfg we loaded
            // before the HTTP POST is now seconds stale.
            let mut latest = HyprlayerConfig::load(&config_path)?;
            latest.telemetry.last_flush = unix_now();
            latest.save(&config_path)?;
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("flush failed: {e}")),
    }
}
