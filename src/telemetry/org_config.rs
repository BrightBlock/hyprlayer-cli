//! Optional org-managed telemetry config via `gh variable get`.
//!
//! When the user's thoughts repo is a GitHub-hosted git checkout, we shell
//! out to `gh` to read `HYPRLAYER_TELEMETRY_KEY` and `HYPRLAYER_ORG_ID`
//! variables. Failures are silent — gh missing, gh unauthed, variable
//! missing, non-GitHub remote, network blip — every path funnels to `None`
//! and the caller falls back to the hardcoded community key.

use std::path::Path;
use std::process::Command;

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
    let output = Command::new("git")
        .args(["-C", path, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?;
    parse_github_owner_repo(&url)
}

/// `gh -R <owner/repo> variable get <name>`. Returns the stdout (trimmed,
/// non-empty), or `None` on any failure path.
pub fn fetch_variable(owner_repo: &str, name: &str) -> Option<String> {
    let output = Command::new("gh")
        .args(["-R", owner_repo, "variable", "get", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let val = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if val.is_empty() { None } else { Some(val) }
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
    #[test]
    fn fetch_variable_missing_returns_none() {
        // gh-not-installed / gh-not-authed paths also funnel here. Either
        // way, the function must return None without panicking.
        let v = fetch_variable("octocat/Hello-World", "DEFINITELY_NOT_A_REAL_VAR_xyz_123");
        assert!(v.is_none(), "expected None for missing var, got {v:?}");
    }
}
