use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{HyprlayerConfig, KeySource, TelemetryMode, ThoughtsConfig};

use super::{disclosure, generate_device_salt, identify, org_config, unix_now};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    Manual,
    NoBackend,
    NotGithub,
}

#[derive(Debug)]
pub struct LockedError(pub(crate) String);

impl std::fmt::Display for LockedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LockedError {}

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

pub fn resolve_owner_repo(config: &HyprlayerConfig) -> Result<String, ResolveError> {
    resolve_owner_repo_at(config, std::env::current_dir().ok().as_deref())
}

fn resolve_owner_repo_at(
    config: &HyprlayerConfig,
    cwd: Option<&Path>,
) -> Result<String, ResolveError> {
    if config.telemetry.api_key_source == KeySource::Manual {
        return Err(ResolveError::Manual);
    }
    let thoughts = config.thoughts.as_ref().ok_or(ResolveError::NoBackend)?;
    let path = active_thoughts_repo_path(thoughts, cwd).ok_or(ResolveError::NoBackend)?;
    org_config::discover_github_owner_repo(&path).ok_or(ResolveError::NotGithub)
}

fn active_thoughts_repo_path(thoughts: &ThoughtsConfig, cwd: Option<&Path>) -> Option<PathBuf> {
    if let Some(cwd) = cwd {
        let cwd_str = cwd.display().to_string();
        let eff = thoughts.effective_config_for(&cwd_str);
        if let Some(p) = eff.backend.thoughts_repo_path() {
            return Some(p);
        }
    }
    thoughts.backend.thoughts_repo_path()
}

