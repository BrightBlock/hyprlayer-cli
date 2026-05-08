use anyhow::Result;

use crate::cli::TelemetryStatusArgs;
use crate::telemetry::spool;

/// Default output is one line: the resolved mode (`off` / `anonymous` /
/// `identified`). All the operational detail (installation_id, spool
/// depth, last flush timestamp) is `--json` only, so a casual
/// `hyprlayer telemetry status` doesn't surface internals to anyone who
/// happens to glance at the terminal.
pub fn status(args: TelemetryStatusArgs) -> Result<()> {
    let TelemetryStatusArgs { json, config } = args;
    let cfg = config.load_if_exists()?;

    if json {
        let payload = match cfg {
            None => serde_json::json!({"mode": "off", "configured": false}),
            Some(cfg) => {
                let (bytes, count) = spool::depth().unwrap_or((0, 0));
                serde_json::json!({
                    "mode": cfg.telemetry.mode.to_string(),
                    "source": cfg.telemetry.api_key_source.to_string(),
                    "installation_id": cfg.telemetry.installation_id,
                    "org_id": cfg.telemetry.org_id,
                    "spool_bytes": bytes,
                    "spool_count": count,
                    "last_flush": cfg.telemetry.last_flush,
                })
            }
        };
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let mode = cfg
        .map(|c| c.telemetry.mode.to_string())
        .unwrap_or_else(|| "off".to_string());
    println!("{mode}");
    Ok(())
}
