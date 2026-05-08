use anyhow::Result;

use crate::cli::TelemetryConfigArgs;
use crate::config::{HyprlayerConfig, KeySource, TelemetryMode};
use crate::telemetry::lifecycle::{ResolveError, resolve_owner_repo};
use crate::telemetry::{org_config, unix_now};

pub fn config(args: TelemetryConfigArgs) -> Result<()> {
    let TelemetryConfigArgs {
        api_key,
        org_id,
        reset,
        refresh,
        show,
        config,
    } = args;
    let config_path = config.path()?;
    let mut cfg = config.load_or_default()?;

    if show {
        println!("{}", cfg.telemetry.api_key_source);
        return Ok(());
    }

    if refresh {
        run_refresh(&mut cfg, &config_path)?;
        return Ok(());
    }

    if reset {
        cfg.telemetry.api_key = None;
        cfg.telemetry.api_key_source = KeySource::Default;
        cfg.save(&config_path)?;
        return Ok(());
    }

    let mut mutated = false;
    if let Some(key) = api_key {
        cfg.telemetry.api_key = Some(key);
        cfg.telemetry.api_key_source = KeySource::Manual;
        mutated = true;
    }
    if let Some(org) = org_id {
        cfg.telemetry.org_id = Some(org);
        mutated = true;
    }
    if mutated {
        cfg.save(&config_path)?;
    }
    Ok(())
}

/// Force-pull HYPRLAYER_TELEMETRY_KEY / HYPRLAYER_ORG_ID from the thoughts
/// repo on GitHub. Differs from the auto-lifecycle refresh by printing
/// friendly diagnostics for every refusal path and demoting a stale
/// `Github` source back to `Default` when the org's variable has been
/// removed (so a rotated/removed org key auto-falls-back to community).
///
/// Refuses to run while telemetry is off — `gh`/`git` are exec surfaces
/// we promised the user we wouldn't touch until they explicitly opt in.
fn run_refresh(cfg: &mut HyprlayerConfig, config_path: &std::path::Path) -> Result<()> {
    if cfg.telemetry.mode == TelemetryMode::Off {
        eprintln!(
            "Telemetry is off; refresh would shell out to `gh`/`git`. Run \
             `hyprlayer telemetry on` first if you want to populate the \
             org-managed key."
        );
        return Ok(());
    }
    let owner_repo = match resolve_owner_repo(cfg) {
        Ok(r) => r,
        Err(ResolveError::Manual) => {
            eprintln!("Manual override is active. Run with --reset first to clear it.");
            return Ok(());
        }
        Err(ResolveError::NoBackend) => {
            eprintln!("No git-backend thoughts repo configured; nothing to refresh.");
            return Ok(());
        }
        Err(ResolveError::NotGithub) => {
            eprintln!("Thoughts repo origin is not a GitHub remote; falling back to default key.");
            return Ok(());
        }
    };

    if let Some(key) = org_config::fetch_telemetry_key(&owner_repo) {
        cfg.telemetry.api_key = Some(key);
        cfg.telemetry.api_key_source = KeySource::Github;
    } else {
        cfg.telemetry.api_key = None;
        if cfg.telemetry.api_key_source == KeySource::Github {
            cfg.telemetry.api_key_source = KeySource::Default;
        }
    }
    if let Some(org) = org_config::fetch_org_id(&owner_repo) {
        cfg.telemetry.org_id = Some(org);
    }
    cfg.telemetry.last_config_refresh = unix_now();
    cfg.save(config_path)?;
    Ok(())
}
