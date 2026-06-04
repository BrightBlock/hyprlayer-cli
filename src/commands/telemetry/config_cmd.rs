use anyhow::Result;

use crate::cli::TelemetryConfigArgs;
use crate::config::{HyprlayerConfig, KeySource, TelemetryMode};
use crate::telemetry::lifecycle::{ResolveError, auto_elevate_if_org_keyed, resolve_owner_repo};
use crate::telemetry::verbose::vlog;
use crate::telemetry::{identify, org_config, unix_now};

pub fn config(args: TelemetryConfigArgs) -> Result<()> {
    let TelemetryConfigArgs {
        api_key,
        org_id,
        reset,
        refresh,
        show,
        verbose,
        config,
    } = args;
    if verbose {
        crate::telemetry::verbose::set_enabled(true);
    }
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

    if cfg.telemetry.is_locked() && (api_key.is_some() || reset) {
        return Err(cfg.telemetry.locked_error());
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
/// repo on GitHub.
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
        Ok(r) => {
            vlog!("resolved thoughts-repo GitHub origin: {r}");
            r
        }
        Err(ResolveError::Manual) => {
            eprintln!("Manual override is active. Run with --reset first to clear it.");
            return Ok(());
        }
        Err(ResolveError::NoBackend) => {
            eprintln!("No git-backend thoughts repo configured; nothing to refresh.");
            demote_stale_github_key(cfg, config_path)?;
            return Ok(());
        }
        Err(ResolveError::NotGithub) => {
            eprintln!("Thoughts repo origin is not a GitHub remote; falling back to default key.");
            demote_stale_github_key(cfg, config_path)?;
            return Ok(());
        }
    };

    if let Some(key) = org_config::fetch_telemetry_key(&owner_repo) {
        cfg.telemetry.api_key = Some(key);
        cfg.telemetry.api_key_source = KeySource::Github;
        // Refresh org tag only while the org-managed key is in effect.
        if let Some(org) = org_config::fetch_org_id(&owner_repo) {
            cfg.telemetry.org_id = Some(org);
        }
        vlog!("org-managed key applied; telemetry is now identified for {owner_repo}");
    } else {
        // Only classify (an extra `gh variable list` shell-out) when the
        // operator asked to see the detail. A readable repo with no key is
        // the expected personal-repo case — not a failure.
        if crate::telemetry::verbose::is_enabled() {
            use org_config::VariableAccess;
            match org_config::repo_variables_access(&owner_repo) {
                VariableAccess::Readable => vlog!(
                    "{owner_repo} has no HYPRLAYER_TELEMETRY_KEY set — that's fine; staying \
                     on the default community key (anonymous)."
                ),
                VariableAccess::PermissionDenied(detail) => vlog!(
                    "{owner_repo} is visible but this `gh` account can't read its Actions \
                     variables (HTTP 403): {detail}. Org-managed telemetry needs the org to \
                     grant variable access; staying anonymous meanwhile."
                ),
                VariableAccess::NotFound(detail) => vlog!(
                    "{owner_repo} returned HTTP 404 ({detail}). Check the remote URL and your \
                     `gh` access; staying on the default community key (anonymous)."
                ),
                VariableAccess::OtherError(detail) => vlog!(
                    "could not read {owner_repo}'s Actions variables: {detail}. Staying \
                     anonymous."
                ),
                VariableAccess::GhMissing => vlog!(
                    "`gh` is not installed, so {owner_repo}'s org key can't be read; staying \
                     on the default community key (anonymous)."
                ),
            }
        }
        release_lock(cfg);
    }
    cfg.telemetry.last_config_refresh = unix_now();
    let mode_elevated = auto_elevate_if_org_keyed(cfg);

    cfg.save(config_path)?;
    if mode_elevated {
        let _ = identify::record_identify(cfg);
    }
    Ok(())
}

fn demote_stale_github_key(cfg: &mut HyprlayerConfig, config_path: &std::path::Path) -> Result<()> {
    if cfg.telemetry.api_key_source != KeySource::Github {
        return Ok(());
    }
    release_lock(cfg);
    cfg.save(config_path)?;
    Ok(())
}

