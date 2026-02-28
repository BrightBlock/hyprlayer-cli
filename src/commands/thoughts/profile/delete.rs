use anyhow::Result;
use colored::Colorize;
use std::fs;

use crate::cli::ProfileDeleteArgs;

fn check_profile_not_in_use(config: &serde_json::Value, profile_name: &str) -> Result<()> {
    let repo_mappings = config
        .get("thoughts")
        .and_then(|t| t.get("repoMappings"))
        .and_then(|m| m.as_object());

    let Some(mappings) = repo_mappings else {
        return Ok(());
    };

    let in_use_repo = mappings.iter().find(|(_, mapping)| {
        mapping
            .get("profile")
            .and_then(|p| p.as_str())
            .is_some_and(|p| p == profile_name)
    });

    match in_use_repo {
        Some((repo, _)) => Err(anyhow::anyhow!(
            "Profile \"{}\" is in use by repository: {}. Use --force to delete anyway.",
            profile_name,
            repo
        )),
        None => Ok(()),
    }
}

pub fn delete(args: ProfileDeleteArgs) -> Result<()> {
    let ProfileDeleteArgs {
        name: profile_name,
        force,
        config,
    } = args;
    let (config_path, mut config_json) = config.load_raw()?;

    // Check if profile is in use (unless force)
    if !force {
        check_profile_not_in_use(&config_json, &profile_name)?;
    }
    let thoughts_obj = config_json
        .get_mut("thoughts")
        .and_then(|t| t.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("No thoughts configuration"))?;

    let profiles = thoughts_obj
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
        thoughts_obj.remove("profiles");
    }

    fs::write(&config_path, serde_json::to_string_pretty(&config_json)?)?;

    println!(
        "{}",
        format!("✅ Profile \"{}\" deleted", profile_name).green()
    );

    Ok(())
}
