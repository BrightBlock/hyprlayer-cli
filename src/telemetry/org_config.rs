//! Optional org-managed telemetry config via `gh variable get`.
//!
//! When the user's thoughts repo is a GitHub-hosted git checkout, we shell
//! out to `gh` to read `HYPRLAYER_TELEMETRY_KEY` and `HYPRLAYER_ORG_ID`
//! variables. Failures are silent — gh missing, gh unauthed, variable
//! missing, non-GitHub remote, network blip — every path funnels to `None`
//! and the caller falls back to the hardcoded community key.

use std::path::Path;
use std::process::Command;

use super::verbose::vlog;

/// `(owner, repo)` pair from a GitHub remote URL, or `None` if the URL
/// isn't a GitHub remote we recognize.
pub fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let url = url.trim();

    const PREFIXES: &[&str] = &[
        "git@github.com:",
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git://github.com/",
    ];
    let after_host = PREFIXES.iter().find_map(|p| url.strip_prefix(p))?;

    let trimmed = after_host
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let (owner, repo) = trimmed.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Resolve the `(owner, repo)` of the GitHub remote at `thoughts_repo_path`,
/// or `None` if not a git checkout / not GitHub-hosted / git missing.
pub fn discover_github_remote(thoughts_repo_path: &Path) -> Option<(String, String)> {
    let path = thoughts_repo_path.to_str()?;
    let output = match Command::new("git")
        .args(["-C", path, "remote", "get-url", "origin"])
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            vlog!("`git` not found on PATH — cannot read the thoughts-repo origin at {path}");
            return None;
        }
        Err(e) => {
            vlog!("`git remote get-url origin` failed to run in {path}: {e}");
            return None;
        }
    };
    if !output.status.success() {
        vlog!(
            "`git -C {path} remote get-url origin` exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?;
    match parse_github_owner_repo(&url) {
        Some(owner_repo) => Some(owner_repo),
        None => {
            vlog!(
                "thoughts-repo origin '{}' is not a recognized GitHub remote; \
                 org-managed telemetry keys live in GitHub repo variables",
                url.trim()
            );
            None
        }
    }
}

pub fn discover_github_owner_repo(thoughts_repo_path: &Path) -> Option<String> {
    let (owner, repo) = discover_github_remote(thoughts_repo_path)?;
    Some(format!("{owner}/{repo}"))
}

/// `gh -R <owner/repo> variable get <name>`. Returns the stdout (trimmed,
/// non-empty), or `None` on any failure path.
pub fn fetch_variable(owner_repo: &str, name: &str) -> Option<String> {
    let output = match Command::new("gh")
        .args(["-R", owner_repo, "variable", "get", name])
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            vlog!(
                "`gh` CLI not found on PATH — cannot read {name} from {owner_repo}. \
                 Install GitHub CLI (https://cli.github.com) and run `gh auth login`."
            );
            return None;
        }
        Err(e) => {
            vlog!("`gh variable get {name}` failed to run for {owner_repo}: {e}");
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        // `gh variable get` reports the *same* "variable ... was not found"
        // whether the repo is accessible but the key simply isn't set (the
        // normal case for a personal repo — NOT a failure) or the repo
        // isn't accessible at all. It can't tell them apart, so phrase this
        // neutrally; `repo_variables_access` (used by the `--verbose`
        // diagnostics) does the disambiguation via `gh variable list`.
        if stderr.contains("was not found") {
            vlog!(
                "{name} is not set on {owner_repo} (or its variables aren't readable). \
                 Using the default community key."
            );
        } else {
            // Auth errors and other unexpected failures keep gh's own text.
            vlog!("`gh -R {owner_repo} variable get {name}` failed: {stderr}");
        }
        return None;
    }
    let val = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if val.is_empty() {
        vlog!("variable {name} on {owner_repo} is set but empty");
        None
    } else {
        // Never log the value — it's a PostHog write key.
        vlog!("resolved {name} from {owner_repo} (GitHub repo variable)");
        Some(val)
    }
}

/// Coarse health of the `gh` CLI, surfaced by `telemetry status
/// --verbose`. Read-only — `gh auth status` performs no mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhStatus {
    NotInstalled,
    NotAuthenticated,
    Ready,
}

