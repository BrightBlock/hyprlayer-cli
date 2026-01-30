use anyhow::Result;
use colored::Colorize;
use std::path::MAIN_SEPARATOR_STR as SEP;

use crate::cli::StatusArgs;
use crate::config::{expand_path, get_current_repo_path};
use crate::git_ops::GitRepo;

pub fn status(args: StatusArgs) -> Result<()> {
    println!("{}", "Thoughts Repository Status".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    // Load config
    let thoughts_config = args.config.load()?;

    // Show configuration
    println!("{}", "Configuration:".yellow());
    println!("  Repository: {}", thoughts_config.thoughts_repo.cyan());
    println!("  Repos directory: {}", thoughts_config.repos_dir.cyan());
    println!("  Global directory: {}", thoughts_config.global_dir.cyan());
    println!("  User: {}", thoughts_config.user.cyan());
    println!(
        "  Mapped repos: {}",
        thoughts_config.repo_mappings.len().to_string().cyan()
    );
    println!();

    // Check current repo mapping
    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();
    let effective = thoughts_config.effective_config_for(&current_repo_str);

    if let Some(ref mapped_name) = effective.mapped_name {
        println!("{}", "Current Repository:".yellow());
        println!("  Path: {}", current_repo_str.cyan());
        if let Some(ref profile) = effective.profile_name {
            println!("  Profile: {}", profile.cyan());
        }
        println!(
            "  Thoughts directory: {}{SEP}{}",
            effective.repos_dir.cyan(),
            mapped_name.cyan()
        );
        println!(
            "  Full path: {}{SEP}{}{SEP}{}",
            effective.thoughts_repo.cyan(),
            effective.repos_dir.cyan(),
            mapped_name.cyan()
        );

        let thoughts_dir = current_repo.join("thoughts");
        if thoughts_dir.exists() {
            println!("  Status: {}", "✓ Initialized".green());
        } else {
            println!("  Status: {}", "✗ Not initialized".red());
        }
    } else {
        println!("{}", "Current repository not mapped to thoughts".yellow());
    }
    println!();

    // Show thoughts repository git status
    let expanded_repo = expand_path(&effective.thoughts_repo);
    if !expanded_repo.exists() {
        println!(
            "{}",
            format!("Thoughts repository not found at {}", effective.thoughts_repo).red()
        );
        return Ok(());
    }

    println!("{}", "Thoughts Repository Git Status:".yellow());
    let git_repo = match GitRepo::open(&expanded_repo) {
        Ok(repo) => repo,
        Err(e) => {
            println!("  Error: {}", e.to_string().red());
            return Ok(());
        }
    };

    // Show last commit
    let last_commit = git_repo
        .get_last_commit()
        .unwrap_or_else(|_| "No commits yet".bright_black().to_string());
    println!("  Last commit: {}", last_commit);

    // Show remote status
    let remote_status = git_repo
        .remote_url()
        .map(|_| "origin configured".green().to_string())
        .unwrap_or_else(|| "No remote configured".bright_black().to_string());
    println!("  Remote: {}", remote_status);

    // Show uncommitted changes
    match git_repo.has_changes() {
        Ok(true) => {
            println!();
            println!("{}", "Uncommitted changes:".yellow());
            git_repo.status().iter().for_each(|s| print!("{}", s));
            println!();
            println!(
                "{}",
                "Run 'hyprlayer thoughts sync' to commit these changes".bright_black()
            );
        }
        Ok(false) => {
            println!();
            println!("{}", "✓ No uncommitted changes".green());
        }
        Err(e) => println!("  Error checking status: {}", e),
    }

    Ok(())
}
