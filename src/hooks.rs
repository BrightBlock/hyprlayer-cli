use anyhow::{Context, Result};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const HOOK_VERSION: &str = "2";

/// Install the pre-commit hook (always) and, when `include_auto_sync` is true,
/// the post-commit hook. With `include_auto_sync = false`, any previously-
/// installed hyprlayer post-commit is removed so backend switches don't leave
/// dead hooks firing on every commit. Returns `Ok(vec![])` if `repo_path`
/// isn't inside a git working tree (safe to call from non-filesystem backends).
pub fn setup_git_hooks(repo_path: &Path, include_auto_sync: bool) -> Result<Vec<String>> {
    let Some(hooks_dir) = get_hooks_dir(repo_path)? else {
        return Ok(Vec::new());
    };
    fs::create_dir_all(&hooks_dir)?;

    let mut updated = Vec::new();

    if install_hook(&hooks_dir, "pre-commit", pre_commit_content())? {
        updated.push("pre-commit".to_string());
    }
    if include_auto_sync {
        if install_hook(&hooks_dir, "post-commit", post_commit_content())? {
            updated.push("post-commit".to_string());
        }
    } else if remove_our_hook(&hooks_dir, "post-commit")? {
        updated.push("post-commit (removed)".to_string());
    }

    Ok(updated)
}

fn backup_path(hook_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.old", hook_path.display()))
}

fn remove_our_hook(hooks_dir: &Path, name: &str) -> Result<bool> {
    let hook_path = hooks_dir.join(name);
    if !hook_path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&hook_path).unwrap_or_default();
    if !content.contains("hyprlayer thoughts") {
        return Ok(false);
    }

    fs::remove_file(&hook_path)?;

    let backup = backup_path(&hook_path);
    if backup.exists() {
        fs::rename(&backup, &hook_path)?;
    }
    Ok(true)
}

/// Returns the hooks directory for `repo_path`, or `None` if the path isn't
/// inside a git working tree. We rely on `git rev-parse --git-common-dir`'s
/// exit code: when outside a repo, git exits non-zero and prints a fatal
/// message on stderr — we must not blindly join its empty stdout to the
/// caller path (that creates a stray `hooks/` directory).
fn get_hooks_dir(repo_path: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output()
        .context("Failed to find git directory")?;

    if !output.status.success() {
        return Ok(None);
    }

    let git_common_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if git_common_dir.is_empty() {
        return Ok(None);
    }
    let git_common_dir = if Path::new(&git_common_dir).is_absolute() {
        PathBuf::from(&git_common_dir)
    } else {
        repo_path.join(&git_common_dir)
    };

    Ok(Some(git_common_dir.join("hooks")))
}

fn hook_needs_update(hook_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(hook_path) else {
        return true;
    };

    if !content.contains("hyprlayer thoughts") {
        return false;
    }

    content
        .lines()
        .find(|l| l.contains("# Version:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v < HOOK_VERSION.parse::<u32>().unwrap_or(1))
        .unwrap_or(true)
}

fn install_hook(hooks_dir: &Path, name: &str, content: String) -> Result<bool> {
    let hook_path = hooks_dir.join(name);

    if !hook_needs_update(&hook_path) {
        return Ok(false);
    }

    if hook_path.exists() {
        let existing = fs::read_to_string(&hook_path)?;
        if !existing.contains("hyprlayer thoughts") {
            fs::rename(&hook_path, backup_path(&hook_path))?;
        }
    }

    fs::write(&hook_path, content)?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    Ok(true)
}

fn pre_commit_content() -> String {
    format!(
        r#"#!/bin/bash
# hyprlayer thoughts protection - prevent committing thoughts directory
# Version: {HOOK_VERSION}

if git diff --cached --name-only | grep -q "^thoughts/"; then
    echo "Cannot commit thoughts/ to code repository"
    echo "The thoughts directory should only exist in your separate thoughts repository."
    git reset HEAD -- thoughts/
    exit 1
fi

# Call any existing pre-commit hook
SCRIPT_PATH="$(realpath "$0")"
if [ -f "$SCRIPT_PATH.old" ]; then
    "$SCRIPT_PATH.old" "$@"
fi
"#
    )
}

fn post_commit_content() -> String {
    format!(
        r#"#!/bin/bash
# hyprlayer thoughts auto-sync
# Version: {HOOK_VERSION}

# Check if we're in a worktree (skip auto-sync in worktrees)
if [ -f .git ]; then
    exit 0
fi

# Get the commit message
COMMIT_MSG=$(git log -1 --pretty=%B)

# Auto-sync thoughts after each commit (only in non-worktree repos)
hyprlayer thoughts sync --message "Auto-sync with commit: $COMMIT_MSG" >/dev/null 2>&1 &

# Call any existing post-commit hook
SCRIPT_PATH="$(realpath "$0")"
if [ -f "$SCRIPT_PATH.old" ]; then
    "$SCRIPT_PATH.old" "$@"
fi
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Running setup against a non-git directory must NOT create a spurious
    /// `hooks/` directory next to the project — it must return Ok(empty).
    #[test]
    fn setup_git_hooks_is_noop_outside_git_repo() {
        let tmp = TempDir::new().unwrap();
        let not_a_repo = tmp.path().join("plain");
        fs::create_dir_all(&not_a_repo).unwrap();

        let updated = setup_git_hooks(&not_a_repo, false).unwrap();
        assert!(updated.is_empty());
        assert!(
            !not_a_repo.join("hooks").exists(),
            "must not create stray hooks/ directory outside git repo"
        );
        assert!(
            !not_a_repo.join(".git").exists(),
            "must not create .git at all"
        );
    }

    #[test]
    fn setup_git_hooks_installs_inside_git_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        // git init
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(&repo)
            .output()
            .unwrap();

        let updated = setup_git_hooks(&repo, true).unwrap();
        assert!(updated.contains(&"pre-commit".to_string()));
        assert!(updated.contains(&"post-commit".to_string()));
        assert!(repo.join(".git/hooks/pre-commit").exists());
        assert!(repo.join(".git/hooks/post-commit").exists());
    }

    #[test]
    fn setup_git_hooks_cleanup_removes_post_commit() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(&repo)
            .output()
            .unwrap();

        // First install with auto-sync (both hooks).
        setup_git_hooks(&repo, true).unwrap();
        assert!(repo.join(".git/hooks/post-commit").exists());

        // Second install without auto-sync — should remove the hyprlayer post-commit.
        let updated = setup_git_hooks(&repo, false).unwrap();
        assert!(
            updated.iter().any(|s| s.contains("post-commit")),
            "expected cleanup to report post-commit removal: {:?}",
            updated
        );
        assert!(
            !repo.join(".git/hooks/post-commit").exists(),
            "post-commit should have been removed after cleanup switch"
        );
        // Pre-commit must still be present.
        assert!(repo.join(".git/hooks/pre-commit").exists());
    }
}