/// Probe whether `gh` is installed and authenticated. Best-effort: a
/// spawn error other than `NotFound` (rare — e.g. a non-executable on
/// PATH) is reported as `NotInstalled`, which is the actionable bucket.
pub fn gh_cli_status() -> GhStatus {
    match Command::new("gh").args(["auth", "status"]).output() {
        Err(_) => GhStatus::NotInstalled,
        Ok(out) if out.status.success() => GhStatus::Ready,
        Ok(_) => GhStatus::NotAuthenticated,
    }
}

/// Why a key lookup did (or didn't) have a chance to succeed — the signal
/// that separates "no org key configured (perfectly fine)" from the
/// genuine access problems.
///
/// We deliberately probe with `gh variable list`, not `get`: `get` reports
/// the identical "variable ... was not found" for an *inaccessible* repo
/// as for an accessible repo with no such variable (verified against a
/// nonexistent repo), so it can't disambiguate. `list` returns **exit 0
/// with an empty table** on an accessible repo that has zero variables,
/// and surfaces the underlying HTTP status only on a real failure.
///
/// Note: reading Actions variables requires write/admin (or the
/// fine-grained "variables" permission), *not* mere read — so a
/// read-only collaborator gets `403`, distinct from the `404` returned
/// when the repo doesn't exist or is wholly invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableAccess {
    /// `gh variable list` exited 0: the repo's variables are visible
    /// (the table may be empty), so a missing key is a deliberate "no
    /// org telemetry" — not a failure.
    Readable,
    /// HTTP 403: the repo is visible but this account lacks permission to
    /// read its variables. Org-managed telemetry can't reach the user
    /// until the org grants variable access.
    PermissionDenied(String),
    /// HTTP 404: the repo doesn't exist or the account can't see it at
    /// all. This is the access failure worth flagging loudly.
    NotFound(String),
    /// Any other non-zero exit (network blip, auth error, gh internals).
    OtherError(String),
    /// gh isn't installed (already surfaced by [`gh_cli_status`]).
    GhMissing,
}

