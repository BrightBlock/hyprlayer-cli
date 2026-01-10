use anyhow::Result;
use colored::Colorize;
use std::fs;

use crate::cli::args::ConfigArgs;
use crate::config::{ConfigFile, expand_path, get_current_repo_path, get_default_config_path};

type MappingInfo = (Option<String>, Option<String>, Option<String>);

fn load_config_mapping(
    config_path: &std::path::Path,
    current_repo_str: &str,
    force: bool,
) -> Result<MappingInfo> {
    if !config_path.exists() {
        if !force {
            println!("{}", "Error: No thoughts configuration found.".red());
            println!(
                "{}",
                "Use --force to remove the thoughts directory anyway.".yellow()
            );
            return Ok((None, None, None));
        }
        return Ok((None, None, None));
    }

    let content = fs::read_to_string(config_path)?;
    let config_file: ConfigFile = serde_json::from_str(&content)?;

    let thoughts_config = match config_file.thoughts {
        Some(config) => config,
        None => return Ok((None, None, None)),
    };

    let mapping = match thoughts_config.repo_mappings.get(current_repo_str) {
        Some(m) => (
            Some(m.repo().to_string()),
            m.profile().map(|p| p.to_string()),
            Some(thoughts_config.thoughts_repo.clone()),
        ),
        None => {
            if !force {
                println!(
                    "{}",
                    "Error: This repository is not in the thoughts configuration.".red()
                );
                println!(
                    "{}",
                    "Use --force to remove the thoughts directory anyway.".yellow()
                );
                return Ok((None, None, None));
            }
            (None, None, Some(thoughts_config.thoughts_repo.clone()))
        }
    };

    Ok(mapping)
}

fn remove_from_config(config_path: &std::path::Path, repo_key: &str) -> Result<()> {
    let content = fs::read_to_string(config_path)?;
    let mut config_file: ConfigFile = serde_json::from_str(&content)?;
    config_file
        .thoughts
        .as_mut()
        .map(|config| config.repo_mappings.remove(repo_key));
    fs::write(config_path, serde_json::to_string_pretty(&config_file)?)?;
    Ok(())
}

pub fn uninit(force: bool, config: ConfigArgs) -> Result<()> {
    let current_repo = get_current_repo_path()?;
    let thoughts_dir = current_repo.join("thoughts");

    if !thoughts_dir.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not initialized for this repository."
        ));
    }

    // Load config
    let config_path = config
        .config_file
        .as_ref()
        .map(|p| expand_path(p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    let current_repo_str = current_repo.display().to_string();

    // Check if repo is in config
    let (mapped_name, profile_name, thoughts_repo) =
        load_config_mapping(&config_path, &current_repo_str, force)?;

    println!(
        "{}",
        "Removing thoughts setup from current repository...".blue()
    );

    // Handle searchable directory if it exists
    let searchable_dir = thoughts_dir.join("searchable");
    if searchable_dir.exists() {
        println!("{}", "Removing searchable directory...".bright_black());
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
        let _ = remove_from_config(&config_path, &current_repo_str);
    }

    println!("{}", "✅ Thoughts removed from repository".green());

    // Provide info about what was done
    if let (Some(name), Some(repo)) = (mapped_name, thoughts_repo) {
        println!();
        println!(
            "{}",
            "Note: Your thoughts content remains safe in:".bright_black()
        );
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
