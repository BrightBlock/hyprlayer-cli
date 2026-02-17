use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::SyncArgs;
use crate::config::{expand_path, get_current_repo_path};
use crate::git_ops::GitRepo;

/// Recursively find all files following symlinks, avoiding cycles
fn find_files_following_symlinks(
    dir: &Path,
    base_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    // Resolve symlinks to get real path for cycle detection
    let real_path = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if visited.contains(&real_path) {
        return Ok(files);
    }
    visited.insert(real_path);

    let entries =
        fs::read_dir(dir).with_context(|| format!("Failed to read directory {:?}", dir))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip hidden files and CLAUDE.md
        if name.starts_with('.') || name == "CLAUDE.md" || name == "searchable" {
            continue;
        }

        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            files.extend(find_files_following_symlinks(&path, base_dir, visited)?);
        } else if file_type.is_symlink() {
            // Follow symlink and check what it points to
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.is_dir() {
                    files.extend(find_files_following_symlinks(&path, base_dir, visited)?);
                } else if metadata.is_file()
                    && let Ok(rel_path) = path.strip_prefix(base_dir)
                {
                    files.push(rel_path.to_path_buf());
                }
            }
        } else if file_type.is_file()
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            files.push(rel_path.to_path_buf());
        }
    }

    Ok(files)
}

/// Create searchable directory with hard links
fn create_search_directory(thoughts_dir: &Path) -> Result<()> {
    let search_dir = thoughts_dir.join("searchable");

    // Remove existing searchable directory
    if search_dir.exists() {
        // Reset permissions so we can delete (Unix only)
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("chmod")
                .args(["-R", "755"])
                .arg(&search_dir)
                .output();
        }
        fs::remove_dir_all(&search_dir)?;
    }

    // Create new searchable directory
    fs::create_dir_all(&search_dir)?;

    // Find all files through symlinks
    let mut visited = HashSet::new();
    let all_files = find_files_following_symlinks(thoughts_dir, thoughts_dir, &mut visited)?;

    // Create hard links
    let mut linked_count = 0;
    for rel_path in all_files {
        let source_path = thoughts_dir.join(&rel_path);
        let target_path = search_dir.join(&rel_path);

        // Create directory structure
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Resolve symlink to get real file path
        if let Ok(real_source) = fs::canonicalize(&source_path) {
            // Create hard link
            if fs::hard_link(&real_source, &target_path).is_ok() {
                linked_count += 1;
            }
        }
    }

    println!(
        "{}",
        format!(
            "Created {} hard links in searchable directory",
            linked_count
        )
        .bright_black()
    );

    Ok(())
}

pub fn sync(args: SyncArgs) -> Result<()> {
    let SyncArgs { message, config } = args;
    println!("{}", "Syncing thoughts...".blue());

    // Load config
    let thoughts_config = config.load()?;

    // Check if current repo has thoughts setup
    let current_repo = get_current_repo_path()?;
    let thoughts_dir = current_repo.join("thoughts");

    if !thoughts_dir.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not initialized for this repository. Run 'hyprlayer thoughts init' first."
        ));
    }

    // Create searchable directory with hard links
    println!("{}", "Creating searchable index...".blue());
    create_search_directory(&thoughts_dir)?;

    // Determine the thoughts repo path based on profile mapping
    let current_repo_str = current_repo.display().to_string();
    let effective = thoughts_config.effective_config_for(&current_repo_str);

    // Sync the thoughts repository
    let expanded_repo = expand_path(&effective.thoughts_repo);

    if !expanded_repo.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts repository not found at {}",
            effective.thoughts_repo
        ));
    }

    let git_repo = GitRepo::open(&expanded_repo)?;

    // Stage all changes
    git_repo.add_all()?;

    // Check if there are changes to commit
    let has_changes = git_repo.has_changes()?;
    if has_changes {
        let commit_message = message.unwrap_or_else(|| {
            format!(
                "Sync thoughts - {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            )
        });
        git_repo.commit(&commit_message)?;
        println!("{}", "✅ Thoughts synchronized".green());
    } else {
        println!("{}", "No changes to commit".bright_black());
    }

    // Try to sync with remote if configured
    if git_repo.remote_url().is_none() {
        println!(
            "{}",
            "ℹ️  No remote configured for thoughts repository".yellow()
        );
        return Ok(());
    }

    // Always pull to get remote changes
    println!("{}", "Pulling latest changes...".bright_black());
    if let Err(e) = git_repo.pull_rebase() {
        println!(
            "{}",
            format!("Warning: Could not pull latest changes: {}", e).yellow()
        );
    }

    // Only push if we committed changes
    if has_changes {
        println!("{}", "Pushing to remote...".bright_black());
        if let Err(e) = git_repo.push() {
            println!(
                "{}",
                format!("⚠️  Could not push to remote: {}", e).yellow()
            );
        }
    }

    Ok(())
}
