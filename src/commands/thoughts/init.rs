use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR as SEP};
use std::process::Command;

use crate::cli::InitArgs;
use crate::config::{
    RepoMapping, ThoughtsConfig, expand_path, get_current_repo_path,
    get_default_thoughts_repo, get_repo_name_from_path, sanitize_directory_name,
};
use crate::git_ops::GitRepo;

const HOOK_VERSION: &str = "1";

/// Context for the init operation
struct InitContext {
    current_repo: PathBuf,
    thoughts_config: ThoughtsConfig,
    thoughts_repo: String,
    repos_dir: String,
    global_dir: String,
    expanded_repo: PathBuf,
    mapped_name: String,
}

pub fn init(args: InitArgs) -> Result<()> {
    let InitArgs { force, directory, profile, config } = args;
    let current_repo = get_current_repo_path()?;

    if !GitRepo::is_repo(&current_repo) {
        return Err(anyhow::anyhow!("Not in a git repository"));
    }

    let config_path = config.path()?;
    let mut thoughts_config = load_or_create_config(&config)?;

    validate_profile(&thoughts_config, &profile)?;

    if !check_existing_setup(&current_repo, force)? {
        return Ok(());
    }

    let (thoughts_repo, repos_dir, global_dir) = resolve_effective_config(&thoughts_config, &profile);
    let expanded_repo = expand_path(&thoughts_repo);

    ensure_repo_exists(&expanded_repo, &thoughts_repo)?;

    let repos_path = expanded_repo.join(&repos_dir);
    fs::create_dir_all(&repos_path)?;

    let mapped_name = select_or_create_directory(&repos_path, &current_repo, directory, &thoughts_repo, &repos_dir)?;

    // Update config with mapping
    let mapping = create_mapping(&mapped_name, &profile);
    thoughts_config.repo_mappings.insert(current_repo.display().to_string(), mapping);
    thoughts_config.save(&config_path)?;
    println!("{}", "✅ Global thoughts configuration saved".green());

    let ctx = InitContext {
        current_repo,
        thoughts_config,
        thoughts_repo,
        repos_dir,
        global_dir,
        expanded_repo,
        mapped_name,
    };

    setup_directory_structure(&ctx)?;
    initialize_git_if_needed(&ctx)?;
    setup_symlinks(&ctx)?;
    setup_hooks_and_print_summary(&ctx)?;

    Ok(())
}

fn load_or_create_config(config: &crate::cli::ConfigArgs) -> Result<ThoughtsConfig> {
    if let Some(existing) = config.load_if_exists()? {
        return Ok(existing);
    }

    let theme = ColorfulTheme::default();
    println!("{}", "=== Initial Thoughts Setup ===".blue());
    println!();

    let default_repo = get_default_thoughts_repo()?.display().to_string();
    let thoughts_repo: String = Input::with_theme(&theme)
        .with_prompt("Thoughts repository location")
        .default(default_repo.clone())
        .allow_empty(true)
        .interact()
        .map(|s: String| if s.is_empty() { default_repo } else { s })?;

    println!();
    let repos_dir: String = Input::with_theme(&theme)
        .with_prompt("Directory name for repository-specific thoughts")
        .default("repos".to_string())
        .interact()?;

    let global_dir: String = Input::with_theme(&theme)
        .with_prompt("Directory name for global thoughts")
        .default("global".to_string())
        .interact()?;

    let user = prompt_for_username(&theme)?;

    println!();
    println!("{}", "Creating thoughts structure:".yellow());
    println!("  {}{SEP}", thoughts_repo.cyan());
    println!("    ├── {}{SEP}     {}", repos_dir.cyan(), "(project-specific thoughts)".bright_black());
    println!("    └── {}{SEP}    {}", global_dir.cyan(), "(cross-project thoughts)".bright_black());
    println!();

    Ok(ThoughtsConfig {
        thoughts_repo,
        repos_dir,
        global_dir,
        user,
        repo_mappings: Default::default(),
        profiles: Default::default(),
    })
}

fn prompt_for_username(theme: &ColorfulTheme) -> Result<String> {
    let default_user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string());

    loop {
        let input: String = Input::with_theme(theme)
            .with_prompt("Your username")
            .default(default_user.clone())
            .interact()?;

        if input.to_lowercase() != "global" {
            return Ok(input);
        }
        println!("{}", "Username cannot be \"global\" as it's reserved for cross-project thoughts.".red());
    }
}

