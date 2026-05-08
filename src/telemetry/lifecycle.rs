//! Lifecycle hooks for telemetry, called from `hyprlayer ai configure`,
//! `hyprlayer thoughts init`, and the once-per-day startup background
//! pass. These are best-effort side effects against an *already-enrolled*
//! installation — they do not opt the user in.
//!
//! Enrollment lives in the explicit `telemetry on` / `telemetry init`
//! commands. A user who has never run those keeps `mode == Off` and
//! `installation_id == None`; lifecycle hooks short-circuit and do
//! nothing. Once they opt in, lifecycle calls run `refresh_org_config` to
//! pick up `HYPRLAYER_TELEMETRY_KEY` / `HYPRLAYER_ORG_ID` from the
//! thoughts repo's GitHub origin.

use std::path::Path;

use anyhow::Result;

use crate::config::{HyprlayerConfig, KeySource, TelemetryMode};

use super::{disclosure, generate_device_salt, org_config, unix_now};

/// Why `resolve_owner_repo` couldn't produce an `<owner>/<repo>` pair.
/// Narrow on purpose so the explicit `--refresh` handler can match
/// exhaustively without `unreachable!` arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    Manual,
    NoBackend,
    NotGithub,
}

/// Outcome of one `gh variable get` cycle. Caller decides whether to print
/// user-facing diagnostics, save, or just record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// Telemetry hasn't been enabled — nothing to refresh against.
    SkippedNotOptedIn,
    /// Manual override is sticky; no fetch attempted.
    SkippedManual,
    /// No git-backend thoughts repo configured.
    SkippedNoBackend,
    /// Thoughts repo origin isn't a GitHub remote.
    SkippedNotGithub,
    /// Fetch ran. `mutated` is true if config state actually changed.
    Fetched { mutated: bool },
}

impl From<ResolveError> for RefreshOutcome {
    fn from(e: ResolveError) -> Self {
        match e {
            ResolveError::Manual => RefreshOutcome::SkippedManual,
            ResolveError::NoBackend => RefreshOutcome::SkippedNoBackend,
            ResolveError::NotGithub => RefreshOutcome::SkippedNotGithub,
        }
    }
}

/// Resolve `<owner>/<repo>` for a `gh variable get` against the user's
/// thoughts repo, or report which preflight check skipped the lookup.
/// Shared by the auto-refresh path and the explicit `telemetry config
/// --refresh` CLI handler.
pub fn resolve_owner_repo(config: &HyprlayerConfig) -> Result<String, ResolveError> {
    if config.telemetry.api_key_source == KeySource::Manual {
        return Err(ResolveError::Manual);
    }
    let path = config
        .thoughts
        .as_ref()
        .and_then(|t| t.backend.thoughts_repo_path())
        .ok_or(ResolveError::NoBackend)?;
    let (owner, repo) = org_config::discover_github_remote(&path).ok_or(ResolveError::NotGithub)?;
    Ok(format!("{owner}/{repo}"))
}

/// Run org-config refresh, save once if anything changed. Skipped entirely
/// when telemetry isn't enabled — no `gh`/`git` shellouts for opted-out
/// users. Never enrolls the user.
pub fn apply_side_effects(config: &mut HyprlayerConfig, config_path: &Path) -> Result<()> {
    let RefreshOutcome::Fetched { mutated: true } = refresh_org_config(config) else {
        return Ok(());
    };
    config.save(config_path)?;
    Ok(())
}

/// Best-effort `gh variable get` of `HYPRLAYER_TELEMETRY_KEY` and
/// `HYPRLAYER_ORG_ID` from the user's thoughts-repo GitHub origin.
/// Silent on every failure path; the gh shell-out itself swallows
/// errors and returns None. Does nothing for opted-out users — the
/// `git`/`gh` exec surface is a privacy concern in its own right and we
/// only run it once the user has explicitly enabled telemetry.
pub fn refresh_org_config(config: &mut HyprlayerConfig) -> RefreshOutcome {
    if config.telemetry.mode == TelemetryMode::Off {
        return RefreshOutcome::SkippedNotOptedIn;
    }
    let owner_repo = match resolve_owner_repo(config) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };

    let mut mutated = false;
    if let Some(key) = org_config::fetch_telemetry_key(&owner_repo) {
        mutated |= replace_if_changed(&mut config.telemetry.api_key, Some(key));
        mutated |= replace_if_changed(&mut config.telemetry.api_key_source, KeySource::Github);
    }
    if let Some(org) = org_config::fetch_org_id(&owner_repo) {
        mutated |= replace_if_changed(&mut config.telemetry.org_id, Some(org));
    }
    if mutated {
        config.telemetry.last_config_refresh = unix_now();
    }
    RefreshOutcome::Fetched { mutated }
}

