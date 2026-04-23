use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use std::fs;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};

use crate::backends::{self, BackendContext};
use crate::cli::InitArgs;
use crate::config::{
    BackendKind, BackendSettings, HyprlayerConfig, ProfileConfig, RepoMapping, ThoughtsConfig,
    expand_path, get_current_repo_path, get_default_thoughts_repo, get_repo_name_from_path,
    sanitize_directory_name,
};
use crate::git_ops::GitRepo;

pub fn init(args: InitArgs) -> Result<()> {
    let InitArgs {
        force,
        directory,
        profile,
        backend,
        vault_path,
        vault_subpath,
        parent_page_id,
        database_id,
        api_token_env,
        yes,
        config,
    } = args;

    let current_repo = get_current_repo_path()?;

    if !GitRepo::is_repo(&current_repo) {
        return Err(anyhow::anyhow!("Not in a git repository"));
    }

    let notion_flags = NotionFlags {
        parent_page_id,
        database_id,
        api_token_env: api_token_env.clone(),
    };

    if yes {
        return init_non_interactive(
            config,
            current_repo,
            directory,
            profile,
            backend,
            vault_path,
            vault_subpath,
            notion_flags,
            force,
        );
    }

    let config_path = config.path()?;
    let mut hyprlayer_config = config.load_if_exists()?.unwrap_or_default();

    if hyprlayer_config
        .ai
        .as_ref()
        .is_none_or(|ai| ai.agent_tool.is_none())
    {
        return Err(anyhow::anyhow!(
            "AI tool not configured. Run 'hyprlayer ai configure' first."
        ));
    }

    let orphaned = hyprlayer_config.thoughts_mut().find_orphaned_mappings();
    if !orphaned.is_empty() {
        println!(
            "{}",
            "Found stale repo mappings (paths no longer exist):".yellow()
        );
        for path in &orphaned {
            println!("  {}", path.bright_black());
        }
        if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Remove stale mappings from config?")
            .default(true)
            .interact()?
        {
            hyprlayer_config.thoughts_mut().remove_mappings(&orphaned);
            hyprlayer_config.save(&config_path)?;
            println!("{}", "Stale mappings removed.".green());
        }
    }

    hyprlayer_config.thoughts_mut().validate_profile(&profile)?;

    if !check_existing_setup(&current_repo, force)? {
        return Ok(());
    }

    let existing_profile = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let backend_kind = resolve_backend_interactive(backend, existing_profile.backend)?;

    let refreshed = prompt_for_thoughts_fields(
        hyprlayer_config.thoughts.clone().unwrap_or_default(),
        &existing_profile,
        backend_kind,
        vault_path,
        vault_subpath,
        &notion_flags,
        &profile,
    )?;
    hyprlayer_config.thoughts = Some(refreshed);

    let resolved = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let mapped_name = if backend_uses_filesystem(backend_kind) {
        let content_root = resolve_content_root(backend_kind, &resolved)?;
        ensure_content_root(&content_root)?;

        let repos_path = content_root.join(&resolved.repos_dir);
        fs::create_dir_all(&repos_path)?;

        select_or_create_directory(
            &repos_path,
            &current_repo,
            directory,
            &content_root,
            &resolved.repos_dir,
        )?
    } else {
        let default_name = get_repo_name_from_path(&current_repo);
        let chosen = match directory {
            Some(d) => d,
            None => Input::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Project identifier (used as the `project` metadata field) [{}]",
                    default_name
                ))
                .default(default_name)
                .interact()?,
        };
        sanitize_directory_name(&chosen)
    };

    let mapping = RepoMapping::new(&mapped_name, &profile);
    hyprlayer_config
        .thoughts_mut()
        .repo_mappings
        .insert(current_repo.display().to_string(), mapping);
    hyprlayer_config.save(&config_path)?;
    println!("{}", "✅ Global thoughts configuration saved".green());

    dispatch_backend_init(
        &hyprlayer_config,
        &current_repo,
        backend_kind,
        &resolved,
        &mapped_name,
    )?;

    Ok(())
}

