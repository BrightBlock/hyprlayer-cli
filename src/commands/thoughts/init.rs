use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, MAIN_SEPARATOR_STR as SEP};
use std::process::Command;

use crate::cli::args::ConfigArgs;
use crate::config::{
    ConfigFile, RepoMapping, ThoughtsConfig, expand_path, get_current_repo_path,
    get_default_config_path, get_default_thoughts_repo, get_repo_name_from_path,
    sanitize_directory_name,
};
use crate::git_ops::GitRepo;

const HOOK_VERSION: &str = "1";

pub fn init(
    force: bool,
    directory: Option<String>,
    profile: Option<String>,
    config: ConfigArgs,
) -> Result<()> {
    let current_repo = get_current_repo_path()?;

    // Check if we're in a git repository
    if !GitRepo::is_repo(&current_repo) {
        return Err(anyhow::anyhow!("Not in a git repository"));
    }

    // Load or create global config
    let config_path = config
        .config_file
        .clone()
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| get_default_config_path().unwrap());

    let mut config = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        let config_file: ConfigFile = serde_json::from_str(&content)?;
        config_file
            .thoughts
            .ok_or_else(|| anyhow::anyhow!("No thoughts configuration found"))?
    } else {
        // Create initial config
        let theme = ColorfulTheme::default();

        println!("{}", "=== Initial Thoughts Setup ===".blue());
        println!();

        let default_repo = get_default_thoughts_repo()?.display().to_string();
        let mut thoughts_repo: String = Input::with_theme(&theme)
            .with_prompt("Thoughts repository location")
            .default(default_repo.clone())
            .allow_empty(true)
            .interact()?;

        if thoughts_repo.is_empty() {
            thoughts_repo = default_repo;
        }

        println!();
        let repos_dir: String = Input::with_theme(&theme)
            .with_prompt("Directory name for repository-specific thoughts")
            .default("repos".to_string())
            .interact()?;

        let global_dir: String = Input::with_theme(&theme)
            .with_prompt("Directory name for global thoughts")
            .default("global".to_string())
            .interact()?;

        let default_user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let user = loop {
            let input: String = Input::with_theme(&theme)
                .with_prompt("Your username")
                .default(default_user.clone())
                .interact()?;

            if input.to_lowercase() != "global" {
                break input;
            }
            println!(
                "{}",
                "Username cannot be \"global\" as it's reserved for cross-project thoughts.".red()
            );
        };

        println!();
        println!("{}", "Creating thoughts structure:".yellow());
        println!("  {}{SEP}", thoughts_repo.cyan());
        println!(
            "    ├── {}{SEP}     {}",
            repos_dir.cyan(),
            "(project-specific thoughts)".bright_black()
        );
        println!(
            "    └── {}{SEP}    {}",
            global_dir.cyan(),
            "(cross-project thoughts)".bright_black()
        );
        println!();

        ThoughtsConfig {
            thoughts_repo: thoughts_repo.clone(),
            repos_dir,
            global_dir,
            user,
            repo_mappings: Default::default(),
            profiles: Default::default(),
        }
    };

    // Validate profile if specified
    if let Some(profile_name) = &profile
        && !config.profiles.contains_key(profile_name)
    {
        return Err(anyhow::anyhow!(
            "Profile \"{}\" does not exist",
            profile_name
        ));
    }

    // Check for existing setup
    let thoughts_dir = current_repo.join("thoughts");
    if thoughts_dir.exists() && !force {
        println!(
            "{}",
            "Thoughts directory already configured for this repository.".yellow()
        );
        let reconfigure: bool = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Do you want to reconfigure?")
            .default(false)
            .interact()?;

        if !reconfigure {
            println!("Setup cancelled.");
            return Ok(());
        }
    }

    // Determine profile config
    let (thoughts_repo, repos_dir, global_dir) = if let Some(profile_name) = &profile {
        let profile = config.profiles.get(profile_name).unwrap();
        (
            profile.thoughts_repo.clone(),
            profile.repos_dir.clone(),
            profile.global_dir.clone(),
        )
    } else {
        (
            config.thoughts_repo.clone(),
            config.repos_dir.clone(),
            config.global_dir.clone(),
        )
    };

    let expanded_repo = expand_path(&thoughts_repo);

    // Ensure thoughts repo exists
    if !expanded_repo.exists() {
        fs::create_dir_all(&expanded_repo)?;
        println!(
            "{}",
            format!("Created thoughts repository at {}", thoughts_repo.cyan()).green()
        );
    }

    // Create directory structure
    let repos_path = expanded_repo.join(&repos_dir);
    if !repos_path.exists() {
        fs::create_dir_all(&repos_path)?;
    }

    // Map current repository
    let mapped_name = if let Some(dir) = directory {
        let sanitized = sanitize_directory_name(&dir);
        if !repos_path.join(&sanitized).exists() {
            return Err(anyhow::anyhow!(
                "Directory \"{}\" not found in thoughts repository",
                sanitized
            ));
        }
        println!(
            "{}",
            format!(
                "✓ Using existing: {}{SEP}{}{SEP}{}",
                thoughts_repo.cyan(),
                repos_dir.cyan(),
                sanitized.cyan()
            )
            .green()
        );
        sanitized
    } else {
        let existing_repos: Vec<_> = fs::read_dir(&repos_path)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        if existing_repos.is_empty() {
            let default_name = get_repo_name_from_path(&current_repo);
            let input: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Directory name for this project's thoughts [{}]",
                    default_name
                ))
                .default(default_name)
                .interact()?;

            let sanitized = sanitize_directory_name(&input);
            println!(
                "{}",
                format!(
                    "✓ Will create: {}{SEP}{}{SEP}{}",
                    thoughts_repo.cyan(),
                    repos_dir.cyan(),
                    sanitized.cyan()
                )
                .green()
            );
            sanitized
        } else {
            let mut options = existing_repos
                .iter()
                .map(|r| format!("Use existing: {}", r))
                .collect::<Vec<_>>();
            options.push("→ Create new directory".to_string());

            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select or create a thoughts directory for this repository")
                .items(&options)
                .default(0)
                .interact()?;

            if selection == options.len() - 1 {
                // Create new
                let default_name = get_repo_name_from_path(&current_repo);
                let input: String = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Directory name for this project's thoughts [{}]",
                        default_name
                    ))
                    .default(default_name)
                    .interact()?;

                let sanitized = sanitize_directory_name(&input);
                println!(
                    "{}",
                    format!(
                        "✓ Will create: {}{SEP}{}{SEP}{}",
                        thoughts_repo.cyan(),
                        repos_dir.cyan(),
                        sanitized.cyan()
                    )
                    .green()
                );
                sanitized
            } else {
                existing_repos[selection].clone()
            }
        }
    };

    // Add to repo mappings
    let mapping = if let Some(profile_name) = &profile {
        RepoMapping::Object {
            repo: mapped_name.clone(),
            profile: Some(profile_name.clone()),
        }
    } else {
        RepoMapping::String(mapped_name.clone())
    };
    config
        .repo_mappings
        .insert(current_repo.display().to_string(), mapping);

    // Save config
    let config_dir = config_path.parent().expect("config_path parent");
    fs::create_dir_all(config_dir)?;
    let content = serde_json::json!({ "thoughts": config });
    fs::write(&config_path, serde_json::to_string_pretty(&content)?)?;
    println!("{}", "✅ Global thoughts configuration created".green());

    // Create directory structure
    let repo_thoughts_path = repos_path.join(&mapped_name);
    fs::create_dir_all(repo_thoughts_path.join(&config.user))?;
    fs::create_dir_all(repo_thoughts_path.join("shared"))?;

    let global_path = expanded_repo.join(&global_dir);
    fs::create_dir_all(global_path.join(&config.user))?;
    fs::create_dir_all(global_path.join("shared"))?;

    // Initialize git repo if needed
    if !GitRepo::is_repo(&expanded_repo) {
        let _ = GitRepo::init(&expanded_repo);
        let gitignore = r#"# OS files
.DS_Store
Thumbs.db

# Editor files
.vscode/
.idea/
*.swp
*.swo
*~

# Temporary files
*.tmp
*.bak
"#;
        fs::write(expanded_repo.join(".gitignore"), gitignore)?;

        // Initial commit
        let git_repo = GitRepo::open(&expanded_repo)?;
        git_repo.add_all()?;
        git_repo.commit("Initial thoughts repository setup")?;
    }

    // Create thoughts directory in current repo
    if thoughts_dir.exists() {
        // Remove existing
        std::fs::remove_dir_all(&thoughts_dir)?;
    }
    fs::create_dir(&thoughts_dir)?;

    // Create symlinks
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            repo_thoughts_path.join(&config.user),
            thoughts_dir.join(&config.user),
        )?;
        std::os::unix::fs::symlink(
            repo_thoughts_path.join("shared"),
            thoughts_dir.join("shared"),
        )?;
        std::os::unix::fs::symlink(&global_path, thoughts_dir.join("global"))?;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;

        let create_symlink = |target: &std::path::Path, link: &std::path::Path| -> Result<()> {
            symlink_dir(target, link).with_context(|| {
                format!(
                    "Failed to create symlink. On Windows, symlinks require either:\n\
                     1. Run as Administrator, or\n\
                     2. Enable Developer Mode in Settings > Update & Security > For developers\n\
                     \n\
                     Target: {}\n\
                     Link: {}",
                    target.display(),
                    link.display()
                )
            })
        };

        create_symlink(
            &repo_thoughts_path.join(&config.user),
            &thoughts_dir.join(&config.user),
        )?;
        create_symlink(
            &repo_thoughts_path.join("shared"),
            &thoughts_dir.join("shared"),
        )?;
        create_symlink(&global_path, &thoughts_dir.join("global"))?;
    }

    // Setup git hooks
    let hooks_updated = setup_git_hooks(&current_repo)?;
    if !hooks_updated.is_empty() {
        println!(
            "{}",
            format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow()
        );
    }

    println!("{}", "✅ Thoughts setup complete!".green());
    println!();
    println!("{}", "=== Summary ===".blue());
    println!();
    println!("Repository structure created:");
    println!("  {}{SEP}", current_repo.display().to_string().cyan());
    println!("    └── thoughts{SEP}");
    println!(
        "         ├── {}{SEP}     → {}{SEP}{}{SEP}{}{SEP}{}{SEP}",
        config.user.cyan(),
        thoughts_repo.cyan(),
        repos_dir.cyan(),
        mapped_name.cyan(),
        config.user.cyan(),
    );
    println!(
        "         ├── shared{SEP}      → {}{SEP}{}{SEP}{}{SEP}shared{SEP}",
        thoughts_repo.cyan(),
        repos_dir.cyan(),
        mapped_name.cyan(),
    );
    println!(
        "         └── global{SEP}      → {}{SEP}{}{SEP}",
        thoughts_repo.cyan(),
        global_dir.cyan(),
    );
    println!();
    println!("Protection enabled:");
    println!(
        "  {} Pre-commit hook: Prevents committing thoughts/",
        "✓".green()
    );
    println!(
        "  {} Post-commit hook: Auto-syncs thoughts after commits",
        "✓".green()
    );

    Ok(())
}