fn validate_profile(config: &ThoughtsConfig, profile: &Option<String>) -> Result<()> {
    if let Some(name) = profile {
        if !config.profiles.contains_key(name) {
            return Err(anyhow::anyhow!("Profile \"{}\" does not exist", name));
        }
    }
    Ok(())
}

fn check_existing_setup(current_repo: &Path, force: bool) -> Result<bool> {
    let thoughts_dir = current_repo.join("thoughts");
    if !thoughts_dir.exists() || force {
        return Ok(true);
    }

    println!("{}", "Thoughts directory already configured for this repository.".yellow());
    let reconfigure: bool = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Do you want to reconfigure?")
        .default(false)
        .interact()?;

    if !reconfigure {
        println!("Setup cancelled.");
    }
    Ok(reconfigure)
}

fn resolve_effective_config(config: &ThoughtsConfig, profile: &Option<String>) -> (String, String, String) {
    profile
        .as_ref()
        .and_then(|name| config.profiles.get(name))
        .map(|p| (p.thoughts_repo.clone(), p.repos_dir.clone(), p.global_dir.clone()))
        .unwrap_or_else(|| (config.thoughts_repo.clone(), config.repos_dir.clone(), config.global_dir.clone()))
}

fn ensure_repo_exists(expanded_repo: &Path, thoughts_repo: &str) -> Result<()> {
    if !expanded_repo.exists() {
        fs::create_dir_all(expanded_repo)?;
        println!("{}", format!("Created thoughts repository at {}", thoughts_repo.cyan()).green());
    }
    Ok(())
}

fn select_or_create_directory(
    repos_path: &Path,
    current_repo: &Path,
    directory: Option<String>,
    thoughts_repo: &str,
    repos_dir: &str,
) -> Result<String> {
    if let Some(dir) = directory {
        return use_existing_directory(repos_path, &dir, thoughts_repo, repos_dir);
    }

    let existing_repos = list_existing_repos(repos_path)?;

    if existing_repos.is_empty() {
        prompt_for_new_directory(current_repo, thoughts_repo, repos_dir)
    } else {
        select_or_create_from_existing(&existing_repos, current_repo, thoughts_repo, repos_dir)
    }
}

fn use_existing_directory(repos_path: &Path, dir: &str, thoughts_repo: &str, repos_dir: &str) -> Result<String> {
    let sanitized = sanitize_directory_name(dir);
    if !repos_path.join(&sanitized).exists() {
        return Err(anyhow::anyhow!("Directory \"{}\" not found in thoughts repository", sanitized));
    }
    println!("{}", format!("✓ Using existing: {}{SEP}{}{SEP}{}", thoughts_repo.cyan(), repos_dir.cyan(), sanitized.cyan()).green());
    Ok(sanitized)
}

