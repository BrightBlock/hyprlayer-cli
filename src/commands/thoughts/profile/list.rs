use anyhow::Result;
use clap::Parser;
use colored::Colorize;

use crate::config::{get_default_config_path, expand_path};
use std::fs;

#[derive(Parser, Debug)]
pub struct ListOptions {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,

    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn list(options: ListOptions) -> Result<()> {
    let config_path = options.config_file.clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    if !config_path.exists() {
        return Err(anyhow::anyhow!("No thoughts configuration found. Run 'hyprlayer thoughts init' first."));
    }

    let content = fs::read_to_string(&config_path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;

    if options.json {
        let profiles = config.get("thoughts")
            .and_then(|t| t.get("profiles"))
            .unwrap_or(&serde_json::Value::Null);
        println!("{}", serde_json::to_string_pretty(profiles)?);
        return Ok(());
    }

    println!("{}", "Thoughts Profiles".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    if let Some(thoughts) = config.get("thoughts") {
        println!("{}", "Default Configuration:".yellow());
        if let Some(tr) = thoughts.get("thoughts_repo") {
            println!("  Thoughts repository: {}", tr.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(rd) = thoughts.get("repos_dir") {
            println!("  Repos directory: {}", rd.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(gd) = thoughts.get("global_dir") {
            println!("  Global directory: {}", gd.as_str().unwrap_or("N/A").cyan());
        }
        println!();

        if let Some(profiles) = thoughts.get("profiles").and_then(|p| p.as_object()) {
            if profiles.is_empty() {
                println!("{}", "No profiles configured.".bright_black());
                println!();
                println!("{}", "Create a profile with: hyprlayer thoughts profile create <name>".bright_black());
            } else {
                println!("{}", format!("Profiles ({}):", profiles.len()).yellow());
                println!();

                for (name, profile) in profiles {
                    println!("  {}:", name.cyan());
                    if let Some(tr) = profile.get("thoughtsRepo") {
                        println!("    Thoughts repository: {}", tr.as_str().unwrap_or("N/A"));
                    }
                    if let Some(rd) = profile.get("reposDir") {
                        println!("    Repos directory: {}", rd.as_str().unwrap_or("N/A"));
                    }
                    if let Some(gd) = profile.get("globalDir") {
                        println!("    Global directory: {}", gd.as_str().unwrap_or("N/A"));
                    }
                    println!();
                }
            }
        }
    }

    Ok(())
}
