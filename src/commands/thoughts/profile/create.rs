use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Input;

use crate::config::{get_default_thoughts_repo, get_default_config_path, expand_path, sanitize_profile_name};
use crate::git_ops::GitRepo;
use std::fs;

#[derive(Parser, Debug)]
pub struct CreateOptions {
    #[arg(long, help = "Thoughts repository path")]
    pub repo: Option<String>,

    #[arg(long, help = "Repos directory name")]
    pub repos_dir: Option<String>,

    #[arg(long, help = "Global directory name")]
    pub global_dir: Option<String>,

    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

pub fn create(profile_name: String, options: CreateOptions) -> Result<()> {
    let config_path = options.config_file.clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    let content = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        "{}".to_string()
    };

    let mut config_json: serde_json::Value = serde_json::from_str(&content)?;

    // Get thoughts config
    let thoughts_config = config_json.get_mut("thoughts")
        .and_then(|t| t.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("Thoughts not configured"))?;

    // Sanitize profile name
    let sanitized_name = sanitize_profile_name(&profile_name);
    if sanitized_name != profile_name {
        println!("{}", format!("Profile name sanitized: \"{}\" → \"{}\"", profile_name, sanitized_name).yellow());
    }

    // Check if profile exists
    if let Some(profiles) = thoughts_config.get("profiles")
        && let Some(profiles_obj) = profiles.as_object()
            && profiles_obj.contains_key(&sanitized_name) {
                return Err(anyhow::anyhow!("Profile \"{}\" already exists", sanitized_name));
            }

    // Get or create profiles object
    let profiles = thoughts_config.get_mut("profiles")
        .and_then(|p| p.as_object_mut());

    let (thoughts_repo, repos_dir, global_dir) = if options.repo.is_some() && options.repos_dir.is_some() && options.global_dir.is_some() {
        (
            options.repo.unwrap(),
            options.repos_dir.unwrap(),
            options.global_dir.unwrap(),
        )
    } else {
        let theme = ColorfulTheme::default();

        println!("{}", format!("\n=== Creating Profile: {} ===\n", sanitized_name).blue());

        let default_repo = format!("{}/{}", get_default_thoughts_repo()?.display(), sanitized_name);
        let thoughts_repo: String = Input::with_theme(&theme)
            .with_prompt("Thoughts repository")
            .default(default_repo.clone())
            .allow_empty(true)
            .interact()?;

        let thoughts_repo = if thoughts_repo.is_empty() { default_repo } else { thoughts_repo };

        println!();
        let repos_dir: String = Input::with_theme(&theme)
            .with_prompt("Repository-specific thoughts directory")
            .default("repos".to_string())
            .interact()?;

        let global_dir: String = Input::with_theme(&theme)
            .with_prompt("Global thoughts directory")
            .default("global".to_string())
            .interact()?;

        (thoughts_repo, repos_dir, global_dir)
    };

    // Create profile object
    let profile = serde_json::json!({
        "thoughtsRepo": thoughts_repo,
        "reposDir": repos_dir,
        "globalDir": global_dir,
    });

    // Add to profiles
    match profiles {
        Some(p) => {
            p.insert(sanitized_name.clone(), profile);
        }
        None => {
            let mut new_profiles = serde_json::Map::new();
            new_profiles.insert(sanitized_name.clone(), profile);
            thoughts_config.insert("profiles".to_string(), serde_json::Value::Object(new_profiles));
        }
    }

    // Save config
    let config_dir = config_path.parent().unwrap();
    fs::create_dir_all(config_dir)?;
    fs::write(&config_path, serde_json::to_string_pretty(&config_json)?)?;

    // Initialize profile's thoughts repository
    println!("{}", "\nInitializing profile thoughts repository...".bright_black());
    let expanded_repo = expand_path(&thoughts_repo);
    fs::create_dir_all(&expanded_repo)?;
    if !GitRepo::is_repo(&expanded_repo) {
        let _ = GitRepo::init(&expanded_repo);
    }

    println!("{}", format!("\n✅ Profile \"{}\" created successfully!", sanitized_name).green());
    println!();
    println!("{}", "=== Profile Configuration ===".blue());
    println!("  Name: {}", sanitized_name.cyan());
    println!("  Thoughts repository: {}", thoughts_repo.cyan());
    println!("  Repos directory: {}", repos_dir.cyan());
    println!("  Global directory: {}", global_dir.cyan());
    println!();
    println!("{}", "Next steps:".bright_black());
    println!("  1. Run \"hyprlayer thoughts init --profile {}\" in a repository", sanitized_name.cyan());
    println!("  2. Your thoughts will sync to to profile's repository");

    Ok(())
}