/// Read-only `gh variable list` probe used by the `--verbose` diagnostics
/// to classify *why* a key didn't resolve. Performs no mutation.
pub fn repo_variables_access(owner_repo: &str) -> VariableAccess {
    let output = match Command::new("gh")
        .args(["-R", owner_repo, "variable", "list"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return VariableAccess::GhMissing,
        Err(e) => return VariableAccess::OtherError(e.to_string()),
        Ok(o) => o,
    };
    if output.status.success() {
        return VariableAccess::Readable;
    }
    classify_list_failure(String::from_utf8_lossy(&output.stderr).trim())
}

/// Map a failed `gh variable list` stderr to its access category. Split
/// out as a pure function so the HTTP-status parsing is unit-testable
/// against gh's real message strings.
fn classify_list_failure(stderr: &str) -> VariableAccess {
    if stderr.contains("HTTP 404") {
        VariableAccess::NotFound(stderr.to_string())
    } else if stderr.contains("HTTP 403") {
        VariableAccess::PermissionDenied(stderr.to_string())
    } else {
        VariableAccess::OtherError(stderr.to_string())
    }
}

#[allow(dead_code)]
pub fn fetch_telemetry_key(owner_repo: &str) -> Option<String> {
    fetch_variable(owner_repo, "HYPRLAYER_TELEMETRY_KEY")
}

#[allow(dead_code)]
pub fn fetch_org_id(owner_repo: &str) -> Option<String> {
    fetch_variable(owner_repo, "HYPRLAYER_ORG_ID")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table-driven coverage for every supported URL prefix in `PREFIXES`,
    /// plus whitespace trimming and the optional `.git` tail.
    #[test]
    fn parse_remote_urls_happy_paths() {
        let cases: &[(&str, (&str, &str))] = &[
            ("https://github.com/Owner/repo.git", ("Owner", "repo")),
            ("https://github.com/Owner/repo", ("Owner", "repo")),
            ("git@github.com:Owner/repo.git", ("Owner", "repo")),
            ("git@github.com:Owner/repo", ("Owner", "repo")),
            ("ssh://git@github.com/Owner/repo.git", ("Owner", "repo")),
            ("git://github.com/Owner/repo.git", ("Owner", "repo")),
            ("  https://github.com/Owner/repo.git\n", ("Owner", "repo")),
        ];
        for (input, (owner, repo)) in cases {
            assert_eq!(
                parse_github_owner_repo(input),
                Some((owner.to_string(), repo.to_string())),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn parse_remote_rejects_non_github() {
        assert_eq!(
            parse_github_owner_repo("git@gitlab.com:owner/repo.git"),
            None
        );
        assert_eq!(
            parse_github_owner_repo("https://gitea.example.com/owner/repo.git"),
            None
        );
        assert_eq!(parse_github_owner_repo(""), None);
        assert_eq!(parse_github_owner_repo("not-a-url"), None);
    }

    #[test]
    fn parse_remote_rejects_malformed() {
        // Missing repo segment.
        assert_eq!(parse_github_owner_repo("https://github.com/owner"), None);
        assert_eq!(parse_github_owner_repo("git@github.com:owner"), None);
        // Empty owner.
        assert_eq!(parse_github_owner_repo("https://github.com//repo"), None);
        // Three+ segments.
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo/extra"),
            None
        );
    }

    #[test]
    fn discover_github_remote_rejects_non_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_github_remote(dir.path()).is_none());
    }

    #[test]
    fn discover_github_remote_handles_non_github_origin() {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "init", "-q"])
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            return;
        }
        let _ = Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "git@gitlab.com:owner/repo.git",
            ])
            .status();
        assert_eq!(discover_github_remote(dir.path()), None);
    }

    #[test]
    fn discover_github_remote_picks_up_github_origin() {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "init", "-q"])
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            return;
        }
        let _ = Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "https://github.com/octocat/Hello-World.git",
            ])
            .status();
        assert_eq!(
            discover_github_remote(dir.path()),
            Some(("octocat".to_string(), "Hello-World".to_string()))
        );
    }

    /// `fetch_variable` for a non-existent variable on a real public repo
    /// the test machine has access to. Skipped in CI / non-authed envs:
    /// gh exit 1 with message ⇒ `None`. Asserts the absence handling.
    /// Environment-dependent (gh may or may not be installed/authed on
    /// the test machine), so we only assert it returns a well-formed
    /// variant without panicking or hanging.
    #[test]
    fn gh_cli_status_returns_a_variant() {
        let s = gh_cli_status();
        assert!(matches!(
            s,
            GhStatus::NotInstalled | GhStatus::NotAuthenticated | GhStatus::Ready
        ));
    }

    /// Environment-dependent — only asserts it returns a well-formed
    /// variant without panicking. A repo that almost certainly doesn't
    /// exist must never classify as `Readable`.
    #[test]
    fn repo_variables_access_classifies_without_panicking() {
        let v = repo_variables_access("octocat/definitely-not-a-real-repo-xyz-123");
        assert_ne!(
            v,
            VariableAccess::Readable,
            "a nonexistent repo must not classify as readable"
        );
    }

    /// Deterministic coverage of the HTTP-status parsing, using the exact
    /// stderr strings `gh variable list` emitted in manual testing.
    #[test]
    fn classify_list_failure_distinguishes_403_from_404() {
        let not_found = "failed to get variables: HTTP 404: Not Found \
             (https://api.github.com/repos/acme/missing/actions/variables?per_page=100)";
        assert!(matches!(
            classify_list_failure(not_found),
            VariableAccess::NotFound(_)
        ));

        let forbidden = "failed to get variables: HTTP 403: You must have repository read \
             permissions or have the repository variables fine-grained permission. \
             (https://api.github.com/repos/cli/cli/actions/variables?per_page=100)";
        assert!(matches!(
            classify_list_failure(forbidden),
            VariableAccess::PermissionDenied(_)
        ));

        // An auth/other error keeps gh's text but lands in OtherError.
        assert!(matches!(
            classify_list_failure("gh: To get started with GitHub CLI, please run: gh auth login"),
            VariableAccess::OtherError(_)
        ));
    }

    #[test]
    fn fetch_variable_missing_returns_none() {
        // gh-not-installed / gh-not-authed paths also funnel here. Either
        // way, the function must return None without panicking.
        let v = fetch_variable("octocat/Hello-World", "DEFINITELY_NOT_A_REAL_VAR_xyz_123");
        assert!(v.is_none(), "expected None for missing var, got {v:?}");
    }
}
