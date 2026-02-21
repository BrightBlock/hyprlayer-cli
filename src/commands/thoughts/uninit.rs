use anyhow::Result;
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Input};
use std::fs;
use std::path::{Path, MAIN_SEPARATOR_STR as SEP};

use crate::cli::UninitArgs;
use crate::config::{get_current_repo_path, EffectiveConfig, ThoughtsConfig};

fn remove_from_config(config_path: &Path, repo_key: &str) -> Result<()> {
    let mut config = ThoughtsConfig::load(config_path)?;
    config.repo_mappings.remove(repo_key);

    // Check for other stale mappings while we're saving
    let orphaned = config.find_orphaned_mappings();
    if !orphaned.is_empty() {
        println!(
            "{}",
            "Found stale repo mappings (paths no longer exist):".yellow()
        );
        for path in &orphaned {
            println!("  {}", path.bright_black());
        }
        let remove: bool = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Remove stale mappings from config?")
            .default(true)
            .interact()?;
        if remove {
            config.remove_mappings(&orphaned);
            println!("{}", "Stale mappings removed.".green());
        }
    }

    config.save(config_path)?;
    Ok(())
}

fn print_validation_error(msg: &str) {
    println!("{}", format!("Error: {}", msg).red());
    println!(
        "{}",
        "Use --force to remove the thoughts directory anyway.".yellow()
    );
}

fn print_safe_location(eff: &EffectiveConfig) {
    let Some(ref name) = eff.mapped_name else {
        return;
    };

    println!();
    println!(
        "{}",
        "Note: Your thoughts content remains safe in:".bright_black()
    );
    println!(
        "  {}",
        format!(
            "Repo:   {}{SEP}{}{SEP}{}",
            eff.thoughts_repo, eff.repos_dir, name
        )
        .bright_black()
    );
    println!(
        "  {}",
        format!("Global: {}{SEP}{}", eff.thoughts_repo, eff.global_dir).bright_black()
    );

    eff.profile_name
        .as_ref()
        .inspect(|p| println!("  {}", format!("(profile: {})", p).bright_black()));

    println!(
        "{}",
        "Only the local symlinks and configuration were removed.".bright_black()
    );
}

pub fn uninit(args: UninitArgs) -> Result<()> {
    let UninitArgs { force, config } = args;
    let current_repo = get_current_repo_path()?;
    let thoughts_dir = current_repo.join("thoughts");

    if !thoughts_dir.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not initialized for this repository."
        ));
    }

    let config_path = config.path()?;
    let thoughts_config = config.load_if_exists()?;
    let current_repo_str = current_repo.display().to_string();

    let effective = thoughts_config
        .as_ref()
        .map(|c| c.effective_config_for(&current_repo_str));

    let is_mapped = effective.as_ref().is_some_and(|e| e.mapped_name.is_some());

    // Validation (skip if force)
    if !force && thoughts_config.is_none() {
        print_validation_error("No thoughts configuration found.");
        return Ok(());
    }
    if !force && !is_mapped {
        print_validation_error("This repository is not in the thoughts configuration.");
        return Ok(());
    }

    println!(
        "{}",
        "Removing thoughts setup from current repository...".blue()
    );

    // Remove searchable directory if exists
    let searchable_dir = thoughts_dir.join("searchable");
    if searchable_dir.exists() {
        println!("{}", "Removing searchable directory...".bright_black());
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("chmod")
                .args(["-R", "755"])
                .arg(&searchable_dir)
                .output();
        }
        fs::remove_dir_all(&searchable_dir)?;
    }

    // Remove thoughts directory (symlinks only)
    println!(
        "{}",
        "Removing thoughts directory (symlinks only)...".bright_black()
    );
    fs::remove_dir_all(&thoughts_dir)?;

    // Remove from config if mapped
    if is_mapped && config_path.exists() {
        println!(
            "{}",
            "Removing repository from thoughts configuration...".bright_black()
        );
        remove_from_config(&config_path, &current_repo_str)?;
    }

    println!("{}", "✅ Thoughts removed from repository".green());

    // Show where content remains
    effective.as_ref().inspect(|e| print_safe_location(e));

    Ok(())
}
