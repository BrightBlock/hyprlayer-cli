#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use std::fs;
use std::path::Path;

use super::BackendContext;

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
