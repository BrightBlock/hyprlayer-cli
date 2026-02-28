use anyhow::Result;
use colored::Colorize;

use crate::cli::ProfileShowArgs;

pub fn show(args: ProfileShowArgs) -> Result<()> {
    let ProfileShowArgs {
        name: profile_name,
        json,
        config,
    } = args;
    let (_, config_json) = config.load_raw()?;

    if json {
        let profile = config_json
            .get("thoughts")
            .and_then(|t| t.get("profiles"))
            .and_then(|p| p.get(&profile_name))
            .ok_or_else(|| anyhow::anyhow!("Profile \"{}\" not found", profile_name))?;

        println!("{}", serde_json::to_string_pretty(profile)?);
        return Ok(());
    }

    println!("{}", format!("Profile: {}", profile_name).blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    let profile = config_json
        .get("thoughts")
        .and_then(|t| t.get("profiles"))
        .and_then(|p| p.get(&profile_name))
        .ok_or_else(|| anyhow::anyhow!("Profile \"{}\" not found", profile_name))?;

    let get_str = |key: &str| profile.get(key).and_then(|v| v.as_str()).unwrap_or("N/A");

    println!("  Thoughts repository: {}", get_str("thoughtsRepo").cyan());
    println!("  Repos directory: {}", get_str("reposDir").cyan());
    println!("  Global directory: {}", get_str("globalDir").cyan());

    Ok(())
}