#[derive(Debug, Default, Clone)]
struct NotionFlags {
    parent_page_id: Option<String>,
    database_id: Option<String>,
    api_token_env: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn init_non_interactive(
    config: crate::cli::ConfigArgs,
    current_repo: PathBuf,
    directory: Option<String>,
    profile: Option<String>,
    backend_flag: Option<BackendKind>,
    vault_path_flag: Option<String>,
    vault_subpath_flag: Option<String>,
    notion_flags: NotionFlags,
    force: bool,
) -> Result<()> {
    let directory =
        directory.ok_or_else(|| anyhow::anyhow!("--directory is required when using --yes"))?;

    let config_path = config.path()?;
    let mut hyprlayer_config = config.load_if_exists()?.ok_or_else(|| {
        anyhow::anyhow!(
            "No existing config found. Run 'hyprlayer thoughts init' interactively first."
        )
    })?;

    {
        let thoughts = hyprlayer_config.thoughts.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Config is incomplete. Run 'hyprlayer thoughts init' interactively to complete setup."
            )
        })?;

        if !thoughts.is_thoughts_configured() {
            return Err(anyhow::anyhow!(
                "Config is incomplete. Run 'hyprlayer thoughts init' interactively to complete setup."
            ));
        }

        if hyprlayer_config
            .ai
            .as_ref()
            .is_none_or(|ai| ai.agent_tool.is_none())
        {
            return Err(anyhow::anyhow!(
                "AI tool not configured. Run 'hyprlayer ai configure' first."
            ));
        }

        thoughts.validate_profile(&profile)?;
    }

    let thoughts_dir = current_repo.join("thoughts");
    if thoughts_dir.exists() && !force {
        println!(
            "{}",
            "Thoughts already configured for this repository, skipping.".bright_black()
        );
        return Ok(());
    }

    let existing_profile = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let backend_kind = backend_flag.unwrap_or(existing_profile.backend);

    if backend_kind == BackendKind::Obsidian {
        let vault_from_flag =
            vault_path_flag.or_else(|| existing_profile.backend_settings.vault_path.clone());
        if vault_from_flag.is_none() {
            return Err(anyhow::anyhow!(
                "--vault-path is required when --backend obsidian is used with --yes"
            ));
        }
        let subpath = vault_subpath_flag
            .or_else(|| existing_profile.backend_settings.vault_subpath.clone())
            .unwrap_or_else(|| "hyprlayer".to_string());

        let new_settings = BackendSettings {
            vault_path: vault_from_flag,
            vault_subpath: Some(subpath),
            ..existing_profile.backend_settings.clone()
        };
        apply_backend_and_settings(
            hyprlayer_config.thoughts_mut(),
            &profile,
            backend_kind,
            new_settings,
        );
    } else if backend_kind == BackendKind::Notion {
        let parent = notion_flags
            .parent_page_id
            .clone()
            .or_else(|| existing_profile.backend_settings.parent_page_id.clone());
        if parent.as_deref().unwrap_or("").is_empty() {
            return Err(anyhow::anyhow!(
                "--parent-page-id is required when --backend notion is used with --yes"
            ));
        }
        let token_env = notion_flags
            .api_token_env
            .clone()
            .or_else(|| existing_profile.backend_settings.api_token_env.clone())
            .unwrap_or_else(|| "NOTION_TOKEN".to_string());
        let db_id = notion_flags
            .database_id
            .clone()
            .or_else(|| existing_profile.backend_settings.database_id.clone());

        let new_settings = BackendSettings {
            parent_page_id: parent,
            database_id: db_id,
            api_token_env: Some(token_env),
            ..existing_profile.backend_settings.clone()
        };
        apply_backend_and_settings(
            hyprlayer_config.thoughts_mut(),
            &profile,
            backend_kind,
            new_settings,
        );
    } else if backend_flag.is_some() {
        apply_backend_and_settings(
            hyprlayer_config.thoughts_mut(),
            &profile,
            backend_kind,
            existing_profile.backend_settings.clone(),
        );
    }

    let resolved = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let mapped_name = sanitize_directory_name(&directory);

    if backend_uses_filesystem(backend_kind) {
        let content_root = resolve_content_root(backend_kind, &resolved)?;
        ensure_content_root(&content_root)?;

        let repos_path = content_root.join(&resolved.repos_dir);
        fs::create_dir_all(&repos_path)?;

        let target_dir = repos_path.join(&mapped_name);
        if !target_dir.exists() {
            fs::create_dir_all(&target_dir)?;
            println!(
                "{}",
                format!(
                    "Created thoughts directory: {}{SEP}{}{SEP}{}",
                    content_root.display().to_string().cyan(),
                    resolved.repos_dir.cyan(),
                    mapped_name.cyan()
                )
                .green()
            );
        }
    }

    let mapping = RepoMapping::new(&mapped_name, &profile);
    hyprlayer_config
        .thoughts_mut()
        .repo_mappings
        .insert(current_repo.display().to_string(), mapping);
    hyprlayer_config.save(&config_path)?;
    println!("{}", "✅ Global thoughts configuration saved".green());

    dispatch_backend_init(
        &hyprlayer_config,
        &current_repo,
        backend_kind,
        &resolved,
        &mapped_name,
    )?;

    Ok(())
}

