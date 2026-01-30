use anyhow::Result;
use colored::Colorize;
use std::fs;

use crate::cli::ProfileListArgs;

pub fn list(args: ProfileListArgs) -> Result<()> {
    let ProfileListArgs { json, config } = args;
    let config_path = config.path()?;

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "No thoughts configuration found. Run 'hyprlayer thoughts init' first."
        ));
    }

    let content = fs::read_to_string(&config_path)?;
    let config_json: serde_json::Value = serde_json::from_str(&content)?;

    if json {
        let profiles = config_json
            .get("thoughts")
            .and_then(|t| t.get("profiles"))
            .unwrap_or(&serde_json::Value::Null);
        println!("{}", serde_json::to_string_pretty(profiles)?);
        return Ok(());
    }

    println!("{}", "Thoughts Profiles".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    let Some(thoughts) = config_json.get("thoughts") else {
        return Ok(());
    };

    let get_str = |key: &str| thoughts.get(key).and_then(|v| v.as_str()).unwrap_or("N/A");

    println!("{}", "Default Configuration:".yellow());
    println!("  Thoughts repository: {}", get_str("thoughtsRepo").cyan());
    println!("  Repos directory: {}", get_str("reposDir").cyan());
    println!("  Global directory: {}", get_str("globalDir").cyan());
    println!();

    let Some(profiles) = thoughts.get("profiles").and_then(|p| p.as_object()) else {
        return Ok(());
    };

    if profiles.is_empty() {
        println!("{}", "No profiles configured.".bright_black());
        println!();
        println!(
            "{}",
            "Create a profile with: hyprlayer thoughts profile create <name>".bright_black()
        );
        return Ok(());
    }

    println!("{}", format!("Profiles ({}):", profiles.len()).yellow());
    println!();

    for (name, profile) in profiles {
        let get_profile_str =
            |key: &str| profile.get(key).and_then(|v| v.as_str()).unwrap_or("N/A");

        println!("  {}:", name.cyan());
        println!("    Thoughts repository: {}", get_profile_str("thoughtsRepo"));
        println!("    Repos directory: {}", get_profile_str("reposDir"));
        println!("    Global directory: {}", get_profile_str("globalDir"));
        println!();
    }

    Ok(())
}
