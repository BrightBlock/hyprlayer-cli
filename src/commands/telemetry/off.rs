use anyhow::Result;

use crate::cli::TelemetryOffArgs;
use crate::commands::telemetry::hook;
use crate::config::{HyprlayerConfig, TelemetryMode};
use crate::telemetry::generate_device_salt;

pub fn off(args: TelemetryOffArgs) -> Result<()> {
    let config_path = args.config.path()?;
    let mut cfg = if config_path.exists() {
        HyprlayerConfig::load(&config_path)?
    } else {
        HyprlayerConfig::default()
    };
    if cfg.telemetry.is_locked() {
        return Err(cfg.telemetry.locked_error());
    }
    // `installation_id` makes the next auto-enroll pass take
    // SkippedStickyOff instead of treating this as a pristine install.
    cfg.telemetry.ensure_ids(generate_device_salt);
    cfg.telemetry.mode = TelemetryMode::Off;
    cfg.save(&config_path)?;

    // Best-effort: remove the Stop hook so opted-out users don't pay
    // the per-turn process spawn that would no-op on `is_recording()`.
    if let Ok(path) = hook::settings_path() {
        let _ = hook::uninstall_at(&path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ConfigArgs;
    use crate::config::{KeySource, TelemetryConfig};
    use crate::telemetry::lifecycle::LockedError;

    fn locked_config_with_org(org_id: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                installation_id: Some("id".into()),
                api_key: Some("phc_corp".into()),
                api_key_source: KeySource::Github,
                org_id: org_id.map(str::to_string),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn off_refuses_when_locked_and_leaves_config_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = locked_config_with_org(Some("acme"));
        cfg.save(&path).unwrap();

        let args = TelemetryOffArgs {
            config: ConfigArgs {
                config_file: Some(path.to_string_lossy().into_owned()),
            },
        };
        let err = off(args).expect_err("expected locked refusal");
        assert!(
            err.downcast_ref::<LockedError>().is_some(),
            "expected LockedError"
        );
        let message = err.to_string();
        assert!(
            message.contains("acme"),
            "message should name the org: {message}"
        );

        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.mode, TelemetryMode::Identified);
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
    }

    /// Without a persisted `installation_id`, the next command's
    /// auto-enroll would treat this as a pristine install and enrol
    /// anonymous, silently losing the user's opt-out.
    #[test]
    fn off_on_missing_config_writes_sticky_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(!path.exists());

        let args = TelemetryOffArgs {
            config: ConfigArgs {
                config_file: Some(path.to_string_lossy().into_owned()),
            },
        };
        off(args).expect("off must succeed on missing config");

        let reloaded = HyprlayerConfig::load(&path).expect("config must now exist");
        assert_eq!(reloaded.telemetry.mode, TelemetryMode::Off);
        assert!(
            reloaded.telemetry.installation_id.is_some(),
            "installation_id must be provisioned so auto-enroll sees sticky-off"
        );
    }

    #[test]
    fn off_locked_error_omits_org_clause_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = locked_config_with_org(None);
        cfg.save(&path).unwrap();

        let args = TelemetryOffArgs {
            config: ConfigArgs {
                config_file: Some(path.to_string_lossy().into_owned()),
            },
        };
        let err = off(args).expect_err("expected locked refusal");
        assert!(err.downcast_ref::<LockedError>().is_some());
        let message = err.to_string();
        assert!(!message.contains("by `"), "{message}");
        assert!(
            message.contains("organization-managed settings"),
            "{message}"
        );
    }
}