fn resolve_backend_interactive(
    from_flag: Option<BackendKind>,
    current: BackendKind,
) -> Result<BackendKind> {
    if let Some(kind) = from_flag {
        return Ok(kind);
    }

    if current != BackendKind::Git {
        // Respect the existing non-git backend without re-prompting.
        return Ok(current);
    }

    // First-run: offer a short menu. Keep git as the default so existing
    // users see no change when they press enter.
    let items = ["git (default)", "obsidian", "notion"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Storage backend for thoughts")
        .items(&items)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => BackendKind::Git,
        1 => BackendKind::Obsidian,
        2 => BackendKind::Notion,
        _ => unreachable!(),
    })
}

/// Prompt the user for thoughts directory configuration, preserving any
/// already-populated fields. Branches on backend kind so Obsidian users
/// are asked for a vault path instead of a thoughts repo.
fn prompt_for_thoughts_fields(
    existing: ThoughtsConfig,
    existing_profile: &ProfileConfig,
    backend_kind: BackendKind,
    vault_path_flag: Option<String>,
    vault_subpath_flag: Option<String>,
    notion_flags: &NotionFlags,
    profile: &Option<String>,
) -> Result<ThoughtsConfig> {
    let theme = ColorfulTheme::default();
    println!("{}", "=== Initial Thoughts Setup ===".blue());
    println!();

    let (thoughts_repo, backend_settings) = match backend_kind {
        BackendKind::Git => {
            let fallback = get_default_thoughts_repo()?.display().to_string();
            let default_repo = if existing_profile.thoughts_repo.is_empty() {
                fallback
            } else {
                existing_profile.thoughts_repo.clone()
            };
            let repo: String = Input::with_theme(&theme)
                .with_prompt("Thoughts repository location")
                .default(default_repo.clone())
                .allow_empty(true)
                .interact()
                .map(|s: String| if s.is_empty() { default_repo } else { s })?;
            (repo, existing_profile.backend_settings.clone())
        }
        BackendKind::Obsidian => {
            let existing_vault = existing_profile
                .backend_settings
                .vault_path
                .clone()
                .unwrap_or_default();
            let vault_path = match vault_path_flag {
                Some(v) => v,
                None => prompt_vault_path(&theme, &existing_vault)?,
            };
            let default_sub = existing_profile
                .backend_settings
                .vault_subpath
                .clone()
                .unwrap_or_else(|| "hyprlayer".to_string());
            let vault_subpath = match vault_subpath_flag {
                Some(v) => v,
                None => Input::with_theme(&theme)
                    .with_prompt("Subfolder within vault (leave blank for vault root)")
                    .default(default_sub)
                    .allow_empty(true)
                    .interact()?,
            };
            let settings = BackendSettings {
                vault_path: Some(vault_path.clone()),
                vault_subpath: Some(vault_subpath),
                ..existing_profile.backend_settings.clone()
            };
            // Preserve any previous git thoughts_repo so switching git↔obsidian doesn't lose it.
            (existing.thoughts_repo.clone(), settings)
        }
        BackendKind::Notion => {
            let settings =
                prompt_notion_settings(&theme, &existing_profile.backend_settings, notion_flags)?;
            // Preserve any existing filesystem repo path so switching backends doesn't lose it.
            (existing.thoughts_repo.clone(), settings)
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Backend '{}' is not yet supported in this version",
                backend_kind.as_str()
            ));
        }
    };

    let (repos_dir, global_dir) = if backend_uses_filesystem(backend_kind) {
        println!();
        let default_repos_dir = if existing_profile.repos_dir.is_empty() {
            "repos".to_string()
        } else {
            existing_profile.repos_dir.clone()
        };
        let repos_dir: String = Input::with_theme(&theme)
            .with_prompt("Directory name for repository-specific thoughts")
            .default(default_repos_dir)
            .interact()?;

        let default_global_dir = if existing_profile.global_dir.is_empty() {
            "global".to_string()
        } else {
            existing_profile.global_dir.clone()
        };
        let global_dir: String = Input::with_theme(&theme)
            .with_prompt("Directory name for global thoughts")
            .default(default_global_dir)
            .interact()?;
        (repos_dir, global_dir)
    } else {
        let repos_dir = if existing_profile.repos_dir.is_empty() {
            "repos".to_string()
        } else {
            existing_profile.repos_dir.clone()
        };
        let global_dir = if existing_profile.global_dir.is_empty() {
            "global".to_string()
        } else {
            existing_profile.global_dir.clone()
        };
        (repos_dir, global_dir)
    };

    let user = prompt_for_username(&theme, &existing.user)?;

    if backend_uses_filesystem(backend_kind) {
        println!();
        println!("{}", "Creating thoughts structure:".yellow());
        let preview = ProfileConfig {
            thoughts_repo: thoughts_repo.clone(),
            repos_dir: repos_dir.clone(),
            global_dir: global_dir.clone(),
            backend: backend_kind,
            backend_settings: backend_settings.clone(),
        };
        println!("  {}{SEP}", display_root(backend_kind, &preview).cyan());
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
    }

    let mut out = ThoughtsConfig {
        user,
        repo_mappings: existing.repo_mappings,
        profiles: existing.profiles,
        ..existing
    };
    let new_profile = ProfileConfig {
        thoughts_repo,
        repos_dir,
        global_dir,
        backend: backend_kind,
        backend_settings,
    };
    match profile.as_ref() {
        Some(name) => {
            out.profiles.insert(name.clone(), new_profile);
        }
        None => {
            out.thoughts_repo = new_profile.thoughts_repo;
            out.repos_dir = new_profile.repos_dir;
            out.global_dir = new_profile.global_dir;
            out.backend = new_profile.backend;
            out.backend_settings = new_profile.backend_settings;
        }
    }

    Ok(out)
}

