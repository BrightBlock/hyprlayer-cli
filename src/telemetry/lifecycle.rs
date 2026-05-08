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
//! thoughts repo's GitHub origin — including the active per-cwd profile
//! when the user keeps a non-git default backend (notion / obsidian /
//! anytype) but maps a corporate project to a git profile.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{HyprlayerConfig, KeySource, TelemetryMode, ThoughtsConfig};

use super::{disclosure, generate_device_salt, identify, org_config, unix_now};

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
    resolve_owner_repo_at(config, std::env::current_dir().ok().as_deref())
}

/// Testable inner of `resolve_owner_repo`: cwd is injected so unit tests
/// don't have to mutate process-wide state. Production callers go through
/// `resolve_owner_repo`, which reads `std::env::current_dir()`.
fn resolve_owner_repo_at(
    config: &HyprlayerConfig,
    cwd: Option<&Path>,
) -> Result<String, ResolveError> {
    if config.telemetry.api_key_source == KeySource::Manual {
        return Err(ResolveError::Manual);
    }
    let thoughts = config.thoughts.as_ref().ok_or(ResolveError::NoBackend)?;
    let path = active_thoughts_repo_path(thoughts, cwd).ok_or(ResolveError::NoBackend)?;
    let (owner, repo) = org_config::discover_github_remote(&path).ok_or(ResolveError::NotGithub)?;
    Ok(format!("{owner}/{repo}"))
}

/// Pick the thoughts repo path the org-config refresh should consult.
/// Prefers the active per-cwd profile's git backend (so a user with
/// notion as their default but a per-project profile mapped to the
/// corporate repo gets the corporate origin scanned). Falls back to the
/// top-level backend when the active profile has no git checkout.
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

/// Run org-config refresh + auto-elevate, save once if anything changed.
/// Skipped entirely when telemetry isn't enabled — no `gh`/`git` shellouts
/// for opted-out users. Never enrolls the user.
///
/// Save is ordered before the `$identify` spool: a failed save must not
/// leave behind an `$identify` event that links an installation_id to a
/// user_id while the on-disk config still says anonymous (the spool
/// would survive across runs and PostHog would receive a half-committed
/// state change).
pub fn apply_side_effects(config: &mut HyprlayerConfig, config_path: &Path) -> Result<()> {
    let key_mut = matches!(
        refresh_org_config(config),
        RefreshOutcome::Fetched { mutated: true }
    );
    let mode_mut = auto_elevate_if_org_keyed(config);
    if key_mut || mode_mut {
        config.save(config_path)?;
    }
    if mode_mut {
        let _ = identify::record_identify(config);
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

/// Flip mode Anonymous → Identified when an org-managed PostHog key is
/// in effect for a configured user. Returns true if the mode changed.
///
/// Caller contract: persist the config first, then (only on a successful
/// save) spool the `$identify` event via `identify::record_identify`. A
/// spooled identify with no on-disk mode change leaves a half-committed
/// state across runs. The split exists so unit tests can verify the
/// decision without touching the on-disk spool.
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

/// `telemetry on` opt-in entry point. Provisions identity, pulls org
/// config, auto-elevates to Identified when an org key is in effect, and
/// persists the result. `telemetry init` does *not* go through this; it
/// pins `--mode` explicitly and uses the granular pieces above.
///
/// Save is ordered before the `$identify` spool so a failed save can't
/// leave behind an `$identify` event with no corresponding mode change
/// on disk. Returns true on first run (installation_id was just
/// generated).
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

    /// `telemetry on` is the explicit opt-in. First call flips mode and
    /// generates ids; second call is idempotent.
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

    /// Off survives subsequent lifecycle hooks: a user who explicitly
    /// opted out keeps `mode == Off` even when `ai configure` /
    /// `thoughts init` re-fire `apply_side_effects`.
    #[test]
    fn off_survives_lifecycle_apply_side_effects() {
        let (_dir, path) = temp_config_path();
        let mut cfg = fresh_config();
        enroll(&mut cfg, &path).unwrap();
        cfg.telemetry.mode = TelemetryMode::Off;

        // Lifecycle hook would short-circuit — the gate is `mode == Off`.
        let outcome = refresh_org_config(&mut cfg);
        assert_eq!(outcome, RefreshOutcome::SkippedNotOptedIn);
        assert_eq!(cfg.telemetry.mode, TelemetryMode::Off);
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
