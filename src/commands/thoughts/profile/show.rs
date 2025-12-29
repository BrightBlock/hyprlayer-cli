use anyhow::Result;
use clap::Parser;
use colored::Colorize;

use crate::config::{get_default_config_path, expand_path};
use std::fs;

#[derive(Parser, Debug)]
pub struct ShowOptions {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,

    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn show(profile_name: String, options: ShowOptions) -> Result<()> {
    let config_path = options.config_file.clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    if !config_path.exists() {
        return Err(anyhow::anyhow!("No thoughts configuration found"));
    }

    let content = fs::read_to_string(&config_path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;

    if options.json {
        let profile = config.get("thoughts")
            .and_then(|t| t.get("profiles"))
            .and_then(|p| p.get(&profile_name))
            .ok_or_else(|| anyhow::anyhow!("Profile \"{}\" not found", profile_name))?;

        println!("{}", serde_json::to_string_pretty(profile)?);
        return Ok(());
    }

    println!("{}", format!("Profile: {}", profile_name).blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    if let Some(profile) = config.get("thoughts")
        .and_then(|t| t.get("profiles"))
        .and_then(|p| p.get(&profile_name)) {
        if let Some(tr) = profile.get("thoughtsRepo") {
            println!("  Thoughts repository: {}", tr.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(rd) = profile.get("reposDir") {
            println!("  Repos directory: {}", rd.as_str().unwrap_or("N/A").cyan());
        }
        if let Some(gd) = profile.get("globalDir") {
            println!("  Global directory: {}", gd.as_str().unwrap_or("N/A").cyan());
        }
    } else {
        return Err(anyhow::anyhow!("Profile \"{}\" not found", profile_name));
    }

    Ok(())
}