fn backend_uses_filesystem(kind: BackendKind) -> bool {
    matches!(kind, BackendKind::Git | BackendKind::Obsidian)
}

fn prompt_notion_settings(
    theme: &ColorfulTheme,
    existing: &BackendSettings,
    flags: &NotionFlags,
) -> Result<BackendSettings> {
    let default_parent = existing.parent_page_id.clone().unwrap_or_default();
    let parent_page_id = match flags.parent_page_id.clone() {
        Some(v) => v,
        None => {
            let mut input = Input::<String>::with_theme(theme)
                .with_prompt("Notion parent page ID (from the page URL after the last `-`)");
            if !default_parent.is_empty() {
                input = input.default(default_parent);
            }
            input.interact()?
        }
    };
    if parent_page_id.trim().is_empty() {
        return Err(anyhow::anyhow!("Notion parent page ID is required"));
    }

    let default_env = existing
        .api_token_env
        .clone()
        .unwrap_or_else(|| "NOTION_TOKEN".to_string());
    let api_token_env = match flags.api_token_env.clone() {
        Some(v) => v,
        None => Input::<String>::with_theme(theme)
            .with_prompt("Env var holding your Notion integration token")
            .default(default_env)
            .interact()?,
    };

    let default_db = existing.database_id.clone().unwrap_or_default();
    let db_input = match flags.database_id.clone() {
        Some(v) => v,
        None => Input::<String>::with_theme(theme)
            .with_prompt("Existing Notion database ID (leave blank to create on first use)")
            .default(default_db)
            .allow_empty(true)
            .interact()?,
    };
    let database_id = if db_input.trim().is_empty() {
        None
    } else {
        Some(db_input)
    };

    Ok(BackendSettings {
        parent_page_id: Some(parent_page_id),
        database_id,
        api_token_env: Some(api_token_env),
        ..existing.clone()
    })
}

