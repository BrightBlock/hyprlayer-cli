use anyhow::Result;
use colored::Colorize;

use crate::cli::ProfileListArgs;
use crate::commands::thoughts::backend_display::print_backend_block;

pub fn list(args: ProfileListArgs) -> Result<()> {
    let ProfileListArgs { json, config } = args;
    let (_, config_json) = config.load_raw()?;

    if json {
        let profiles = config_json
            .get("thoughts")
            .and_then(|t| t.get("profiles"))
            .unwrap_or(&serde_json::Value::Null);
        println!("{}", serde_json::to_string_pretty(profiles)?);
        return Ok(());
    }

    let hyprlayer_config = config.load_if_exists()?;
    let Some(thoughts) = hyprlayer_config.as_ref().and_then(|c| c.thoughts.as_ref()) else {
        return Ok(());
    };

    println!("{}", "Default Configuration:".yellow());
    println!("  Backend: {}", thoughts.backend.kind().as_str().cyan());
    print_backend_block(&thoughts.backend, "  ", false);
    println!();

    if thoughts.profiles.is_empty() {
        println!("{}", "No profiles configured.".bright_black());
        println!();
        println!(
            "{}",
            "Create a profile with: hyprlayer thoughts profile create <name>".bright_black()
        );
        return Ok(());
    }

    println!(
        "{}",
        format!("Profiles ({}):", thoughts.profiles.len()).yellow()
    );
    println!();

    for (name, profile) in &thoughts.profiles {
        println!("  {}:", name.cyan());
        println!("    Backend: {}", profile.backend.kind().as_str().cyan());
        print_backend_block(&profile.backend, "    ", false);
        println!();
    }

    Ok(())
}