fn list_existing_repos(repos_path: &Path) -> Result<Vec<String>> {
    Ok(fs::read_dir(repos_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect())
}

fn prompt_for_new_directory(current_repo: &Path, thoughts_repo: &str, repos_dir: &str) -> Result<String> {
    let default_name = get_repo_name_from_path(current_repo);
    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Directory name for this project's thoughts [{}]", default_name))
        .default(default_name)
        .interact()?;

    let sanitized = sanitize_directory_name(&input);
    println!("{}", format!("✓ Will create: {}{SEP}{}{SEP}{}", thoughts_repo.cyan(), repos_dir.cyan(), sanitized.cyan()).green());
    Ok(sanitized)
}

fn select_or_create_from_existing(
    existing_repos: &[String],
    current_repo: &Path,
    thoughts_repo: &str,
    repos_dir: &str,
) -> Result<String> {
    let mut options: Vec<String> = existing_repos.iter().map(|r| format!("Use existing: {}", r)).collect();
    options.push("→ Create new directory".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select or create a thoughts directory for this repository")
        .items(&options)
        .default(0)
        .interact()?;

    if selection == options.len() - 1 {
        prompt_for_new_directory(current_repo, thoughts_repo, repos_dir)
    } else {
        Ok(existing_repos[selection].clone())
    }
}

fn create_mapping(mapped_name: &str, profile: &Option<String>) -> RepoMapping {
    match profile {
        Some(name) => RepoMapping::Object {
            repo: mapped_name.to_string(),
            profile: Some(name.clone()),
        },
        None => RepoMapping::String(mapped_name.to_string()),
    }
}

fn setup_directory_structure(ctx: &InitContext) -> Result<()> {
    let repo_thoughts_path = ctx.expanded_repo.join(&ctx.repos_dir).join(&ctx.mapped_name);
    fs::create_dir_all(repo_thoughts_path.join(&ctx.thoughts_config.user))?;
    fs::create_dir_all(repo_thoughts_path.join("shared"))?;

    let global_path = ctx.expanded_repo.join(&ctx.global_dir);
    fs::create_dir_all(global_path.join(&ctx.thoughts_config.user))?;
    fs::create_dir_all(global_path.join("shared"))?;

    Ok(())
}

fn initialize_git_if_needed(ctx: &InitContext) -> Result<()> {
    if GitRepo::is_repo(&ctx.expanded_repo) {
        return Ok(());
    }

    GitRepo::init(&ctx.expanded_repo)?;

    let gitignore = "# OS files\n.DS_Store\nThumbs.db\n\n# Editor files\n.vscode/\n.idea/\n*.swp\n*.swo\n*~\n\n# Temporary files\n*.tmp\n*.bak\n";
    fs::write(ctx.expanded_repo.join(".gitignore"), gitignore)?;

    let git_repo = GitRepo::open(&ctx.expanded_repo)?;
    git_repo.add_all()?;
    git_repo.commit("Initial thoughts repository setup")?;

    Ok(())
}

fn setup_symlinks(ctx: &InitContext) -> Result<()> {
    let thoughts_dir = ctx.current_repo.join("thoughts");
    let repo_thoughts_path = ctx.expanded_repo.join(&ctx.repos_dir).join(&ctx.mapped_name);
    let global_path = ctx.expanded_repo.join(&ctx.global_dir);

    if thoughts_dir.exists() {
        fs::remove_dir_all(&thoughts_dir)?;
    }
    fs::create_dir(&thoughts_dir)?;

    create_symlinks(&thoughts_dir, &repo_thoughts_path, &global_path, &ctx.thoughts_config.user)
}

#[cfg(unix)]
fn create_symlinks(thoughts_dir: &Path, repo_thoughts_path: &Path, global_path: &Path, user: &str) -> Result<()> {
    std::os::unix::fs::symlink(repo_thoughts_path.join(user), thoughts_dir.join(user))?;
    std::os::unix::fs::symlink(repo_thoughts_path.join("shared"), thoughts_dir.join("shared"))?;
    std::os::unix::fs::symlink(global_path, thoughts_dir.join("global"))?;
    Ok(())
}

#[cfg(windows)]
fn create_symlinks(thoughts_dir: &Path, repo_thoughts_path: &Path, global_path: &Path, user: &str) -> Result<()> {
    use std::os::windows::fs::symlink_dir;

    let create = |target: &Path, link: &Path| -> Result<()> {
        symlink_dir(target, link).with_context(|| {
            format!(
                "Failed to create symlink. On Windows, symlinks require either:\n\
                 1. Run as Administrator, or\n\
                 2. Enable Developer Mode in Settings > Update & Security > For developers\n\n\
                 Target: {}\nLink: {}",
                target.display(),
                link.display()
            )
        })
    };

    create(&repo_thoughts_path.join(user), &thoughts_dir.join(user))?;
    create(&repo_thoughts_path.join("shared"), &thoughts_dir.join("shared"))?;
    create(global_path, &thoughts_dir.join("global"))?;
    Ok(())
}

fn setup_hooks_and_print_summary(ctx: &InitContext) -> Result<()> {
    let hooks_updated = setup_git_hooks(&ctx.current_repo)?;
    if !hooks_updated.is_empty() {
        println!("{}", format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow());
    }

    print_summary(ctx);
    Ok(())
}

fn print_summary(ctx: &InitContext) {
    println!("{}", "✅ Thoughts setup complete!".green());
    println!();
    println!("{}", "=== Summary ===".blue());
    println!();
    println!("Repository structure created:");
    println!("  {}{SEP}", ctx.current_repo.display().to_string().cyan());
    println!("    └── thoughts{SEP}");
    println!(
        "         ├── {}{SEP}     → {}{SEP}{}{SEP}{}{SEP}{}{SEP}",
        ctx.thoughts_config.user.cyan(),
        ctx.thoughts_repo.cyan(),
        ctx.repos_dir.cyan(),
        ctx.mapped_name.cyan(),
        ctx.thoughts_config.user.cyan(),
    );
    println!(
        "         ├── shared{SEP}      → {}{SEP}{}{SEP}{}{SEP}shared{SEP}",
        ctx.thoughts_repo.cyan(),
        ctx.repos_dir.cyan(),
        ctx.mapped_name.cyan(),
    );
    println!(
        "         └── global{SEP}      → {}{SEP}{}{SEP}",
        ctx.thoughts_repo.cyan(),
        ctx.global_dir.cyan(),
    );
    println!();
    println!("Protection enabled:");
    println!("  {} Pre-commit hook: Prevents committing thoughts/", "✓".green());
    println!("  {} Post-commit hook: Auto-syncs thoughts after commits", "✓".green());
}

// === Git Hooks ===

fn hook_needs_update(hook_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(hook_path) else {
        return true;
    };

    if !content.contains("hyprlayer thoughts") {
        return false;
    }

    content
        .lines()
        .find(|l| l.contains("# Version:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v < HOOK_VERSION.parse::<u32>().unwrap_or(1))
        .unwrap_or(true)
}

fn setup_git_hooks(repo_path: &Path) -> Result<Vec<String>> {
    let hooks_dir = get_hooks_dir(repo_path)?;
    fs::create_dir_all(&hooks_dir)?;

    let mut updated = Vec::new();

    if install_hook(&hooks_dir, "pre-commit", pre_commit_content())? {
        updated.push("pre-commit".to_string());
    }
    if install_hook(&hooks_dir, "post-commit", post_commit_content())? {
        updated.push("post-commit".to_string());
    }

    Ok(updated)
}

fn get_hooks_dir(repo_path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output()
        .context("Failed to find git directory")?;

    let git_common_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let git_common_dir = if Path::new(&git_common_dir).is_absolute() {
        PathBuf::from(&git_common_dir)
    } else {
        repo_path.join(&git_common_dir)
    };

    Ok(git_common_dir.join("hooks"))
}

fn install_hook(hooks_dir: &Path, name: &str, content: String) -> Result<bool> {
    let hook_path = hooks_dir.join(name);

    if !hook_needs_update(&hook_path) {
        return Ok(false);
    }

    // Backup existing non-hyprlayer hook
    if hook_path.exists() {
        let existing = fs::read_to_string(&hook_path)?;
        if !existing.contains("hyprlayer thoughts") {
            fs::rename(&hook_path, format!("{}.old", hook_path.display()))?;
        }
    }

    fs::write(&hook_path, content)?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    Ok(true)
}

fn pre_commit_content() -> String {
    format!(
        r#"#!/bin/bash
# hyprlayer thoughts protection - prevent committing thoughts directory
# Version: {HOOK_VERSION}

if git diff --cached --name-only | grep -q "^thoughts/"; then
    echo "❌ Cannot commit thoughts/ to code repository"
    echo "The thoughts directory should only exist in your separate thoughts repository."
    git reset HEAD -- thoughts/
    exit 1
fi

# Call any existing pre-commit hook
SCRIPT_PATH="$(realpath "$0")"
if [ -f "$SCRIPT_PATH.old" ]; then
    "$SCRIPT_PATH.old" "$@"
fi
"#
    )
}

fn post_commit_content() -> String {
    format!(
        r#"#!/bin/bash
# hyprlayer thoughts auto-sync
# Version: {HOOK_VERSION}

# Check if we're in a worktree (skip auto-sync in worktrees)
if [ -f .git ]; then
    exit 0
fi

# Get the commit message
COMMIT_MSG=$(git log -1 --pretty=%B)

# Auto-sync thoughts after each commit (only in non-worktree repos)
hyprlayer thoughts sync --message "Auto-sync with commit: $COMMIT_MSG" >/dev/null 2>&1 &

# Call any existing post-commit hook
SCRIPT_PATH="$(realpath "$0")"
if [ -f "$SCRIPT_PATH.old" ]; then
    "$SCRIPT_PATH.old" "$@"
fi
"#
    )
}