fn prompt_vault_path(theme: &ColorfulTheme, existing: &str) -> Result<String> {
    loop {
        let mut input = Input::<String>::with_theme(theme)
            .with_prompt("Obsidian vault path (e.g. ~/Documents/MyVault)");
        if !existing.is_empty() {
            input = input.default(existing.to_string());
        }
        let raw: String = input.interact()?;
        let expanded = expand_path(&raw);
        if !expanded.exists() {
            println!(
                "{}",
                format!("Path does not exist: {}", expanded.display()).red()
            );
            continue;
        }
        if !expanded.is_dir() {
            println!("{}", "Vault path must be a directory".red());
            continue;
        }
        return Ok(raw);
    }
}

fn prompt_for_username(theme: &ColorfulTheme, existing_user: &str) -> Result<String> {
    let default_user = if existing_user.is_empty() {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string())
    } else {
        existing_user.to_string()
    };

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
    let reconfigure = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Do you want to reconfigure?")
        .default(false)
        .interact()?;

    if !reconfigure {
        println!("Setup cancelled.");
    }
    Ok(reconfigure)
}

fn resolve_content_root(backend_kind: BackendKind, resolved: &ProfileConfig) -> Result<PathBuf> {
    match backend_kind {
        BackendKind::Git => Ok(expand_path(&resolved.thoughts_repo)),
        BackendKind::Obsidian => {
            // Check vault existence here, before `ensure_content_root` would
            // create the missing path. Obsidian vaults are user-managed — we
            // never auto-create them.
            let vault = expand_path(resolved.backend_settings.vault_path.as_deref().ok_or_else(
                || anyhow::anyhow!("Obsidian backend requires vaultPath in settings"),
            )?);
            if !vault.exists() {
                return Err(anyhow::anyhow!(
                    "Obsidian vault does not exist: {}. Create it in Obsidian first.",
                    vault.display()
                ));
            }
            resolved
                .backend_settings
                .obsidian_root()
                .ok_or_else(|| anyhow::anyhow!("Obsidian backend requires vaultPath in settings"))
        }
        other => Err(anyhow::anyhow!(
            "Backend '{}' is not yet supported in this version",
            other.as_str()
        )),
    }
}

fn display_root(backend_kind: BackendKind, resolved: &ProfileConfig) -> String {
    match backend_kind {
        BackendKind::Git => resolved.thoughts_repo.clone(),
        BackendKind::Obsidian => resolved
            .backend_settings
            .obsidian_root()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn ensure_content_root(content_root: &Path) -> Result<()> {
    if !content_root.exists() {
        fs::create_dir_all(content_root)?;
        println!(
            "{}",
            format!(
                "Created thoughts content root at {}",
                content_root.display()
            )
            .green()
        );
    }
    Ok(())
}

fn apply_backend_and_settings(
    thoughts: &mut ThoughtsConfig,
    profile: &Option<String>,
    backend_kind: BackendKind,
    settings: BackendSettings,
) {
    match profile.as_ref() {
        Some(name) => {
            if let Some(p) = thoughts.profiles.get_mut(name) {
                p.backend = backend_kind;
                p.backend_settings = settings;
            }
        }
        None => {
            thoughts.backend = backend_kind;
            thoughts.backend_settings = settings;
        }
    }
}

fn select_or_create_directory(
    repos_path: &Path,
    current_repo: &Path,
    directory: Option<String>,
    content_root: &Path,
    repos_dir: &str,
) -> Result<String> {
    if let Some(dir) = directory {
        return use_existing_directory(repos_path, &dir, content_root, repos_dir);
    }

    let existing_repos = list_existing_repos(repos_path)?;

    if existing_repos.is_empty() {
        prompt_for_new_directory(current_repo, content_root, repos_dir)
    } else {
        select_or_create_from_existing(&existing_repos, current_repo, content_root, repos_dir)
    }
}

fn use_existing_directory(
    repos_path: &Path,
    dir: &str,
    content_root: &Path,
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
            content_root.display().to_string().cyan(),
            repos_dir.cyan(),
            sanitized.cyan()
        )
        .green()
    );
    Ok(sanitized)
}

