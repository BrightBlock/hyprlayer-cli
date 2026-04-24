#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use std::fs;
use std::path::Path;

use super::BackendContext;

/// Build the `KEY=VALUE` pair to pass as `<cli> mcp add -e <pair>`.
///
/// `Command::new` does not invoke a shell, and `claude mcp add -e` stores its
/// argument verbatim in `~/.claude.json`. So `"NOTION_TOKEN=$NOTION_TOKEN"`
/// would land there as a literal placeholder and the MCP server would send
/// that string as the bearer token — every request 401s. Resolve the value
/// ourselves and pass it through expanded.
pub fn resolve_mcp_env_pair(env_var: &str) -> Result<String> {
    let value = std::env::var(env_var).map_err(|_| {
        anyhow::anyhow!(
            "Env var {} is not set. Export it before running init so the token \
             can be stored in the MCP registration.",
            env_var
        )
    })?;
    Ok(format!("{}={}", env_var, value))
}

/// Create the `repos/<mapped>/<user>`, `repos/<mapped>/shared`,
/// `global/<user>`, `global/shared` tree rooted at `root`.
pub fn setup_directory_structure_at(root: &Path, ctx: &BackendContext) -> Result<()> {
    let mapped = ctx
        .effective
        .mapped_name
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Cannot create thoughts tree: repo is not mapped"))?;

    let repo_thoughts_path = root.join(&ctx.effective.repos_dir).join(mapped);
    fs::create_dir_all(repo_thoughts_path.join(&ctx.effective.user))?;
    fs::create_dir_all(repo_thoughts_path.join("shared"))?;

    let global_path = root.join(&ctx.effective.global_dir);
    fs::create_dir_all(global_path.join(&ctx.effective.user))?;
    fs::create_dir_all(global_path.join("shared"))?;

    Ok(())
}

/// Create `<code_repo>/thoughts/` with symlinks into the tree rooted at `root`.
pub fn setup_symlinks_into(root: &Path, ctx: &BackendContext) -> Result<()> {
    let mapped = ctx
        .effective
        .mapped_name
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Cannot create symlinks: repo is not mapped"))?;

    let thoughts_dir = ctx.code_repo.join("thoughts");
    let repo_thoughts_path = root.join(&ctx.effective.repos_dir).join(mapped);
    let global_path = root.join(&ctx.effective.global_dir);

    if thoughts_dir.exists() {
        fs::remove_dir_all(&thoughts_dir)?;
    }
    fs::create_dir(&thoughts_dir)?;

    create_symlinks(
        &thoughts_dir,
        &repo_thoughts_path,
        &global_path,
        &ctx.effective.user,
    )
}

#[cfg(unix)]
fn create_symlinks(
    thoughts_dir: &Path,
    repo_thoughts_path: &Path,
    global_path: &Path,
    user: &str,
) -> Result<()> {
    std::os::unix::fs::symlink(repo_thoughts_path.join(user), thoughts_dir.join(user))?;
    std::os::unix::fs::symlink(
        repo_thoughts_path.join("shared"),
        thoughts_dir.join("shared"),
    )?;
    std::os::unix::fs::symlink(global_path, thoughts_dir.join("global"))?;
    Ok(())
}

#[cfg(windows)]
fn create_symlinks(
    thoughts_dir: &Path,
    repo_thoughts_path: &Path,
    global_path: &Path,
    user: &str,
) -> Result<()> {
    use std::os::windows::fs::symlink_dir;

    let create = |target: &Path, link: &Path| -> Result<()> {
        symlink_dir(target, link).with_context(|| {
            format!(
                "Failed to create symlink. On Windows, symlinks require either:\n\
                 1. Run as Administrator, or\n\
                 2. Enable Developer Mode in Settings > Update & Security > For developers\n\n\
                 Target: {}\nLink: {}",
                target.display(),
                link.display()
            )
        })
    };

    create(&repo_thoughts_path.join(user), &thoughts_dir.join(user))?;
    create(
        &repo_thoughts_path.join("shared"),
        &thoughts_dir.join("shared"),
    )?;
    create(global_path, &thoughts_dir.join("global"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_mcp_env_pair_expands_value() {
        // Unique var name per test so parallel tests in this crate don't race
        // on the shared process environment.
        let key = "HYPRLAYER_TEST_RESOLVE_MCP_ENV_PAIR_SET";
        unsafe { std::env::set_var(key, "secret-xyz") };

        let pair = resolve_mcp_env_pair(key).unwrap();

        assert_eq!(pair, format!("{}=secret-xyz", key));
        // Regression guard: the old code passed `$NAME` verbatim, which
        // `claude mcp add` then stored as a literal placeholder.
        assert!(
            !pair.contains('$'),
            "env var must be expanded to its value, not stored as a literal placeholder: {pair}"
        );

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn resolve_mcp_env_pair_errors_when_unset() {
        let key = "HYPRLAYER_TEST_RESOLVE_MCP_ENV_PAIR_UNSET";
        unsafe { std::env::remove_var(key) };

        let err = resolve_mcp_env_pair(key).unwrap_err();
        assert!(err.to_string().contains(key));
    }
}