/// Provision installation_id + device_salt and flip mode → Anonymous.
/// Called from `telemetry on` / `telemetry init` — the *only* explicit
/// opt-in surfaces. Refreshes org config first so the resolved key is
/// known before we decide whether to print the disclosure. Idempotent on
/// `installation_id`: a user who has run `telemetry off` keeps their id
/// and is never re-enrolled when they run `on` again.
pub fn enroll(config: &mut HyprlayerConfig) -> bool {
    let was_first_run = config.telemetry.installation_id.is_none();
    config.telemetry.ensure_ids(generate_device_salt);
    config.telemetry.mode = TelemetryMode::Anonymous;

    // Resolve the org override now (before disclosure decision) so the
    // user only sees the community-key disclosure when no org has taken
    // over comms.
    let _ = refresh_org_config(config);

    if was_first_run && config.telemetry.api_key_source == KeySource::Default {
        disclosure::print_telemetry_disclosure();
    }
    was_first_run
}

fn replace_if_changed<T: PartialEq>(slot: &mut T, new: T) -> bool {
    if *slot == new {
        false
    } else {
        *slot = new;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BackendConfig, HyprlayerConfig, NotionConfig, TelemetryConfig, ThoughtsConfig,
    };

    fn fresh_config() -> HyprlayerConfig {
        HyprlayerConfig::default()
    }

    /// `telemetry on` is the explicit opt-in. First call flips mode and
    /// generates ids; second call is idempotent.
    #[test]
    fn enroll_first_run_flips_mode_and_generates_ids() {
        let mut cfg = fresh_config();
        let was_first = enroll(&mut cfg);
        assert!(was_first);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        let id = cfg.telemetry.installation_id.clone();
        assert!(id.is_some());
        assert!(cfg.telemetry.device_salt.is_some());

        // Second enroll is a no-op on identity but re-flips mode (which
        // is the point of `telemetry on` being idempotent).
        let was_first_again = enroll(&mut cfg);
        assert!(!was_first_again);
        assert_eq!(cfg.telemetry.installation_id, id);
    }

    /// Off survives subsequent lifecycle hooks: a user who explicitly
    /// opted out keeps `mode == Off` even when `ai configure` /
    /// `thoughts init` re-fire `apply_side_effects`.
    #[test]
    fn off_survives_lifecycle_apply_side_effects() {
        let mut cfg = fresh_config();
        enroll(&mut cfg);
        cfg.telemetry.mode = TelemetryMode::Off;

        // Lifecycle hook would short-circuit — the gate is `mode == Off`.
        let outcome = refresh_org_config(&mut cfg);
        assert_eq!(outcome, RefreshOutcome::SkippedNotOptedIn);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Off);
    }

    /// A user who never ran `telemetry on` is never enrolled by lifecycle.
    /// Confirms the privacy-critical invariant: `ai configure` and
    /// `thoughts init` cannot opt the user in.
    #[test]
    fn lifecycle_does_not_enroll_pristine_install() {
        let mut cfg = fresh_config();
        let outcome = refresh_org_config(&mut cfg);
        assert_eq!(outcome, RefreshOutcome::SkippedNotOptedIn);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Off);
        assert!(cfg.telemetry.installation_id.is_none());
    }

    #[test]
    fn refresh_skips_when_source_is_manual() {
        let mut cfg = fresh_config();
        enroll(&mut cfg);
        cfg.telemetry.api_key_source = KeySource::Manual;
        cfg.telemetry.api_key = Some("phc_manual".to_string());

        assert_eq!(refresh_org_config(&mut cfg), RefreshOutcome::SkippedManual);
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Manual);
        assert_eq!(cfg.telemetry.api_key.as_deref(), Some("phc_manual"));
    }

    #[test]
    fn refresh_skips_when_thoughts_backend_is_not_git() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "u".to_string(),
                backend: BackendConfig::Notion(NotionConfig {
                    parent_page_id: "p1".to_string(),
                    database_id: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            refresh_org_config(&mut cfg),
            RefreshOutcome::SkippedNoBackend
        );
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Default);
    }
}
