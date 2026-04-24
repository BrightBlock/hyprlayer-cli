use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::process::Command;

use crate::cli::ConfigArgsCmd;

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

    // Display configuration
    println!("{}", "Thoughts Configuration".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();
    println!("{}", "Settings:".yellow());
    println!(
        "  Config file: {}",
        config_path.display().to_string().cyan()
    );

    if !config_path.exists() {
        println!("  {}", "No configuration found".bright_black());
        return Ok(());
    }

    let content = fs::read_to_string(&config_path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;

    let Some(thoughts) = config.get("thoughts") else {
        return Ok(());
    };

    let get_str = |key: &str| thoughts.get(key).and_then(|v| v.as_str()).unwrap_or("N/A");
    println!("  Thoughts repository: {}", get_str("thoughtsRepo").cyan());
    println!("  Repos directory: {}", get_str("reposDir").cyan());
    println!("  Global directory: {}", get_str("globalDir").cyan());
    println!("  User: {}", get_str("user").cyan());
    println!("  Backend: {}", get_str("backend").cyan());

    if let Some(settings) = thoughts.get("backendSettings").and_then(|s| s.as_object())
        && !settings.is_empty()
    {
        println!();
        println!("{}", "Backend settings:".yellow());
        print_backend_settings(settings);
    }

    if let Some(profiles) = thoughts.get("profiles").and_then(|p| p.as_object())
        && !profiles.is_empty()
    {
        println!();
        println!("{}", "Profiles:".yellow());
        for (name, profile) in profiles {
            println!("  {}", name.cyan());
            let Some(obj) = profile.as_object() else {
                continue;
            };
            if let Some(backend) = obj.get("backend").and_then(|v| v.as_str()) {
                println!("    Backend: {}", backend.green());
            }
            if let Some(settings) = obj.get("backendSettings").and_then(|s| s.as_object())
                && !settings.is_empty()
            {
                for (key, val) in settings {
                    let display = super::format_backend_setting(key, val);
                    println!("    {}: {}", key, display);
                }
            }
        }
    }

    let Some(mappings) = thoughts.get("repoMappings").and_then(|m| m.as_object()) else {
        return Ok(());
    };

    println!();
    println!("{}", "Repository Mappings:".yellow());
    if mappings.is_empty() {
        println!("  {}", "No repositories mapped yet".bright_black());
    } else {
        for (repo, mapping) in mappings {
            println!("  {}", repo.cyan());
            mapping
                .get("repo")
                .and_then(|r| r.as_str())
                .inspect(|name| println!("    → {}", name.green()));
        }
    }

    println!();
    println!(
        "{}",
        "To edit configuration, run: hyprlayer thoughts config --edit".bright_black()
    );

    Ok(())
}

fn print_backend_settings(settings: &serde_json::Map<String, serde_json::Value>) {
    for (key, val) in settings {
        let display = super::format_backend_setting(key, val);
        println!("  {}: {}", key, display.cyan());
    }
}
