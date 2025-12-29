use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use std::fs;

use crate::config::{expand_path, get_current_repo_path, get_default_config_path, ConfigFile};

#[derive(Parser, Debug)]
pub struct UninitOptions {
    #[arg(long, help = "Force removal even if not in configuration")]
    pub force: bool,

    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn uninit(options: UninitOptions) -> Result<()> {
    let current_repo = get_current_repo_path()?;
    let thoughts_dir = current_repo.join("thoughts");

    if !thoughts_dir.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not initialized for this repository."
        ));
    }

    // Load config
    let config_path = options
        .config_file
        .as_ref()
        .map(|p| expand_path(p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    let current_repo_str = current_repo.display().to_string();

    // Check if repo is in config
    let (mapped_name, profile_name, thoughts_repo) = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let config_file: ConfigFile = serde_json::from_str(&content)?;

        if let Some(ref config) = config_file.thoughts {
            if let Some(mapping) = config.repo_mappings.get(&current_repo_str) {
                (
                    Some(mapping.repo().to_string()),
                    None::<String>, // TODO: extract profile from mapping
                    Some(config.thoughts_repo.clone()),
                )
            } else if !options.force {
                println!(
                    "{}",
                    "Error: This repository is not in the thoughts configuration.".red()
                );
                println!("{}", "Use --force to remove the thoughts directory anyway.".yellow());
                return Ok(());
            } else {
                (None, None, Some(config.thoughts_repo.clone()))
            }
        } else {
            (None, None, None)
        }
    } else {
        if !options.force {
            println!(
                "{}",
                "Error: No thoughts configuration found.".red()
            );
            println!("{}", "Use --force to remove the thoughts directory anyway.".yellow());
            return Ok(());
        }
        (None, None, None)
    };

    println!(
        "{}",
        "Removing thoughts setup from current repository...".blue()
    );

    // Handle searchable directory if it exists
    let searchable_dir = thoughts_dir.join("searchable");
    if searchable_dir.exists() {
        println!("{}", "Removing searchable directory...".bright_black());
        // Reset permissions in case they're restricted
        let _ = std::process::Command::new("chmod")
            .args(["-R", "755"])
            .arg(&searchable_dir)
            .output();
        fs::remove_dir_all(&searchable_dir)?;
    }

    // Remove the entire thoughts directory (only symlinks)
    println!(
        "{}",
        "Removing thoughts directory (symlinks only)...".bright_black()
    );
    fs::remove_dir_all(&thoughts_dir)?;

    // Remove from config if mapped
    if mapped_name.is_some() && config_path.exists() {
        println!(
            "{}",
            "Removing repository from thoughts configuration...".bright_black()
        );

        let content = fs::read_to_string(&config_path)?;
        let mut config_file: ConfigFile = serde_json::from_str(&content)?;

        if let Some(ref mut config) = config_file.thoughts {
            config.repo_mappings.remove(&current_repo_str);
        }

        fs::write(&config_path, serde_json::to_string_pretty(&config_file)?)?;
    }

    println!("{}", "✅ Thoughts removed from repository".green());

    // Provide info about what was done
    if let (Some(name), Some(repo)) = (mapped_name, thoughts_repo) {
        println!();
        println!("{}", "Note: Your thoughts content remains safe in:".bright_black());
        println!("  {}", format!("{}/repos/{}", repo, name).bright_black());
        if let Some(profile) = profile_name {
            println!("  {}", format!("(profile: {})", profile).bright_black());
        }
        println!(
            "{}",
            "Only the local symlinks and configuration were removed.".bright_black()
        );
    }

    Ok(())
}
