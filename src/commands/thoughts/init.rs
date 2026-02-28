#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use colored::Colorize;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::fs;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};

use crate::cli::InitArgs;
use crate::config::{
    ProfileConfig, RepoMapping, ThoughtsConfig, expand_path, get_current_repo_path,
    get_default_thoughts_repo, get_repo_name_from_path, sanitize_directory_name,
};
use crate::git_ops::GitRepo;
use crate::hooks;

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
    let InitArgs {
        force,
        directory,
        profile,
        config,
    } = args;
    let current_repo = get_current_repo_path()?;

    if !GitRepo::is_repo(&current_repo) {
        return Err(anyhow::anyhow!("Not in a git repository"));
    }

    let config_path = config.path()?;
    let mut thoughts_config = load_or_create_config(&config)?;

    // Require AI to be configured first
    if thoughts_config.agent_tool.is_none() {
        return Err(anyhow::anyhow!(
            "AI tool not configured. Run 'hyprlayer ai configure' first."
        ));
    }

    // Check for stale repo mappings (paths that no longer exist on disk)
    let orphaned = thoughts_config.find_orphaned_mappings();
    if !orphaned.is_empty() {
        println!(
            "{}",
            "Found stale repo mappings (paths no longer exist):".yellow()
        );
        for path in &orphaned {
            println!("  {}", path.bright_black());
        }
        let remove: bool = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Remove stale mappings from config?")
            .default(true)
            .interact()?;
        if remove {
            thoughts_config.remove_mappings(&orphaned);
            thoughts_config.save(&config_path)?;
            println!("{}", "Stale mappings removed.".green());
        }
    }

    // Install agent files if not already present, or if --force is used.
    if let Some(ref agent_tool) = thoughts_config.agent_tool {
        if force || !agent_tool.is_installed() {
            let provider = thoughts_config.opencode_provider.as_ref();
            agent_tool.install(provider)?;
            println!(
                "{}",
                format!(
                    "  {} agent files installed to {}",
                    agent_tool,
                    agent_tool.dest_display()
                )
                .green()
            );
        } else {
            println!(
                "{}",
                format!(
                    "  {} agent files already installed, skipping download (use --force to reinstall)",
                    agent_tool,
                )
                .bright_black()
            );
        }
    }

    thoughts_config.validate_profile(&profile)?;

    if !check_existing_setup(&current_repo, force)? {
        return Ok(());
    }

    let ProfileConfig {
        thoughts_repo,
        repos_dir,
        global_dir,
    } = thoughts_config.resolve_dirs(&profile);
    let expanded_repo = expand_path(&thoughts_repo);

    ensure_repo_exists(&expanded_repo, &thoughts_repo)?;

    let repos_path = expanded_repo.join(&repos_dir);
    fs::create_dir_all(&repos_path)?;

    let mapped_name = select_or_create_directory(
        &repos_path,
        &current_repo,
        directory,
        &thoughts_repo,
        &repos_dir,
    )?;

    // Update config with mapping
    let mapping = RepoMapping::new(&mapped_name, &profile);
    thoughts_config
        .repo_mappings
        .insert(current_repo.display().to_string(), mapping);
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

    Ok(ThoughtsConfig {
        thoughts_repo,
        repos_dir,
        global_dir,
        user,
        agent_tool: None, // Will be set by `ai configure`
        opencode_provider: None,
        opencode_sonnet_model: None,
        opencode_opus_model: None,
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
        println!(
            "{}",
            "Username cannot be \"global\" as it's reserved for cross-project thoughts.".red()
        );
    }
}

fn check_existing_setup(current_repo: &Path, force: bool) -> Result<bool> {
    let thoughts_dir = current_repo.join("thoughts");
    if !thoughts_dir.exists() || force {
        return Ok(true);
    }

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
    }
    Ok(reconfigure)
}

fn ensure_repo_exists(expanded_repo: &Path, thoughts_repo: &str) -> Result<()> {
    if !expanded_repo.exists() {
        fs::create_dir_all(expanded_repo)?;
        println!(
            "{}",
            format!("Created thoughts repository at {}", thoughts_repo.cyan()).green()
        );
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

fn use_existing_directory(
    repos_path: &Path,
    dir: &str,
    thoughts_repo: &str,
    repos_dir: &str,
) -> Result<String> {
    let sanitized = sanitize_directory_name(dir);
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

fn prompt_for_new_directory(
    current_repo: &Path,
    thoughts_repo: &str,
    repos_dir: &str,
) -> Result<String> {
    let default_name = get_repo_name_from_path(current_repo);
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
    Ok(sanitized)
}

fn select_or_create_from_existing(
    existing_repos: &[String],
    current_repo: &Path,
    thoughts_repo: &str,
    repos_dir: &str,
) -> Result<String> {
    let mut options: Vec<String> = existing_repos
        .iter()
        .map(|r| format!("Use existing: {}", r))
        .collect();
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

fn setup_directory_structure(ctx: &InitContext) -> Result<()> {
    let repo_thoughts_path = ctx
        .expanded_repo
        .join(&ctx.repos_dir)
        .join(&ctx.mapped_name);
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
    let repo_thoughts_path = ctx
        .expanded_repo
        .join(&ctx.repos_dir)
        .join(&ctx.mapped_name);
    let global_path = ctx.expanded_repo.join(&ctx.global_dir);

    if thoughts_dir.exists() {
        fs::remove_dir_all(&thoughts_dir)?;
    }
    fs::create_dir(&thoughts_dir)?;

    create_symlinks(
        &thoughts_dir,
        &repo_thoughts_path,
        &global_path,
        &ctx.thoughts_config.user,
    )
}

#[cfg(unix)]
fn create_symlinks(
    thoughts_dir: &Path,
    repo_thoughts_path: &Path,
    global_path: &Path,
    user: &str,
) -> Result<()> {
    std::os::unix::fs::symlink(repo_thoughts_path.join(user), thoughts_dir.join(user))?;
    std::os::unix::fs::symlink(
        repo_thoughts_path.join("shared"),
        thoughts_dir.join("shared"),
    )?;
    std::os::unix::fs::symlink(global_path, thoughts_dir.join("global"))?;
    Ok(())
}

#[cfg(windows)]
fn create_symlinks(
    thoughts_dir: &Path,
    repo_thoughts_path: &Path,
    global_path: &Path,
    user: &str,
) -> Result<()> {
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
    create(
        &repo_thoughts_path.join("shared"),
        &thoughts_dir.join("shared"),
    )?;
    create(global_path, &thoughts_dir.join("global"))?;
    Ok(())
}

fn setup_hooks_and_print_summary(ctx: &InitContext) -> Result<()> {
    let hooks_updated = hooks::setup_git_hooks(&ctx.current_repo)?;
    if !hooks_updated.is_empty() {
        println!(
            "{}",
            format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow()
        );
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
    println!(
        "  {} Pre-commit hook: Prevents committing thoughts/",
        "✓".green()
    );
    println!(
        "  {} Post-commit hook: Auto-syncs thoughts after commits",
        "✓".green()
    );
}
