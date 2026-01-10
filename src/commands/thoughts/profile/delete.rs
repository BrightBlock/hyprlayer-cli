use anyhow::Result;
use colored::Colorize;

use crate::cli::args::ConfigArgs;
use crate::config::{expand_path, get_default_config_path};
use std::fs;

pub fn delete(profile_name: String, force: bool, config: ConfigArgs) -> Result<()> {
    let config_path = config
        .config_file
        .clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    if !config_path.exists() {
        return Err(anyhow::anyhow!("No thoughts configuration found"));
    }

    let content = fs::read_to_string(&config_path)?;

    // Check if profile is in use
    if !force
        && let Some(thoughts) = serde_json::from_str::<serde_json::Value>(&content)?
            .get("thoughts")
            .and_then(|t| t.as_object())
        && let Some(repo_mappings) = thoughts.get("repo_mappings").and_then(|m| m.as_object())
    {
        for (repo, mapping) in repo_mappings {
            if let Some(profile) = mapping.get("profile").and_then(|p| p.as_str())
                && profile == profile_name
            {
                return Err(anyhow::anyhow!(
                    "Profile \"{}\" is in use by repository: {}. Use --force to delete anyway.",
                    profile_name,
                    repo
                ));
            }
        }
    }

    let mut config: serde_json::Value = serde_json::from_str(&content)?;
    let thoughts_config = config
        .get_mut("thoughts")
        .and_then(|t| t.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("No thoughts configuration"))?;

    let profiles = thoughts_config
        .get_mut("profiles")
        .and_then(|p| p.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("No profiles configured"))?;

    if !profiles.contains_key(&profile_name) {
        return Err(anyhow::anyhow!(
            "Profile \"{}\" does not exist",
            profile_name
        ));
    }

    profiles.remove(&profile_name);

    if profiles.is_empty() {
        thoughts_config.remove("profiles");
    }

    fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;

    println!(
        "{}",
        format!("✅ Profile \"{}\" deleted", profile_name).green()
    );

    Ok(())
}
