use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::common::FilesystemDirs;
use super::{BackendContext, StatusReport, ThoughtsBackend, common};
use crate::config::expand_path;
use crate::git_ops::GitRepo;
use crate::hooks;

pub struct GitBackend;

impl ThoughtsBackend for GitBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        let git = ctx.effective.backend.require_git()?;
        let mapped =
            ctx.effective.mapped_name.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Cannot create thoughts tree: repo is not mapped")
            })?;
        let dirs = FilesystemDirs {
            repos_dir: &git.repos_dir,
            global_dir: &git.global_dir,
            user: &ctx.effective.user,
            mapped_name: mapped,
        };

        let root = expand_path(&git.thoughts_repo);
        fs::create_dir_all(&root)?;

        common::setup_directory_structure_at(&root, &dirs)?;
        initialize_git_if_needed(&root)?;
        common::setup_symlinks_into(&root, ctx.code_repo, &dirs)?;

        hooks::setup_git_hooks(ctx.code_repo, true)?;
        Ok(())
    }

    fn sync(&self, ctx: &BackendContext, message: Option<&str>) -> Result<()> {
        let git = ctx.effective.backend.require_git()?;

        let thoughts_dir = ctx.code_repo.join("thoughts");
        if !thoughts_dir.exists() {
            return Err(anyhow::anyhow!(
                "Thoughts not initialized for this repository. Run 'hyprlayer thoughts init' first."
            ));
        }

        create_search_directory(&thoughts_dir)?;

        let expanded_repo = expand_path(&git.thoughts_repo);
        if !expanded_repo.exists() {
            return Err(anyhow::anyhow!(
                "Thoughts repository not found at {}",
                git.thoughts_repo
            ));
        }

        let git_repo = GitRepo::open(&expanded_repo)?;
        git_repo.add_all()?;

        let had_changes = git_repo.has_changes()?;
        if had_changes {
            let commit_message = message.map(|s| s.to_string()).unwrap_or_else(|| {
                format!(
                    "Sync thoughts - {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                )
            });
            git_repo.commit(&commit_message)?;
        }

        if git_repo.remote_url().is_none() {
            return Ok(());
        }

        if let Err(e) = git_repo.pull_rebase() {
            eprintln!(
                "{}",
                format!("Warning: pull --rebase failed: {}", e).yellow()
            );
        }

        if had_changes && let Err(e) = git_repo.push() {
            eprintln!("{}", format!("Warning: push failed: {}", e).yellow());
        }

        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        use std::path::MAIN_SEPARATOR_STR as SEP;
        let mut lines = Vec::new();
        let git = ctx.effective.backend.require_git()?;
        let expanded_repo = expand_path(&git.thoughts_repo);

        lines.push(format!("  Repository: {}", git.thoughts_repo.cyan()));
        if let Some(name) = &ctx.effective.mapped_name {
            lines.push(format!(
                "  Full path: {}{SEP}{}{SEP}{}",
                git.thoughts_repo.cyan(),
                git.repos_dir.cyan(),
                name.cyan()
            ));
        }
        lines.push(String::new());

        if !expanded_repo.exists() {
            lines.push(
                format!("Thoughts repository not found at {}", git.thoughts_repo)
                    .red()
                    .to_string(),
            );
            return Ok(StatusReport { lines });
        }

        lines.push(format!("{}", "Thoughts Repository Git Status:".yellow()));

        let git_repo = match GitRepo::open(&expanded_repo) {
            Ok(repo) => repo,
            Err(e) => {
                lines.push(format!("  Error: {}", e.to_string().red()));
                return Ok(StatusReport { lines });
            }
        };

        let last_commit = git_repo
            .get_last_commit()
            .unwrap_or_else(|_| "No commits yet".bright_black().to_string());
        lines.push(format!("  Last commit: {}", last_commit));

        let remote_status = git_repo
            .remote_url()
            .map(|_| "origin configured".green().to_string())
            .unwrap_or_else(|| "No remote configured".bright_black().to_string());
        lines.push(format!("  Remote: {}", remote_status));

        match git_repo.has_changes() {
            Ok(true) => {
                lines.push(String::new());
                lines.push(format!("{}", "Uncommitted changes:".yellow()));
                let status_output = git_repo.status()?;
                for line in status_output.lines() {
                    lines.push(line.to_string());
                }
                lines.push(String::new());
                lines.push(
                    "Run 'hyprlayer thoughts sync' to commit these changes"
                        .bright_black()
                        .to_string(),
                );
            }
            Ok(false) => {
                lines.push(String::new());
                lines.push(format!("{}", "No uncommitted changes".green()));
            }
            Err(e) => lines.push(format!("  Error checking status: {}", e)),
        }

        Ok(StatusReport { lines })
    }
}

fn initialize_git_if_needed(thoughts_repo_root: &Path) -> Result<()> {
    if GitRepo::is_repo(thoughts_repo_root) {
        return Ok(());
    }

    GitRepo::init(thoughts_repo_root)?;

    let gitignore = "# OS files\n.DS_Store\nThumbs.db\n\n# Editor files\n.vscode/\n.idea/\n*.swp\n*.swo\n*~\n\n# Temporary files\n*.tmp\n*.bak\n";
    fs::write(thoughts_repo_root.join(".gitignore"), gitignore)?;

    let git_repo = GitRepo::open(thoughts_repo_root)?;
    git_repo.add_all()?;
    git_repo.commit("Initial thoughts repository setup")?;

    Ok(())
}

fn find_files_following_symlinks(
    dir: &Path,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    let real_path = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if visited.contains(&real_path) {
        return Ok(files);
    }
    visited.insert(real_path);

    let entries = fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if name.starts_with('.') || name == "CLAUDE.md" || name == "searchable" {
            continue;
        }

        let file_type = entry.file_type()?;

        let (is_dir, is_file) = if file_type.is_symlink() {
            fs::metadata(&path)
                .map(|m| (m.is_dir(), m.is_file()))
                .unwrap_or((false, false))
        } else {
            (file_type.is_dir(), file_type.is_file())
        };

        if is_dir {
            files.extend(find_files_following_symlinks(&path, base_dir, visited)?);
        } else if is_file {
            files.extend(path.strip_prefix(base_dir).ok().map(Path::to_path_buf));
        }
    }

    Ok(files)
}

fn create_search_directory(thoughts_dir: &Path) -> Result<()> {
    let search_dir = thoughts_dir.join("searchable");

    if search_dir.exists() {
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("chmod")
                .args(["-R", "755"])
                .arg(&search_dir)
                .output();
        }
        fs::remove_dir_all(&search_dir)?;
    }

    fs::create_dir_all(&search_dir)?;

    let mut visited = HashSet::new();
    let all_files = find_files_following_symlinks(thoughts_dir, thoughts_dir, &mut visited)?;

    for rel_path in all_files {
        let source_path = thoughts_dir.join(&rel_path);
        let target_path = search_dir.join(&rel_path);

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let _ = fs::canonicalize(&source_path).and_then(|real| fs::hard_link(real, &target_path));
    }

    Ok(())
}