/// Check if a hook needs updating based on version
fn hook_needs_update(hook_path: &Path) -> bool {
    if !hook_path.exists() {
        return true;
    }

    let content = match fs::read_to_string(hook_path) {
        Ok(c) => c,
        Err(_) => return true,
    };

    // Not our hook
    if !content.contains("hyprlayer thoughts") {
        return false;
    }

    // Check version
    if let Some(version_line) = content.lines().find(|l| l.contains("# Version:"))
        && let Some(version) = version_line.split(':').nth(1)
    {
        let current_version: u32 = version.trim().parse().unwrap_or(0);
        let target_version: u32 = HOOK_VERSION.parse().unwrap_or(1);
        return current_version < target_version;
    }

    true // No version found, needs update
}

/// Setup git hooks for thoughts protection
fn setup_git_hooks(repo_path: &Path) -> Result<Vec<String>> {
    let mut updated = Vec::new();

    // Get git common dir (handles worktrees)
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output()
        .context("Failed to find git directory")?;

    let git_common_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let git_common_dir = if std::path::Path::new(&git_common_dir).is_absolute() {
        std::path::PathBuf::from(&git_common_dir)
    } else {
        repo_path.join(&git_common_dir)
    };

    let hooks_dir = git_common_dir.join("hooks");
    fs::create_dir_all(&hooks_dir)?;

    // Pre-commit hook - prevents committing thoughts/
    let pre_commit_path = hooks_dir.join("pre-commit");
    let pre_commit_content = format!(
        r#"#!/bin/bash
# hyprlayer thoughts protection - prevent committing thoughts directory
# Version: {}

if git diff --cached --name-only | grep -q "^thoughts/"; then
    echo "❌ Cannot commit thoughts/ to code repository"
    echo "The thoughts directory should only exist in your separate thoughts repository."
    git reset HEAD -- thoughts/
    exit 1
fi

# Call any existing pre-commit hook
if [ -f "{}.old" ]; then
    "{}.old" "$@"
fi
"#,
        HOOK_VERSION,
        pre_commit_path.display(),
        pre_commit_path.display()
    );

    // Post-commit hook - auto-syncs thoughts
    let post_commit_path = hooks_dir.join("post-commit");
    let post_commit_content = format!(
        r#"#!/bin/bash
# hyprlayer thoughts auto-sync
# Version: {}

# Check if we're in a worktree (skip auto-sync in worktrees)
if [ -f .git ]; then
    exit 0
fi

# Get the commit message
COMMIT_MSG=$(git log -1 --pretty=%B)

# Auto-sync thoughts after each commit (only in non-worktree repos)
hyprlayer thoughts sync --message "Auto-sync with commit: $COMMIT_MSG" >/dev/null 2>&1 &

# Call any existing post-commit hook
if [ -f "{}.old" ]; then
    "{}.old" "$@"
fi
"#,
        HOOK_VERSION,
        post_commit_path.display(),
        post_commit_path.display()
    );

    // Install pre-commit hook
    if hook_needs_update(&pre_commit_path) {
        // Backup existing non-hyprlayer hook
        if pre_commit_path.exists() {
            let content = fs::read_to_string(&pre_commit_path)?;
            if !content.contains("hyprlayer thoughts") {
                fs::rename(
                    &pre_commit_path,
                    format!("{}.old", pre_commit_path.display()),
                )?;
            }
        }

        fs::write(&pre_commit_path, pre_commit_content)?;
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&pre_commit_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&pre_commit_path, perms)?;
        }
        updated.push("pre-commit".to_string());
    }

    // Install post-commit hook
    if hook_needs_update(&post_commit_path) {
        // Backup existing non-hyprlayer hook
        if post_commit_path.exists() {
            let content = fs::read_to_string(&post_commit_path)?;
            if !content.contains("hyprlayer thoughts") {
                fs::rename(
                    &post_commit_path,
                    format!("{}.old", post_commit_path.display()),
                )?;
            }
        }

        fs::write(&post_commit_path, post_commit_content)?;
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&post_commit_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&post_commit_path, perms)?;
        }
        updated.push("post-commit".to_string());
    }

    Ok(updated)
}
