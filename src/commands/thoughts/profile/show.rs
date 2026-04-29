use anyhow::Result;
use colored::Colorize;

use crate::cli::ProfileShowArgs;
use crate::commands::thoughts::backend_display::print_backend_block;

pub fn show(args: ProfileShowArgs) -> Result<()> {
    let ProfileShowArgs {
        name: profile_name,
        json,
        config,
    } = args;

    if json {
        let (_, config_json) = config.load_raw()?;
        let profile = config_json
            .get("thoughts")
            .and_then(|t| t.get("profiles"))
            .and_then(|p| p.get(&profile_name))
            .ok_or_else(|| anyhow::anyhow!("Profile \"{}\" not found", profile_name))?;

        println!("{}", serde_json::to_string_pretty(profile)?);
        return Ok(());
    }

    let hyprlayer_config = config
        .load_if_exists()?
        .ok_or_else(|| anyhow::anyhow!("No thoughts configuration found"))?;
    let thoughts = hyprlayer_config
        .thoughts
        .ok_or_else(|| anyhow::anyhow!("No thoughts configuration found"))?;
    let profile = thoughts
        .profiles
        .get(&profile_name)
        .ok_or_else(|| anyhow::anyhow!("Profile \"{}\" not found", profile_name))?;

    println!("  Backend: {}", profile.backend.kind().as_str().cyan());
    print_backend_block(&profile.backend, "  ", true);
    Ok(())
}
