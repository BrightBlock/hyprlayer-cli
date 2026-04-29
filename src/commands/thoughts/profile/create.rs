use anyhow::Result;
use colored::Colorize;
use dialoguer::Input;
use dialoguer::theme::ColorfulTheme;
use std::fs;
use std::path::MAIN_SEPARATOR_STR as SEP;

use crate::cli::ProfileCreateArgs;
use crate::config::{
    BackendConfig, GitConfig, HyprlayerConfig, ProfileConfig, expand_path,
    get_default_thoughts_repo, sanitize_directory_name,
};
use crate::git_ops::GitRepo;

fn prompt_for_profile_config(profile_name: &str) -> Result<(String, String, String)> {
    let theme = ColorfulTheme::default();

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

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not configured. Run 'hyprlayer thoughts init' first."
        ));
    }

    let mut hyprlayer_config = HyprlayerConfig::load(&config_path)?;
    let thoughts = hyprlayer_config
        .thoughts
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Thoughts not configured"))?;

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

    if thoughts.profiles.contains_key(&sanitized_name) {
        return Err(anyhow::anyhow!(
            "Profile \"{}\" already exists",
            sanitized_name
        ));
    }

    let (thoughts_repo, repos_dir, global_dir) = match (repo, repos_dir, global_dir) {
        (Some(r), Some(rd), Some(gd)) => (r, rd, gd),
        _ => prompt_for_profile_config(&sanitized_name)?,
    };

    let profile = ProfileConfig {
        backend: BackendConfig::Git(GitConfig {
            thoughts_repo: thoughts_repo.clone(),
            repos_dir,
            global_dir,
        }),
    };
    thoughts.profiles.insert(sanitized_name.clone(), profile);

    hyprlayer_config.save(&config_path)?;

    let expanded_repo = expand_path(&thoughts_repo);
    fs::create_dir_all(&expanded_repo)?;
    if !GitRepo::is_repo(&expanded_repo) {
        let _ = GitRepo::init(&expanded_repo);
    }

    Ok(())
}
