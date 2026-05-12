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
            None => serde_json::json!({"mode": "off", "configured": false, "locked": false}),
            Some(cfg) => {
                let (bytes, count) = spool::depth().unwrap_or((0, 0));
                serde_json::json!({
                    "mode": cfg.telemetry.mode.to_string(),
                    "source": cfg.telemetry.api_key_source.to_string(),
                    "locked": cfg.telemetry.is_locked(),
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

    let line = cfg
        .map(|c| {
            let mode = c.telemetry.mode.to_string();
            if c.telemetry.is_locked() {
                format!("{mode} (locked)")
            } else {
                mode
            }
        })
        .unwrap_or_else(|| "off".to_string());
    println!("{line}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{HyprlayerConfig, KeySource, TelemetryConfig, TelemetryMode};

    /// `status` itself prints to stdout, so we test the predicate
    /// that drives the JSON `locked` field.
    #[test]
    fn locked_field_truth_table() {
        let locked = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                api_key: Some("phc_corp".into()),
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(locked.telemetry.is_locked());

        let unlocked = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                api_key_source: KeySource::Default,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!unlocked.telemetry.is_locked());
    }
}
