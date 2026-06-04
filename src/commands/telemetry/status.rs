use anyhow::Result;

use crate::cli::TelemetryStatusArgs;
use crate::config::{HyprlayerConfig, TelemetryMode};
use crate::telemetry::lifecycle::{ResolveError, resolve_owner_repo};
use crate::telemetry::org_config::{self, GhStatus, VariableAccess};
use crate::telemetry::spool;
use crate::telemetry::verbose::{self, vlog};

/// Default output is one line: the resolved mode (`off` / `anonymous` /
/// `identified`). All the operational detail (installation_id, spool
/// depth, last flush timestamp) is `--json` only, so a casual
/// `hyprlayer telemetry status` doesn't surface internals to anyone who
/// happens to glance at the terminal.
pub fn status(args: TelemetryStatusArgs) -> Result<()> {
    let TelemetryStatusArgs {
        json,
        verbose,
        config,
    } = args;
    let cfg = config.load_if_exists()?;

    // `--verbose` traces the org-managed-key resolution on stderr, leaving
    // stdout (the mode line / JSON) clean for scripted callers.
    if verbose {
        verbose::set_enabled(true);
        match cfg.as_ref() {
            Some(cfg) => diagnose(cfg),
            None => vlog!(
                "no config found — telemetry has never been initialized; mode is off. \
                 Run `hyprlayer telemetry on` (or `hyprlayer thoughts init`) to enroll."
            ),
        }
    }

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

/// Read-only walk of the same resolution the auto-enroll / refresh paths
/// take, narrating each step on stderr. Explains *why* the current mode
/// is what it is — most usefully, why an org with `HYPRLAYER_TELEMETRY_KEY`
/// set is still landing on anonymous. Performs no config mutation.
fn diagnose(cfg: &HyprlayerConfig) {
    vlog!(
        "mode={} source={} locked={}",
        cfg.telemetry.mode,
        cfg.telemetry.api_key_source,
        cfg.telemetry.is_locked()
    );

    match org_config::gh_cli_status() {
        GhStatus::NotInstalled => {
            vlog!(
                "gh CLI: NOT installed. The org-managed key lives in a GitHub repo \
                 variable; without `gh` it can't be read and telemetry stays anonymous. \
                 Install https://cli.github.com and run `gh auth login`."
            );
            // Nothing downstream can read variables without `gh`. Stop here
            // rather than fall through and emit a second, near-identical
            // "gh not found" line from `fetch_variable`.
            return;
        }
        GhStatus::NotAuthenticated => vlog!(
            "gh CLI: installed but NOT authenticated. Run `gh auth login` — the same \
             auth gates pull requests and commits."
        ),
        GhStatus::Ready => vlog!("gh CLI: installed and authenticated."),
    }

    match resolve_owner_repo(cfg) {
        Ok(owner_repo) => {
            vlog!("thoughts-repo GitHub origin: {owner_repo}");
            if cfg.telemetry.mode == TelemetryMode::Off {
                vlog!("telemetry is off — skipping the live `gh variable get` probe.");
            } else if org_config::fetch_telemetry_key(&owner_repo).is_some() {
                vlog!("HYPRLAYER_TELEMETRY_KEY resolved for {owner_repo} — identified mode.");
            } else {
                // No key came back. That's only a problem if we can't
                // *read* the repo's variables; an accessible repo with no
                // key is the normal personal-repo case, not a failure.
                match org_config::repo_variables_access(&owner_repo) {
                    VariableAccess::Readable => vlog!(
                        "no org-managed key (HYPRLAYER_TELEMETRY_KEY) is set on {owner_repo}. \
                         This is NOT a failure — repos without an org key use the default \
                         community key (anonymous mode)."
                    ),
                    VariableAccess::PermissionDenied(detail) => vlog!(
                        "{owner_repo} is visible but this `gh` account can't read its Actions \
                         variables (HTTP 403): {detail}. Reading variables needs write/admin \
                         (or the fine-grained variables) permission — org-managed telemetry \
                         can't reach you until the org grants it; you stay anonymous meanwhile."
                    ),
                    VariableAccess::NotFound(detail) => vlog!(
                        "{owner_repo} returned HTTP 404 — it doesn't exist or this `gh` account \
                         can't see it: {detail}. Check the thoughts-repo remote URL and your \
                         `gh` access (the same access gates pull requests and pushes)."
                    ),
                    VariableAccess::OtherError(detail) => {
                        vlog!("could not read {owner_repo}'s Actions variables: {detail}.")
                    }
                    VariableAccess::GhMissing => {} // already reported above
                }
            }
        }
        Err(ResolveError::Manual) => vlog!(
            "a manual key override is active; org discovery is skipped. \
             Run `hyprlayer telemetry config --reset` to return to auto-resolution."
        ),
        Err(ResolveError::NoBackend) => vlog!(
            "no git-backend thoughts repo is configured, so there's no GitHub origin \
             to read an org key from. Anonymous mode is expected here."
        ),
        Err(ResolveError::NotGithub) => vlog!(
            "the thoughts-repo origin is not a GitHub remote; org-managed keys are \
             read from GitHub repo variables, so anonymous mode is expected here."
        ),
    }
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
