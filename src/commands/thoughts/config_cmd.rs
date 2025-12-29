use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use std::process::Command;

use crate::config::{get_default_config_path, expand_path};
use std::fs;

#[derive(Parser, Debug)]
pub struct ConfigOptions {
    #[arg(long, help = "Open configuration in editor")]
    pub edit: bool,

    #[arg(long, help = "Output configuration as JSON")]
    pub json: bool,

    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn config(options: ConfigOptions) -> Result<()> {
    let config_path = options.config_file.clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    if options.edit {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        Command::new(&editor)
            .arg(&config_path)
            .status()?;
        return Ok(());
    }

    if options.json {
        let content = fs::read_to_string(&config_path)?;
        println!("{}", serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&content)?)?);
        return Ok(());
    }

    // Display configuration
    println!("{}", "Thoughts Configuration".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();
    println!("{}", "Settings:".yellow());
    println!("  Config file: {}", config_path.display().to_string().cyan());

    if !config_path.exists() {
        println!("  {}", "No configuration found".bright_black());
        return Ok(());
    }

    let content = fs::read_to_string(&config_path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;

    if let Some(thoughts) = config.get("thoughts") {
        if let Some(tr) = thoughts.get("thoughts_repo") {
            println!("  Thoughts repository: {}", tr.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(rd) = thoughts.get("repos_dir") {
            println!("  Repos directory: {}", rd.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(gd) = thoughts.get("global_dir") {
            println!("  Global directory: {}", gd.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(u) = thoughts.get("user") {
            println!("  User: {}", u.as_str().unwrap_or("N/A").cyan());
        }

        if let Some(mappings) = thoughts.get("repo_mappings").and_then(|m| m.as_object()) {
            println!();
            println!("{}", "Repository Mappings:".yellow());
            if mappings.is_empty() {
                println!("  {}", "No repositories mapped yet".bright_black());
            } else {
                for (repo, mapping) in mappings {
                    println!("  {}", repo.cyan());
                    if let Some(repo_name) = mapping.get("repo").and_then(|r| r.as_str()) {
                        println!("    → {}", repo_name.green());
                    }
                }
            }
        }
    }

    println!();
    println!("{}", "To edit configuration, run: hyprlayer thoughts config --edit".bright_black());

    Ok(())
}