/// Reconcile telemetry state at explicit-user-intent entry points
/// (`thoughts init`, `ai configure`). Bypasses the 24h throttle.
pub fn apply_side_effects(config: &mut HyprlayerConfig, config_path: &Path) -> Result<()> {
    let _ = auto_enroll_and_enforce(config, config_path, true)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AutoEnrollOutcome {
    /// `was_first_lock` gates the one-time disclosure and `$identify` spool.
    Locked {
        owner_repo: String,
        org_id: Option<String>,
        was_first_lock: bool,
    },
    SkippedStickyOff,
    AlreadyEnrolled,
    EnrolledAnonymous,
    /// Throttle window: cached `is_locked()` re-enforced; no discovery.
    Reenforced,
}

/// Returns the first git backend (top-level or profile) whose GitHub
/// origin has `HYPRLAYER_TELEMETRY_KEY` set. Profiles are visited in
/// sorted-key order — HashMap iteration would otherwise pick a
/// different first-match per invocation.
pub fn discover_any_corporate_origin(
    thoughts: &ThoughtsConfig,
) -> Option<(String, String, Option<String>)> {
    discover_with(thoughts, |owner_repo| {
        let key = org_config::fetch_telemetry_key(owner_repo)?;
        Some((key, org_config::fetch_org_id(owner_repo)))
    })
}

fn discover_with<F>(
    thoughts: &ThoughtsConfig,
    fetch_fn: F,
) -> Option<(String, String, Option<String>)>
where
    F: Fn(&str) -> Option<(String, Option<String>)>,
{
    let mut profile_keys: Vec<&String> = thoughts.profiles.keys().collect();
    profile_keys.sort();
    let backends = std::iter::once(&thoughts.backend).chain(
        profile_keys
            .into_iter()
            .map(|k| &thoughts.profiles[k].backend),
    );
    for backend in backends {
        let Some(repo_path) = backend.thoughts_repo_path() else {
            continue;
        };
        let Some(owner_repo) = org_config::discover_github_owner_repo(&repo_path) else {
            continue;
        };
        let Some((key, org_id)) = fetch_fn(&owner_repo) else {
            continue;
        };
        return Some((owner_repo, key, org_id));
    }
    None
}

/// Reconcile on-disk telemetry state with the policy. Saves inline on
/// every mutating branch: the `telemetry flush` child spawned by
/// `maybe_flush_telemetry_in` reads mode from disk, so the final mode
/// must land before the fork.
///
/// `allow_gh_shellout=true` runs the discovery walk; `false` enforces
/// only the cached `is_locked()` state.
pub fn auto_enroll_and_enforce(
    config: &mut HyprlayerConfig,
    config_path: &Path,
    allow_gh_shellout: bool,
) -> Result<AutoEnrollOutcome> {
    // `thoughts == None` and "thoughts Some but no git backend" share
    // the same code path: empty walk, falls through to Anonymous.
    let discovery = if allow_gh_shellout {
        config
            .thoughts
            .as_ref()
            .and_then(discover_any_corporate_origin)
    } else {
        None
    };
    auto_enroll_inner(config, config_path, discovery, allow_gh_shellout)
}

fn auto_enroll_inner(
    config: &mut HyprlayerConfig,
    config_path: &Path,
    discovery: Option<(String, String, Option<String>)>,
    allow_gh_shellout: bool,
) -> Result<AutoEnrollOutcome> {
    if !allow_gh_shellout {
        reenforce_cached_lock(config, config_path)?;
        return Ok(AutoEnrollOutcome::Reenforced);
    }

    if let Some((owner_repo, key, org_id)) = discovery {
        let was_first_lock =
            !config.telemetry.is_locked() || config.telemetry.mode != TelemetryMode::Identified;
        config.telemetry.ensure_ids(generate_device_salt);
        config.telemetry.api_key = Some(key);
        config.telemetry.api_key_source = KeySource::Github;
        if org_id.is_some() {
            config.telemetry.org_id = org_id.clone();
        }
        config.telemetry.mode = TelemetryMode::Identified;
        let now = unix_now();
        config.telemetry.last_enrollment_check = now;
        config.telemetry.last_config_refresh = now;
        // Save before spooling $identify so a failed save can't leave
        // behind an identify event that contradicts on-disk mode.
        config.save(config_path)?;
        if was_first_lock {
            disclosure::print_corporate_lock_disclosure(&owner_repo, org_id.as_deref());
            let _ = identify::record_identify(config);
        }
        return Ok(AutoEnrollOutcome::Locked {
            owner_repo,
            org_id,
            was_first_lock,
        });
    }

    if config.telemetry.is_locked() {
        reenforce_cached_lock(config, config_path)?;
        return Ok(AutoEnrollOutcome::Reenforced);
    }

    config.telemetry.last_enrollment_check = unix_now();

    if config.telemetry.installation_id.is_some() {
        let outcome = if config.telemetry.mode == TelemetryMode::Off {
            AutoEnrollOutcome::SkippedStickyOff
        } else {
            AutoEnrollOutcome::AlreadyEnrolled
        };
        config.save(config_path)?;
        return Ok(outcome);
    }

    config.telemetry.ensure_ids(generate_device_salt);
    config.telemetry.mode = TelemetryMode::Anonymous;
    config.save(config_path)?;
    disclosure::print_telemetry_disclosure();
    Ok(AutoEnrollOutcome::EnrolledAnonymous)
}

/// Drag mode back to `Identified` when `is_locked()` says locked but
/// `mode` has been edited away. Shared by the throttled and
/// discovery-empty paths so neither gate becomes a bypass.
fn reenforce_cached_lock(config: &mut HyprlayerConfig, config_path: &Path) -> Result<()> {
    if config.telemetry.is_locked() && config.telemetry.mode != TelemetryMode::Identified {
        config.telemetry.mode = TelemetryMode::Identified;
        config.save(config_path)?;
    }
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

/// Provision installation_id + device_salt and flip Off → Anonymous.
/// Returns true on first run (installation_id was just generated).
/// Idempotent: a user who ran `telemetry off` keeps their id and stays at
/// whatever non-Off mode they were last set to. Used directly by
/// `telemetry init`, which pins mode after refresh and so must skip the
/// auto-elevate that `enroll` does on its behalf.
pub fn provision_identity(config: &mut HyprlayerConfig) -> bool {
    let was_first_run = config.telemetry.installation_id.is_none();
    config.telemetry.ensure_ids(generate_device_salt);
    if config.telemetry.mode == TelemetryMode::Off {
        config.telemetry.mode = TelemetryMode::Anonymous;
    }
    was_first_run
}

/// First-run community-key disclosure. Skipped when an org override is
/// already in effect (the org owns user comms in that case).
pub fn maybe_print_disclosure(config: &HyprlayerConfig, was_first_run: bool) {
    if was_first_run && config.telemetry.api_key_source == KeySource::Default {
        disclosure::print_telemetry_disclosure();
    }
}

pub fn auto_elevate_if_org_keyed(config: &mut HyprlayerConfig) -> bool {
    if config.telemetry.mode != TelemetryMode::Anonymous {
        return false;
    }
    if config.telemetry.api_key_source != KeySource::Github {
        return false;
    }
    let user_set = config
        .thoughts
        .as_ref()
        .map(|t| !t.user.is_empty())
        .unwrap_or(false);
    if !user_set {
        return false;
    }
    config.telemetry.mode = TelemetryMode::Identified;
    true
}

pub fn enroll(config: &mut HyprlayerConfig, config_path: &Path) -> Result<bool> {
    let was_first_run = provision_identity(config);

    // Resolve the org override before the disclosure decision so the
    // user only sees the community-key disclosure when no org has taken
    // over comms.
    let _ = refresh_org_config(config);
    let mode_elevated = auto_elevate_if_org_keyed(config);

    config.save(config_path)?;
    if mode_elevated {
        let _ = identify::record_identify(config);
    }

    maybe_print_disclosure(config, was_first_run);
    Ok(was_first_run)
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
        BackendConfig, GitConfig, HyprlayerConfig, NotionConfig, ProfileConfig, RepoMapping,
        TelemetryConfig, ThoughtsConfig,
    };

    fn fresh_config() -> HyprlayerConfig {
        HyprlayerConfig::default()
    }

    fn git_backend(repo: &str) -> BackendConfig {
        BackendConfig::Git(GitConfig {
            thoughts_repo: repo.to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
        })
    }

    fn notion_backend() -> BackendConfig {
        BackendConfig::Notion(NotionConfig {
            parent_page_id: "p1".to_string(),
            database_id: None,
        })
    }

    fn temp_config_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        (dir, path)
    }

    /// `None` signals "skip this test" — CI without `git` on PATH.
    fn init_git_repo_with_origin(origin_url: &str) -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["-C", path, "init", "-q"])
            .status()
            .ok()
            .filter(|s| s.success())?;
        std::process::Command::new("git")
            .args(["-C", path, "remote", "add", "origin", origin_url])
            .status()
            .ok()
            .filter(|s| s.success())?;
        Some(dir)
    }

    fn locked_config(mode: TelemetryMode, installation_id: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode,
                installation_id: installation_id.map(str::to_string),
                api_key: Some("phc_corp".to_string()),
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn enroll_first_run_flips_mode_and_generates_ids() {
        let (_dir, path) = temp_config_path();
        let mut cfg = fresh_config();
        let was_first = enroll(&mut cfg, &path).unwrap();
        assert!(was_first);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        let id = cfg.telemetry.installation_id.clone();
        assert!(id.is_some());
        assert!(cfg.telemetry.device_salt.is_some());

        // Second enroll is a no-op on identity (and on mode, since the
        // user is already Anonymous and there's no org key to elevate).
        let was_first_again = enroll(&mut cfg, &path).unwrap();
        assert!(!was_first_again);
        assert_eq!(cfg.telemetry.installation_id, id);
    }

    #[test]
    fn auto_enroll_preserves_sticky_off_when_no_corporate() {
        let (_dir, path) = temp_config_path();
        let mut cfg = fresh_config();
        enroll(&mut cfg, &path).unwrap();
        cfg.telemetry.mode = TelemetryMode::Off;
        let installation_id = cfg.telemetry.installation_id.clone();

        let outcome = auto_enroll_inner(&mut cfg, &path, None, true).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::SkippedStickyOff);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Off);
        assert_eq!(cfg.telemetry.installation_id, installation_id);
    }

    /// `apply_side_effects` propagates a save error (rather than
    /// swallowing it). The deeper invariant that motivated this — "no
    /// `$identify` spool on save failure" — is enforced structurally by
    /// the `?` on `config.save` happening before `record_identify`, not
    /// observable from a unit test that can't intercept the global
    /// spool. This test exercises the error path so the structural
    /// invariant has a load-bearing caller.
    #[test]
    fn apply_side_effects_save_error_propagates() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".to_string()),
                api_key_source: KeySource::Github,
                api_key: Some("phc_org".to_string()),
                org_id: Some("acme".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let unwritable = Path::new("/nonexistent/hyprlayer-test/config.json");
        assert!(apply_side_effects(&mut cfg, unwritable).is_err());
    }

    #[test]
    fn auto_enroll_pristine_no_thoughts_lands_at_anonymous() {
        let (_dir, path) = temp_config_path();
        let mut cfg = fresh_config();
        assert!(cfg.thoughts.is_none());
        let outcome = auto_enroll_inner(&mut cfg, &path, None, true).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::EnrolledAnonymous);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        assert!(cfg.telemetry.installation_id.is_some());
        assert!(cfg.telemetry.device_salt.is_some());
    }

    #[test]
    fn auto_enroll_pristine_with_thoughts_no_corporate_lands_at_anonymous() {
        let (_dir, path) = temp_config_path();
        let mut cfg = HyprlayerConfig {
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let outcome = auto_enroll_inner(&mut cfg, &path, None, true).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::EnrolledAnonymous);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        assert!(cfg.telemetry.installation_id.is_some());
    }

    #[test]
    fn auto_enroll_corporate_overrides_sticky_off() {
        let (_dir, path) = temp_config_path();
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Off,
                installation_id: Some("preexisting".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let discovery = Some((
            "acme/thoughts".to_string(),
            "phc_corp".to_string(),
            Some("acme".to_string()),
        ));
        let outcome = auto_enroll_inner(&mut cfg, &path, discovery, true).unwrap();
        assert!(matches!(outcome, AutoEnrollOutcome::Locked { .. }));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Github);
        assert_eq!(cfg.telemetry.api_key.as_deref(), Some("phc_corp"));
        assert_eq!(cfg.telemetry.org_id.as_deref(), Some("acme"));
        assert_eq!(
            cfg.telemetry.installation_id.as_deref(),
            Some("preexisting")
        );
        assert!(cfg.telemetry.is_locked());
    }

    #[test]
    fn auto_enroll_throttled_reenforces_manual_off_edit() {
        let (_dir, path) = temp_config_path();
        let mut cfg = locked_config(TelemetryMode::Off, Some("id"));
        assert!(cfg.telemetry.is_locked());

        let outcome = auto_enroll_inner(&mut cfg, &path, None, false).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::Reenforced);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
    }

    #[test]
    fn auto_enroll_throttled_noop_when_already_consistent() {
        let (_dir, path) = temp_config_path();
        let mut cfg = locked_config(TelemetryMode::Identified, Some("id"));
        let outcome = auto_enroll_inner(&mut cfg, &path, None, false).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::Reenforced);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
    }

    /// `--refresh` is the only legitimate release path, so an empty
    /// discovery walk (transient gh failure, thoughts config removed)
    /// must NOT fall through and let a manual `mode=off` edit survive.
    #[test]
    fn auto_enroll_discovery_empty_reenforces_cached_lock() {
        let (_dir, path) = temp_config_path();
        let mut cfg = locked_config(TelemetryMode::Off, Some("id"));
        assert!(cfg.telemetry.is_locked());

        let outcome = auto_enroll_inner(&mut cfg, &path, None, true).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::Reenforced);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
    }

    #[test]
    fn auto_enroll_first_run_no_corporate_lands_at_anonymous() {
        let (_dir, path) = temp_config_path();
        let mut cfg = HyprlayerConfig {
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let outcome = auto_enroll_inner(&mut cfg, &path, None, true).unwrap();
        assert_eq!(outcome, AutoEnrollOutcome::EnrolledAnonymous);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
        assert_eq!(cfg.telemetry.api_key_source, KeySource::Default);
        assert!(cfg.telemetry.installation_id.is_some());
    }

    #[test]
    fn discover_walk_returns_first_match_across_backends() {
        let Some(dir) = init_git_repo_with_origin("https://github.com/acme/thoughts.git") else {
            return;
        };
        let thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: git_backend(dir.path().to_str().unwrap()),
            ..Default::default()
        };
        let result = discover_with(&thoughts, |owner_repo| {
            (owner_repo == "acme/thoughts")
                .then(|| ("phc_corp".to_string(), Some("acme-org".to_string())))
        });
        assert_eq!(
            result,
            Some((
                "acme/thoughts".to_string(),
                "phc_corp".to_string(),
                Some("acme-org".to_string())
            ))
        );
    }

    #[test]
    fn discover_walk_finds_profile_only_git_backend() {
        let Some(dir) = init_git_repo_with_origin("https://github.com/corp/repo.git") else {
            return;
        };
        let mut thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: notion_backend(),
            ..Default::default()
        };
        thoughts.profiles.insert(
            "corp".to_string(),
            ProfileConfig {
                backend: git_backend(dir.path().to_str().unwrap()),
            },
        );
        let result = discover_with(&thoughts, |_| Some(("phc_corp".to_string(), None)));
        assert!(matches!(result, Some((ref owner, _, _)) if owner == "corp/repo"));
    }

    #[test]
    fn discover_walk_returns_none_without_git_backend() {
        let thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: notion_backend(),
            ..Default::default()
        };
        let result = discover_with(&thoughts, |_| {
            Some(("phc_yes".to_string(), Some("any".to_string())))
        });
        assert_eq!(result, None);
    }

    #[test]
    fn refresh_skips_when_source_is_manual() {
        let (_dir, path) = temp_config_path();
        let mut cfg = fresh_config();
        enroll(&mut cfg, &path).unwrap();
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
                backend: notion_backend(),
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

    /// The motivating bug: a corporate user with notion as their default
    /// backend and an `acme` profile mapped to `~/acme-thoughts` must have
    /// the acme git backend consulted when cwd matches the corporate
    /// project — not the top-level notion backend.
    #[test]
    fn active_path_prefers_profile_git_backend_when_cwd_mapped() {
        let mut thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: notion_backend(),
            ..Default::default()
        };
        thoughts.profiles.insert(
            "acme".to_string(),
            ProfileConfig {
                backend: git_backend("/home/u/acme-thoughts"),
            },
        );
        thoughts.repo_mappings.insert(
            "/home/u/work/corp-project".to_string(),
            RepoMapping::new("corp-project", &Some("acme".to_string())),
        );

        let cwd = Path::new("/home/u/work/corp-project");
        assert_eq!(
            active_thoughts_repo_path(&thoughts, Some(cwd)),
            Some(PathBuf::from("/home/u/acme-thoughts"))
        );
    }

    /// When cwd's profile has a non-git backend, fall through to the
    /// top-level. Useful for users whose default backend *is* the corporate
    /// git checkout, with a notion profile carved out for personal projects.
    #[test]
    fn active_path_falls_back_to_top_level_when_profile_is_non_git() {
        let mut thoughts = ThoughtsConfig {
            user: "u".to_string(),
            backend: git_backend("/home/u/main-thoughts"),
            ..Default::default()
        };
        thoughts.profiles.insert(
            "personal".to_string(),
            ProfileConfig {
                backend: notion_backend(),
            },
        );
        thoughts.repo_mappings.insert(
            "/home/u/personal".to_string(),
            RepoMapping::new("personal", &Some("personal".to_string())),
        );

        let cwd = Path::new("/home/u/personal");
        assert_eq!(
            active_thoughts_repo_path(&thoughts, Some(cwd)),
            Some(PathBuf::from("/home/u/main-thoughts"))
        );
    }

    #[test]
    fn active_path_returns_none_when_no_git_backend_anywhere() {
        let thoughts = ThoughtsConfig {
            user: "u".to_string(),
            backend: notion_backend(),
            ..Default::default()
        };
        assert!(active_thoughts_repo_path(&thoughts, Some(Path::new("/anywhere"))).is_none());
    }

    /// Cwd-aware `resolve_owner_repo_at` reaches the GitHub-discovery step
    /// for a profile-mapped corporate repo. We can't test the actual `git
    /// remote` shell-out without a real checkout, so we verify the path
    /// reaches `NotGithub` (rather than `NoBackend`) when given a
    /// non-existent path — that's proof the gate let us through.
    #[test]
    fn resolve_owner_repo_at_consults_profile_for_mapped_cwd() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let thoughts = cfg.thoughts.as_mut().unwrap();
        thoughts.profiles.insert(
            "acme".to_string(),
            ProfileConfig {
                backend: git_backend("/nonexistent/acme-thoughts"),
            },
        );
        thoughts.repo_mappings.insert(
            "/work/corp".to_string(),
            RepoMapping::new("corp", &Some("acme".to_string())),
        );

        let cwd = Path::new("/work/corp");
        // Without the profile-aware resolver, this would land on
        // SkippedNoBackend (top-level notion has no git path). With it,
        // we get past the backend check and fail at git remote discovery.
        assert_eq!(
            resolve_owner_repo_at(&cfg, Some(cwd)),
            Err(ResolveError::NotGithub)
        );
    }

    #[test]
    fn auto_elevate_flips_when_org_keyed_and_user_set() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".to_string()),
                api_key_source: KeySource::Github,
                api_key: Some("phc_org".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "jt".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(auto_elevate_if_org_keyed(&mut cfg));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
    }

    #[test]
    fn auto_elevate_skips_for_default_key_source() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("id".to_string()),
                api_key_source: KeySource::Default,
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "jt".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!auto_elevate_if_org_keyed(&mut cfg));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
    }

    #[test]
    fn auto_elevate_skips_when_already_identified() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "jt".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!auto_elevate_if_org_keyed(&mut cfg));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Identified);
    }

    #[test]
    fn auto_elevate_skips_when_off() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Off,
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "jt".to_string(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!auto_elevate_if_org_keyed(&mut cfg));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Off);
    }

    /// Anonymous-with-org-key but no thoughts.user: refuse to elevate.
    /// `record_identify` would emit a warning to stderr in this state, and
    /// per-user attribution doesn't make sense without a user.
    #[test]
    fn auto_elevate_skips_when_user_is_empty() {
        let mut cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                api_key_source: KeySource::Github,
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: String::new(),
                backend: notion_backend(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!auto_elevate_if_org_keyed(&mut cfg));
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Anonymous);
    }
}
