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

    let weakens_lock = matches!(new_mode, TelemetryMode::Anonymous) || api_key.is_some();
    if cfg.telemetry.is_locked() && weakens_lock {
        return Err(cfg.telemetry.locked_error());
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{ConfigArgs, TelemetryModeArg};
    use crate::config::{HyprlayerConfig, TelemetryConfig};
    use crate::telemetry::lifecycle::LockedError;

    fn locked_cfg() -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                installation_id: Some("id".into()),
                api_key: Some("phc_corp".into()),
                api_key_source: KeySource::Github,
                org_id: Some("acme".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn args(
        mode: TelemetryModeArg,
        api_key: Option<&str>,
        path: &std::path::Path,
    ) -> TelemetryInitArgs {
        TelemetryInitArgs {
            mode,
            api_key: api_key.map(str::to_string),
            org_id: None,
            config: ConfigArgs {
                config_file: Some(path.to_string_lossy().into_owned()),
            },
        }
    }

    fn write(cfg: &HyprlayerConfig) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        cfg.save(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn init_refuses_mode_anonymous_when_locked() {
        let (_dir, path) = write(&locked_cfg());
        let err = init(args(TelemetryModeArg::Anonymous, None, &path))
            .expect_err("expected locked refusal");
        assert!(err.downcast_ref::<LockedError>().is_some());
        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.mode, TelemetryMode::Identified);
    }

    #[test]
    fn init_refuses_api_key_override_when_locked() {
        let (_dir, path) = write(&locked_cfg());
        let err = init(args(
            TelemetryModeArg::Identified,
            Some("phc_attacker"),
            &path,
        ))
        .expect_err("expected locked refusal");
        assert!(err.downcast_ref::<LockedError>().is_some());
        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
        assert_eq!(reloaded.telemetry.api_key_source, KeySource::Github);
    }

    #[test]
    fn init_mode_identified_without_key_is_noop_when_locked() {
        let (_dir, path) = write(&locked_cfg());
        init(args(TelemetryModeArg::Identified, None, &path)).expect("should be a no-op pass");
        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.mode, TelemetryMode::Identified);
        assert_eq!(reloaded.telemetry.api_key_source, KeySource::Github);
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
    }
}