fn list_existing_repos(repos_path: &Path) -> Result<Vec<String>> {
    if !repos_path.exists() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(repos_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect())
}

fn prompt_for_new_directory(
    current_repo: &Path,
    content_root: &Path,
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
            content_root.display().to_string().cyan(),
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
    content_root: &Path,
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
        prompt_for_new_directory(current_repo, content_root, repos_dir)
    } else {
        Ok(existing_repos[selection].clone())
    }
}

fn dispatch_backend_init(
    config: &HyprlayerConfig,
    current_repo: &Path,
    backend_kind: BackendKind,
    resolved: &ProfileConfig,
    mapped_name: &str,
) -> Result<()> {
    let current_repo_str = current_repo.display().to_string();
    let effective = config
        .thoughts
        .as_ref()
        .expect("thoughts config must exist here")
        .effective_config_for(&current_repo_str);

    let ctx = BackendContext::new(current_repo, &effective);
    let backend_impl = backends::for_kind(backend_kind);
    backend_impl.init(&ctx)?;

    print_summary(
        backend_kind,
        resolved,
        mapped_name,
        current_repo,
        &effective,
    );
    Ok(())
}

fn print_summary(
    backend_kind: BackendKind,
    resolved: &ProfileConfig,
    mapped_name: &str,
    current_repo: &Path,
    effective: &crate::config::EffectiveConfig,
) {
    println!("{}", "✅ Thoughts setup complete!".green());
    println!();
    println!("{}", "=== Summary ===".blue());
    println!();
    println!("Backend: {}", backend_kind.as_str().cyan());
    println!();

    if backend_uses_filesystem(backend_kind) {
        println!("Repository structure created:");
        println!("  {}{SEP}", current_repo.display().to_string().cyan());
        println!("    └── thoughts{SEP}");

        let root_display = display_root(backend_kind, resolved);

        println!(
            "         ├── {}{SEP}     → {}{SEP}{}{SEP}{}{SEP}{}{SEP}",
            effective.user.cyan(),
            root_display.cyan(),
            resolved.repos_dir.cyan(),
            mapped_name.cyan(),
            effective.user.cyan(),
        );
        println!(
            "         ├── shared{SEP}      → {}{SEP}{}{SEP}{}{SEP}shared{SEP}",
            root_display.cyan(),
            resolved.repos_dir.cyan(),
            mapped_name.cyan(),
        );
        println!(
            "         └── global{SEP}      → {}{SEP}{}{SEP}",
            root_display.cyan(),
            resolved.global_dir.cyan(),
        );
        println!();
    } else {
        println!("Project identifier: {}", mapped_name.cyan());
        println!(
            "{}",
            "  (used as the `project` metadata field on every thought)".bright_black()
        );
        println!();
    }

    match backend_kind {
        BackendKind::Git => {
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
        BackendKind::Obsidian => {
            println!("Protection enabled:");
            println!(
                "  {} Pre-commit hook: Prevents committing thoughts/",
                "✓".green()
            );
            println!(
                "{}",
                "  (no post-commit auto-sync — Obsidian vaults sync themselves)".bright_black()
            );
        }
        BackendKind::Notion => {
            println!(
                "{}",
                "The Notion MCP server has been registered with your AI tool.".bright_black()
            );
            println!(
                "{}",
                "Your first /create_plan (or similar) will create the database under the \
                 configured parent page and persist its ID."
                    .bright_black()
            );
        }
        _ => {}
    }
}
