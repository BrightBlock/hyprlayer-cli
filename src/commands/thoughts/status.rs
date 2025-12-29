use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use std::fs;

use crate::config::{expand_path, get_current_repo_path, get_default_config_path, ConfigFile};
use crate::git_ops::GitRepo;

#[derive(Parser, Debug)]
pub struct StatusOptions {
    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn status(options: StatusOptions) -> Result<()> {
    println!("{}", "Thoughts Repository Status".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    // Load config
    let config_path = options
        .config_file
        .as_ref()
        .map(|p| expand_path(p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "No thoughts configuration found. Run 'hyprlayer thoughts init' first."
        ));
    }

    let content = fs::read_to_string(&config_path)?;
    let config_file: ConfigFile = serde_json::from_str(&content)?;
    let config = config_file
        .thoughts
        .ok_or_else(|| anyhow::anyhow!("No thoughts configuration found"))?;

    // Show configuration
    println!("{}", "Configuration:".yellow());
    println!("  Repository: {}", config.thoughts_repo.cyan());
    println!("  Repos directory: {}", config.repos_dir.cyan());
    println!("  Global directory: {}", config.global_dir.cyan());
    println!("  User: {}", config.user.cyan());
    println!(
        "  Mapped repos: {}",
        config.repo_mappings.len().to_string().cyan()
    );
    println!();

    // Check current repo mapping
    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();

    if let Some(mapping) = config.repo_mappings.get(&current_repo_str) {
        let mapped_name = mapping.repo();
        println!("{}", "Current Repository:".yellow());
        println!("  Path: {}", current_repo_str.cyan());
        println!(
            "  Thoughts directory: {}/{}",
            config.repos_dir.cyan(),
            mapped_name.cyan()
        );

        let thoughts_dir = current_repo.join("thoughts");
        if thoughts_dir.exists() {
            println!("  Status: {}", "✓ Initialized".green());
        } else {
            println!("  Status: {}", "✗ Not initialized".red());
        }
    } else {
        println!(
            "{}",
            "Current repository not mapped to thoughts".yellow()
        );
    }
    println!();

    // Show thoughts repository git status
    let expanded_repo = expand_path(&config.thoughts_repo);
    if expanded_repo.exists() {
        println!("{}", "Thoughts Repository Git Status:".yellow());

        match GitRepo::open(&expanded_repo) {
            Ok(git_repo) => {
                // Show last commit
                match git_repo.get_last_commit() {
                    Ok(last_commit) => {
                        println!("  Last commit: {}", last_commit);
                    }
                    Err(_) => {
                        println!("  Last commit: {}", "No commits yet".bright_black());
                    }
                }

                // Show remote status
                if git_repo.remote_url().is_some() {
                    println!("  Remote: {}", "origin configured".green());
                } else {
                    println!("  Remote: {}", "No remote configured".bright_black());
                }

                // Show uncommitted changes
                match git_repo.has_changes() {
                    Ok(true) => {
                        println!();
                        println!("{}", "Uncommitted changes:".yellow());
                        if let Ok(status) = git_repo.status() {
                            print!("{}", status);
                        }
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
                    Err(e) => {
                        println!("  Error checking status: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("  Error: {}", e.to_string().red());
            }
        }
    } else {
        println!(
            "{}",
            format!(
                "Thoughts repository not found at {}",
                config.thoughts_repo
            )
            .red()
        );
    }

    Ok(())
}