fn release_lock(cfg: &mut HyprlayerConfig) {
    cfg.telemetry.api_key = None;
    cfg.telemetry.api_key_source = KeySource::Default;
    cfg.telemetry.org_id = None;
    if cfg.telemetry.mode == TelemetryMode::Identified {
        cfg.telemetry.mode = TelemetryMode::Anonymous;
    }
    cfg.telemetry.last_config_refresh = unix_now();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TelemetryConfig;
    use crate::telemetry::lifecycle::LockedError;
    use tempfile::tempdir;

    fn cfg_with_github_key() -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".into()),
                api_key: Some("phc_org".into()),
                api_key_source: KeySource::Github,
                org_id: Some("org-1".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn demote_clears_github_key_and_resets_source() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = cfg_with_github_key();
        cfg.save(&path).unwrap();

        demote_stale_github_key(&mut cfg, &path).unwrap();

        assert!(cfg.telemetry.api_key.is_none());
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Default);
        assert!(cfg.telemetry.org_id.is_none(), "stale org_id must clear");
        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert!(reloaded.telemetry.api_key.is_none());
        assert_eq!(reloaded.telemetry.api_key_source, KeySource::Default);
        assert!(reloaded.telemetry.org_id.is_none());
    }

    #[test]
    fn release_lock_demotes_identified_to_anonymous() {
        let mut cfg = locked_cfg();
        release_lock(&mut cfg);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Default);
        assert!(cfg.telemetry.api_key.is_none());
        assert!(cfg.telemetry.org_id.is_none());
    }

    /// Release must demote, never promote: an `Anonymous`+Github
    /// cached state (rare — explicit `init --mode anonymous` before
    /// the lock landed) stays Anonymous.
    #[test]
    fn release_lock_preserves_non_identified_mode() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".into()),
                api_key: Some("phc_corp".into()),
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            ..Default::default()
        };
        release_lock(&mut cfg);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
    }

    #[test]
    fn demote_is_noop_when_source_is_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".into()),
                api_key_source: KeySource::Default,
                last_config_refresh: 12345,
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.save(&path).unwrap();

        demote_stale_github_key(&mut cfg, &path).unwrap();
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Default);
        assert_eq!(cfg.telemetry.last_config_refresh, 12345);
    }

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

    fn build_args(
        path: &std::path::Path,
        api_key: Option<&str>,
        org_id: Option<&str>,
        reset: bool,
        refresh: bool,
    ) -> TelemetryConfigArgs {
        TelemetryConfigArgs {
            api_key: api_key.map(str::to_string),
            org_id: org_id.map(str::to_string),
            reset,
            refresh,
            show: false,
            verbose: false,
            config: crate::cli::ConfigArgs {
                config_file: Some(path.to_string_lossy().into_owned()),
            },
        }
    }

    #[test]
    fn config_refuses_api_key_override_when_locked() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        locked_cfg().save(&path).unwrap();

        let err = config(build_args(&path, Some("phc_attacker"), None, false, false))
            .expect_err("expected locked refusal");
        assert!(
            err.downcast_ref::<LockedError>().is_some(),
            "expected LockedError"
        );

        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
        assert_eq!(reloaded.telemetry.api_key_source, KeySource::Github);
    }

    #[test]
    fn config_refuses_reset_when_locked() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        locked_cfg().save(&path).unwrap();

        let err = config(build_args(&path, None, None, true, false))
            .expect_err("expected locked refusal");
        assert!(err.downcast_ref::<LockedError>().is_some());

        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
    }

    #[test]
    fn config_org_id_alone_passes_when_locked() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        locked_cfg().save(&path).unwrap();

        config(build_args(&path, None, Some("acme-2"), false, false))
            .expect("--org-id alone must not trigger the lock");
        let reloaded = HyprlayerConfig::load(&path).unwrap();
        assert_eq!(reloaded.telemetry.org_id.as_deref(), Some("acme-2"));
        assert_eq!(reloaded.telemetry.api_key_source, KeySource::Github);
        assert_eq!(reloaded.telemetry.api_key.as_deref(), Some("phc_corp"));
    }

    /// `--refresh` is the org's release path and must NOT be blocked.
    /// `NoBackend` short-circuits the gh shellout in this unit test;
    /// we're only asserting the lock guard didn't fire.
    #[test]
    fn config_refresh_not_blocked_by_lock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        locked_cfg().save(&path).unwrap();

        let result = config(build_args(&path, None, None, false, true));
        assert!(
            result.is_ok(),
            "refresh must not be lock-blocked: {result:?}"
        );
    }

    #[test]
    fn demote_preserves_manual_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".into()),
                api_key: Some("phc_manual".into()),
                api_key_source: KeySource::Manual,
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.save(&path).unwrap();

        demote_stale_github_key(&mut cfg, &path).unwrap();
        assert_eq!(cfg.telemetry.api_key.as_deref(), Some("phc_manual"));
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Manual);
    }
}
