use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::process::Command;

use crate::cli::ConfigArgsCmd;
use crate::commands::thoughts::backend_display::print_backend_block;
use crate::config::HyprlayerConfig;

pub fn config(args: ConfigArgsCmd) -> Result<()> {
    let ConfigArgsCmd { edit, json, config } = args;
    let config_path = config.path()?;

    if edit {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });
        Command::new(&editor).arg(&config_path).status()?;
        return Ok(());
    }

    if json {
        let content = fs::read_to_string(&config_path)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&content)?)?
        );
        return Ok(());
    }

    println!("{}", "Settings:".yellow());
    println!(
        "  Config file: {}",
        config_path.display().to_string().cyan()
    );

    if !config_path.exists() {
        println!("  {}", "No configuration found".bright_black());
        return Ok(());
    }

    let hyprlayer_config = HyprlayerConfig::load(&config_path)?;
    let Some(thoughts) = hyprlayer_config.thoughts else {
        return Ok(());
    };

    println!("  User: {}", thoughts.user.cyan());
    println!("  Backend: {}", thoughts.backend.kind().as_str().cyan());
    print_backend_block(&thoughts.backend, "  ", true);

    if !thoughts.profiles.is_empty() {
        println!();
        println!("{}", "Profiles:".yellow());
        for (name, profile) in &thoughts.profiles {
            println!("  {}", name.cyan());
            println!("    Backend: {}", profile.backend.kind().as_str().green());
            print_backend_block(&profile.backend, "    ", true);
        }
    }

    println!();
    println!("{}", "Repository Mappings:".yellow());
    if thoughts.repo_mappings.is_empty() {
        println!("  {}", "No repositories mapped yet".bright_black());
    } else {
        for (repo, mapping) in &thoughts.repo_mappings {
            println!("  {}", repo.cyan());
            println!("    → {}", mapping.repo().green());
        }
    }

    println!();
    println!(
        "{}",
        "To edit configuration, run: hyprlayer thoughts config --edit".bright_black()
    );

    Ok(())
}
