use anyhow::Result;
use colored::Colorize;
use dialoguer::Input;
use dialoguer::theme::ColorfulTheme;
use std::fs;
use std::path::MAIN_SEPARATOR_STR as SEP;

use crate::cli::ProfileCreateArgs;
use crate::config::{expand_path, get_default_thoughts_repo, sanitize_directory_name};
use crate::git_ops::GitRepo;

fn prompt_for_profile_config(profile_name: &str) -> Result<(String, String, String)> {
    let theme = ColorfulTheme::default();

    println!(
        "{}",
        format!("\n=== Creating Profile: {} ===\n", profile_name).blue()
    );

    let default_repo = format!(
        "{}{SEP}{}",
        get_default_thoughts_repo()?.display(),
        profile_name
    );
    let thoughts_repo: String = Input::with_theme(&theme)
        .with_prompt("Thoughts repository")
        .default(default_repo.clone())
        .allow_empty(true)
        .interact()
        .map(|s: String| if s.is_empty() { default_repo } else { s })?;

    println!();
    let repos_dir: String = Input::with_theme(&theme)
        .with_prompt("Repository-specific thoughts directory")
        .default("repos".to_string())
        .interact()?;

    let global_dir: String = Input::with_theme(&theme)
        .with_prompt("Global thoughts directory")
        .default("global".to_string())
        .interact()?;

    Ok((thoughts_repo, repos_dir, global_dir))
}

pub fn create(args: ProfileCreateArgs) -> Result<()> {
    let ProfileCreateArgs {
        name: profile_name,
        repo,
        repos_dir,
        global_dir,
        config,
    } = args;
    let config_path = config.path()?;

    let content = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        "{}".to_string()
    };

    let mut config_json: serde_json::Value = serde_json::from_str(&content)?;

    // Get thoughts config
    let thoughts_config = config_json
        .get_mut("thoughts")
        .and_then(|t| t.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("Thoughts not configured"))?;

    // Sanitize profile name
    let sanitized_name = sanitize_directory_name(&profile_name);
    if sanitized_name != profile_name {
        println!(
            "{}",
            format!(
                "Profile name sanitized: \"{}\" → \"{}\"",
                profile_name, sanitized_name
            )
            .yellow()
        );
    }

    // Check if profile exists
    let profile_exists = thoughts_config
        .get("profiles")
        .and_then(|p| p.as_object())
        .is_some_and(|obj| obj.contains_key(&sanitized_name));

    if profile_exists {
        return Err(anyhow::anyhow!(
            "Profile \"{}\" already exists",
            sanitized_name
        ));
    }

    // Get or create profiles object
    let profiles = thoughts_config
        .get_mut("profiles")
        .and_then(|p| p.as_object_mut());

    let (thoughts_repo, repos_dir, global_dir) = match (repo, repos_dir, global_dir) {
        (Some(r), Some(rd), Some(gd)) => (r, rd, gd),
        _ => prompt_for_profile_config(&sanitized_name)?,
    };

    // Create profile object
    let profile = serde_json::json!({
        "thoughtsRepo": thoughts_repo,
        "reposDir": repos_dir,
        "globalDir": global_dir,
    });

    // Add to profiles
    if let Some(p) = profiles {
        p.insert(sanitized_name.clone(), profile);
    } else {
        thoughts_config.insert(
            "profiles".to_string(),
            serde_json::json!({ sanitized_name.clone(): profile }),
        );
    }

    // Save config
    let config_dir = config_path.parent().unwrap();
    fs::create_dir_all(config_dir)?;
    fs::write(&config_path, serde_json::to_string_pretty(&config_json)?)?;

    // Initialize profile's thoughts repository
    println!(
        "{}",
        "\nInitializing profile thoughts repository...".bright_black()
    );
    let expanded_repo = expand_path(&thoughts_repo);
    fs::create_dir_all(&expanded_repo)?;
    if !GitRepo::is_repo(&expanded_repo) {
        let _ = GitRepo::init(&expanded_repo);
    }

    println!(
        "{}",
        format!("\n✅ Profile \"{}\" created successfully!", sanitized_name).green()
    );
    println!();
    println!("{}", "=== Profile Configuration ===".blue());
    println!("  Name: {}", sanitized_name.cyan());
    println!("  Thoughts repository: {}", thoughts_repo.cyan());
    println!("  Repos directory: {}", repos_dir.cyan());
    println!("  Global directory: {}", global_dir.cyan());
    println!();
    println!("{}", "Next steps:".bright_black());
    println!(
        "  1. Run \"hyprlayer thoughts init --profile {}\" in a repository",
        sanitized_name.cyan()
    );
    println!("  2. Your thoughts will sync to the profile's repository");

    Ok(())
}
