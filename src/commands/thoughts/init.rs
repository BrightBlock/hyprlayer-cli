use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use std::fs;
use std::path::{Path, PathBuf};

use crate::backends::{self, BackendContext};
use crate::cli::InitArgs;
use crate::config::{
    AnytypeConfig, BackendConfig, BackendKind, GitConfig, HyprlayerConfig, NotionConfig,
    ObsidianConfig, ProfileConfig, RepoMapping, ThoughtsConfig, expand_path, get_current_repo_path,
    get_default_thoughts_repo, get_repo_name_from_path, sanitize_directory_name,
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
        space_id,
        type_id,
        api_token_env,
        yes,
        config,
    } = args;

    let current_repo = get_current_repo_path()?;

    if backend == Some(BackendKind::Notion) && api_token_env.is_some() {
        return Err(anyhow::anyhow!(
            "--api-token-env is not valid with --backend notion (uses the agent tool's \
             connector — no token to manage)."
        ));
    }

    let notion_flags = NotionFlags {
        parent_page_id,
        database_id,
    };
    let anytype_flags = AnytypeFlags {
        space_id,
        type_id,
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
            anytype_flags,
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
        }
    }

    hyprlayer_config.thoughts_mut().validate_profile(&profile)?;

    if !check_existing_setup(&current_repo, force)? {
        return Ok(());
    }

    let existing_profile = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let backend_kind = resolve_backend_interactive(backend, existing_profile.backend.kind())?;

    require_git_repo_for_filesystem_backend(&current_repo, backend_kind)?;

    let agent_tool = hyprlayer_config.ai.as_ref().and_then(|a| a.agent_tool);
    let refreshed = prompt_for_thoughts_fields(
        hyprlayer_config.thoughts.clone().unwrap_or_default(),
        &existing_profile,
        backend_kind,
        vault_path,
        vault_subpath,
        &notion_flags,
        &anytype_flags,
        &profile,
        agent_tool,
    )?;
    hyprlayer_config.thoughts = Some(refreshed);

    let resolved = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let mapped_name = if backend_kind.uses_filesystem() {
        let content_root = resolve_content_root(&resolved.backend)?;
        ensure_content_root(&content_root)?;

        let repos_dir = resolved.backend.filesystem_repos_dir().unwrap_or("repos");
        let repos_path = content_root.join(repos_dir);
        fs::create_dir_all(&repos_path)?;

        select_or_create_directory(&repos_path, &current_repo, directory)?
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

    dispatch_backend_init(&hyprlayer_config, &current_repo, backend_kind)?;

    Ok(())
}

#[derive(Debug, Default, Clone)]
struct NotionFlags {
    parent_page_id: Option<String>,
    database_id: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct AnytypeFlags {
    space_id: Option<String>,
    type_id: Option<String>,
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
    anytype_flags: AnytypeFlags,
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
    let prior_kind = existing_profile.backend.kind();
    let backend_kind = backend_flag.unwrap_or(prior_kind);

    require_git_repo_for_filesystem_backend(&current_repo, backend_kind)?;

    // When the kind is unchanged, preserve existing variant fields so flags
    // can be applied as overrides. When the kind switches, build a fresh
    // variant — we never carry fields across backends.
    let new_backend = if backend_kind == prior_kind {
        match backend_kind {
            BackendKind::Git => {
                let prior = existing_profile.backend.as_git();
                BackendConfig::Git(GitConfig {
                    thoughts_repo: prior
                        .map(|g| g.thoughts_repo.clone())
                        .unwrap_or_default(),
                    repos_dir: prior
                        .map(|g| g.repos_dir.clone())
                        .unwrap_or_else(|| "repos".to_string()),
                    global_dir: prior
                        .map(|g| g.global_dir.clone())
                        .unwrap_or_else(|| "global".to_string()),
                })
            }
            BackendKind::Obsidian => obsidian_variant_non_interactive(
                vault_path_flag,
                vault_subpath_flag,
                existing_profile.backend.as_obsidian(),
            )?,
            BackendKind::Notion => {
                notion_variant_non_interactive(notion_flags, existing_profile.backend.as_notion())?
            }
            BackendKind::Anytype => anytype_variant_non_interactive(
                anytype_flags,
                existing_profile.backend.as_anytype(),
            )?,
        }
    } else {
        match backend_kind {
            BackendKind::Git => BackendConfig::Git(GitConfig {
                thoughts_repo: get_default_thoughts_repo()?.display().to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            BackendKind::Obsidian => {
                obsidian_variant_non_interactive(vault_path_flag, vault_subpath_flag, None)?
            }
            BackendKind::Notion => notion_variant_non_interactive(notion_flags, None)?,
            BackendKind::Anytype => anytype_variant_non_interactive(anytype_flags, None)?,
        }
    };

    // A bare `--yes` with no `--backend` defaulting to Git has nothing to
    // write; every other branch either set fields or explicitly re-selected
    // Git, and needs to persist.
    if backend_kind != BackendKind::Git || backend_flag.is_some() {
        apply_backend(hyprlayer_config.thoughts_mut(), &profile, new_backend);
    }

    let resolved = hyprlayer_config.thoughts_mut().resolve_dirs(&profile);
    let mapped_name = sanitize_directory_name(&directory);

    if backend_kind.uses_filesystem() {
        let content_root = resolve_content_root(&resolved.backend)?;
        ensure_content_root(&content_root)?;

        let repos_dir = resolved.backend.filesystem_repos_dir().unwrap_or("repos");
        let repos_path = content_root.join(repos_dir);
        fs::create_dir_all(&repos_path)?;

        let target_dir = repos_path.join(&mapped_name);
        if !target_dir.exists() {
            fs::create_dir_all(&target_dir)?;
        }
    }

    let mapping = RepoMapping::new(&mapped_name, &profile);
    hyprlayer_config
        .thoughts_mut()
        .repo_mappings
        .insert(current_repo.display().to_string(), mapping);
    hyprlayer_config.save(&config_path)?;

    dispatch_backend_init(&hyprlayer_config, &current_repo, backend_kind)?;

    Ok(())
}

/// Filesystem backends (git, obsidian) install commit hooks into the working
/// repo, so they need a real git tree. Notion and Anytype store everything
/// externally and have no such requirement.
fn require_git_repo_for_filesystem_backend(
    current_repo: &Path,
    backend_kind: BackendKind,
) -> Result<()> {
    if backend_kind.uses_filesystem() && !GitRepo::is_repo(current_repo) {
        return Err(anyhow::anyhow!("Not in a git repository"));
    }
    Ok(())
}

fn resolve_backend_interactive(
    from_flag: Option<BackendKind>,
    current: BackendKind,
) -> Result<BackendKind> {
    if let Some(kind) = from_flag {
        return Ok(kind);
    }

    let kinds = [
        BackendKind::Git,
        BackendKind::Obsidian,
        BackendKind::Notion,
        BackendKind::Anytype,
    ];
    let default_idx = kinds.iter().position(|k| *k == current).unwrap_or(0);
    let items: Vec<String> = kinds
        .iter()
        .map(|k| {
            if *k == current {
                format!("{} (current)", k.as_str())
            } else {
                k.as_str().to_string()
            }
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Storage backend for thoughts")
        .items(&items)
        .default(default_idx)
        .interact()?;

    Ok(kinds[selection])
}

/// Prompt the user for thoughts directory configuration, building a fresh
/// `BackendConfig` variant for the chosen backend. When the variant is the
/// same as the existing one, prior values seed the prompts; when switching,
/// previous fields are dropped (a tagged enum has no slot for them).
#[allow(clippy::too_many_arguments)]
fn prompt_for_thoughts_fields(
    existing: ThoughtsConfig,
    existing_profile: &ProfileConfig,
    backend_kind: BackendKind,
    vault_path_flag: Option<String>,
    vault_subpath_flag: Option<String>,
    notion_flags: &NotionFlags,
    anytype_flags: &AnytypeFlags,
    profile: &Option<String>,
    agent_tool: Option<crate::agents::AgentTool>,
) -> Result<ThoughtsConfig> {
    let theme = ColorfulTheme::default();

    let new_backend = match backend_kind {
        BackendKind::Git => {
            let prior = existing_profile.backend.as_git();
            let fallback = get_default_thoughts_repo()?.display().to_string();
            let default_repo = prior
                .map(|g| g.thoughts_repo.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or(fallback);
            let repo: String = Input::with_theme(&theme)
                .with_prompt("Thoughts repository location")
                .default(default_repo.clone())
                .allow_empty(true)
                .interact()
                .map(|s: String| if s.is_empty() { default_repo } else { s })?;

            println!();
            let default_repos_dir = prior
                .map(|g| g.repos_dir.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "repos".to_string());
            let repos_dir: String = Input::with_theme(&theme)
                .with_prompt("Directory name for repository-specific thoughts")
                .default(default_repos_dir)
                .interact()?;

            let default_global_dir = prior
                .map(|g| g.global_dir.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "global".to_string());
            let global_dir: String = Input::with_theme(&theme)
                .with_prompt("Directory name for global thoughts")
                .default(default_global_dir)
                .interact()?;

            BackendConfig::Git(GitConfig {
                thoughts_repo: repo,
                repos_dir,
                global_dir,
            })
        }
        BackendKind::Obsidian => {
            let prior = existing_profile.backend.as_obsidian();
            let existing_vault = prior.map(|o| o.vault_path.as_str()).unwrap_or("");
            let vault_path = match vault_path_flag {
                Some(v) => v,
                None => prompt_vault_path(&theme, existing_vault)?,
            };
            let default_sub = prior
                .and_then(|o| o.vault_subpath.clone())
                .unwrap_or_else(|| "hyprlayer".to_string());
            let vault_subpath = match vault_subpath_flag {
                Some(v) => v,
                None => Input::with_theme(&theme)
                    .with_prompt("Subfolder within vault (leave blank for vault root)")
                    .default(default_sub)
                    .allow_empty(true)
                    .interact()?,
            };

            println!();
            let default_repos_dir = prior
                .map(|o| o.repos_dir.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "repos".to_string());
            let repos_dir: String = Input::with_theme(&theme)
                .with_prompt("Directory name for repository-specific thoughts")
                .default(default_repos_dir)
                .interact()?;
            let default_global_dir = prior
                .map(|o| o.global_dir.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "global".to_string());
            let global_dir: String = Input::with_theme(&theme)
                .with_prompt("Directory name for global thoughts")
                .default(default_global_dir)
                .interact()?;

            BackendConfig::Obsidian(ObsidianConfig {
                vault_path,
                vault_subpath: Some(vault_subpath),
                repos_dir,
                global_dir,
            })
        }
        BackendKind::Notion => BackendConfig::Notion(prompt_notion_config(
            &theme,
            existing_profile.backend.as_notion(),
            notion_flags,
        )?),
        BackendKind::Anytype => BackendConfig::Anytype(prompt_anytype_config(
            &theme,
            existing_profile.backend.as_anytype(),
            anytype_flags,
            agent_tool,
        )?),
    };

    let user = prompt_for_username(&theme, &existing.user)?;

    let mut out = ThoughtsConfig {
        user,
        repo_mappings: existing.repo_mappings,
        profiles: existing.profiles,
        backend: existing.backend,
    };
    match profile.as_ref() {
        Some(name) => {
            out.profiles.insert(
                name.clone(),
                ProfileConfig {
                    backend: new_backend,
                },
            );
        }
        None => {
            out.backend = new_backend;
        }
    }

    Ok(out)
}

fn prompt_notion_config(
    theme: &ColorfulTheme,
    existing: Option<&NotionConfig>,
    flags: &NotionFlags,
) -> Result<NotionConfig> {
    let default_parent = existing
        .map(|n| n.parent_page_id.clone())
        .unwrap_or_default();
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

    let default_db = existing
        .and_then(|n| n.database_id.clone())
        .unwrap_or_default();
    let db_input = match flags.database_id.clone() {
        Some(v) => v,
        None => Input::<String>::with_theme(theme)
            .with_prompt("Existing Notion database ID (leave blank to create on first use)")
            .default(default_db)
            .allow_empty(true)
            .interact()?,
    };
    let database_id = Some(db_input).filter(|s| !s.trim().is_empty());

    Ok(NotionConfig {
        parent_page_id,
        database_id,
    })
}

fn prompt_anytype_config(
    theme: &ColorfulTheme,
    existing: Option<&AnytypeConfig>,
    flags: &AnytypeFlags,
    agent_tool: Option<crate::agents::AgentTool>,
) -> Result<AnytypeConfig> {
    let default_space = existing.map(|a| a.space_id.clone()).unwrap_or_default();
    let space_id = match flags.space_id.clone() {
        Some(v) => v,
        None => {
            let mut input = Input::<String>::with_theme(theme).with_prompt(
                "Anytype space ID (open Space settings in Anytype, or list via the \
                     Anytype MCP's API-list-spaces)",
            );
            if !default_space.is_empty() {
                input = input.default(default_space);
            }
            input.interact()?
        }
    };
    if space_id.trim().is_empty() {
        return Err(anyhow::anyhow!("Anytype space ID is required"));
    }

    // Skip the token-env prompt when Anytype MCP is already wired up. The
    // token is only used by the MCP server, and hyprlayer never reads its
    // value directly.
    let mcp_already_registered = agent_tool
        .map(crate::backends::anytype::is_anytype_mcp_registered)
        .unwrap_or(false);
    let api_token_env = if mcp_already_registered {
        println!(
            "{}",
            "Anytype MCP already wired up — skipping token env-var prompt.".bright_black()
        );
        existing.and_then(|a| a.api_token_env.clone())
    } else if let Some(v) = flags.api_token_env.clone() {
        Some(v)
    } else {
        let default_env = existing
            .and_then(|a| a.api_token_env.clone())
            .unwrap_or_else(|| crate::backends::anytype::DEFAULT_ANYTYPE_TOKEN_ENV.to_string());
        Some(
            Input::<String>::with_theme(theme)
                .with_prompt(
                    "Env var NAME holding your Anytype API key (name only, never the value)",
                )
                .default(default_env)
                .interact()?,
        )
    };

    let default_type = existing.and_then(|a| a.type_id.clone()).unwrap_or_default();
    let type_input = match flags.type_id.clone() {
        Some(v) => v,
        None => Input::<String>::with_theme(theme)
            .with_prompt("Existing Anytype type ID (leave blank to create on first use)")
            .default(default_type)
            .allow_empty(true)
            .interact()?,
    };
    let type_id = Some(type_input).filter(|s| !s.trim().is_empty());

    Ok(AnytypeConfig {
        space_id,
        type_id,
        api_token_env,
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

    let reconfigure = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Already configured for this repository. Reconfigure?")
        .default(false)
        .interact()?;
    Ok(reconfigure)
}

fn resolve_content_root(backend: &BackendConfig) -> Result<PathBuf> {
    match backend {
        BackendConfig::Git(g) => Ok(expand_path(&g.thoughts_repo)),
        BackendConfig::Obsidian(o) => {
            // Check vault existence here, before `ensure_content_root` would
            // create the missing path. Obsidian vaults are user-managed — we
            // never auto-create them.
            if o.vault_path.is_empty() {
                return Err(anyhow::anyhow!(
                    "Obsidian backend requires vaultPath in settings"
                ));
            }
            let vault = expand_path(&o.vault_path);
            if !vault.exists() {
                return Err(anyhow::anyhow!(
                    "Obsidian vault does not exist: {}. Create it in Obsidian first.",
                    vault.display()
                ));
            }
            o.obsidian_root()
                .ok_or_else(|| anyhow::anyhow!("Obsidian backend requires vaultPath in settings"))
        }
        other => Err(anyhow::anyhow!(
            "Backend '{}' has no content root",
            other.kind().as_str()
        )),
    }
}

fn ensure_content_root(content_root: &Path) -> Result<()> {
    if !content_root.exists() {
        fs::create_dir_all(content_root)?;
    }
    Ok(())
}

/// Non-interactive Obsidian variant: `--vault-path` is required (no safe default),
/// `--vault-subpath` falls back to the prior value then `hyprlayer`.
fn obsidian_variant_non_interactive(
    vault_path_flag: Option<String>,
    vault_subpath_flag: Option<String>,
    prior: Option<&ObsidianConfig>,
) -> Result<BackendConfig> {
    let vault_path = vault_path_flag
        .or_else(|| {
            prior
                .map(|o| o.vault_path.clone())
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("--vault-path is required when --backend obsidian is used with --yes")
        })?;
    let vault_subpath = vault_subpath_flag
        .or_else(|| prior.and_then(|o| o.vault_subpath.clone()))
        .unwrap_or_else(|| "hyprlayer".to_string());
    let repos_dir = prior
        .map(|o| o.repos_dir.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "repos".to_string());
    let global_dir = prior
        .map(|o| o.global_dir.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "global".to_string());
    Ok(BackendConfig::Obsidian(ObsidianConfig {
        vault_path,
        vault_subpath: Some(vault_subpath),
        repos_dir,
        global_dir,
    }))
}

/// Non-interactive Notion variant. `--parent-page-id` is required.
fn notion_variant_non_interactive(
    flags: NotionFlags,
    prior: Option<&NotionConfig>,
) -> Result<BackendConfig> {
    let NotionFlags {
        parent_page_id,
        database_id,
    } = flags;
    let parent = parent_page_id
        .or_else(|| prior.map(|n| n.parent_page_id.clone()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("--parent-page-id is required when --backend notion is used with --yes")
        })?;
    let database_id = database_id.or_else(|| prior.and_then(|n| n.database_id.clone()));
    Ok(BackendConfig::Notion(NotionConfig {
        parent_page_id: parent,
        database_id,
    }))
}

/// Non-interactive Anytype variant. `--space-id` is required; `--api-token-env`
/// defaults to `ANYTYPE_API_KEY` when unset.
fn anytype_variant_non_interactive(
    flags: AnytypeFlags,
    prior: Option<&AnytypeConfig>,
) -> Result<BackendConfig> {
    let AnytypeFlags {
        space_id,
        type_id,
        api_token_env,
    } = flags;
    let space = space_id
        .or_else(|| prior.map(|a| a.space_id.clone()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("--space-id is required when --backend anytype is used with --yes")
        })?;
    let token_env = api_token_env
        .or_else(|| prior.and_then(|a| a.api_token_env.clone()))
        .or_else(|| Some(crate::backends::anytype::DEFAULT_ANYTYPE_TOKEN_ENV.to_string()));
    let type_id = type_id.or_else(|| prior.and_then(|a| a.type_id.clone()));
    Ok(BackendConfig::Anytype(AnytypeConfig {
        space_id: space,
        type_id,
        api_token_env: token_env,
    }))
}

fn apply_backend(thoughts: &mut ThoughtsConfig, profile: &Option<String>, backend: BackendConfig) {
    match profile.as_ref() {
        Some(name) => {
            if let Some(p) = thoughts.profiles.get_mut(name) {
                p.backend = backend;
            } else {
                thoughts
                    .profiles
                    .insert(name.clone(), ProfileConfig { backend });
            }
        }
        None => {
            thoughts.backend = backend;
        }
    }
}

fn select_or_create_directory(
    repos_path: &Path,
    current_repo: &Path,
    directory: Option<String>,
) -> Result<String> {
    if let Some(dir) = directory {
        return use_existing_directory(repos_path, &dir);
    }

    let existing_repos = list_existing_repos(repos_path)?;

    if existing_repos.is_empty() {
        prompt_for_new_directory(current_repo)
    } else {
        select_or_create_from_existing(&existing_repos, current_repo)
    }
}

fn use_existing_directory(repos_path: &Path, dir: &str) -> Result<String> {
    let sanitized = sanitize_directory_name(dir);
    if !repos_path.join(&sanitized).exists() {
        return Err(anyhow::anyhow!(
            "Directory \"{}\" not found in thoughts repository",
            sanitized
        ));
    }
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

fn prompt_for_new_directory(current_repo: &Path) -> Result<String> {
    let default_name = get_repo_name_from_path(current_repo);
    let input: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Directory name for this project's thoughts [{}]",
            default_name
        ))
        .default(default_name)
        .interact()?;

    Ok(sanitize_directory_name(&input))
}

fn select_or_create_from_existing(
    existing_repos: &[String],
    current_repo: &Path,
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
        prompt_for_new_directory(current_repo)
    } else {
        Ok(existing_repos[selection].clone())
    }
}

fn dispatch_backend_init(
    config: &HyprlayerConfig,
    current_repo: &Path,
    backend_kind: BackendKind,
) -> Result<()> {
    let current_repo_str = current_repo.display().to_string();
    let effective = config
        .thoughts
        .as_ref()
        .expect("thoughts config must exist here")
        .effective_config_for(&current_repo_str);

    let agent_tool = config.ai.as_ref().and_then(|a| a.agent_tool);
    let ctx = BackendContext::new(current_repo, &effective).with_agent_tool(agent_tool);
    let backend_impl = backends::for_kind(backend_kind);
    backend_impl.init(&ctx)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn require_git_repo_passes_for_notion_outside_git() {
        let tmp = tempdir().unwrap();
        require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Notion).unwrap();
        require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Anytype).unwrap();
    }

    #[test]
    fn require_git_repo_errors_for_filesystem_backends_outside_git() {
        let tmp = tempdir().unwrap();
        assert!(
            require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Git)
                .unwrap_err()
                .to_string()
                .contains("Not in a git repository")
        );
        assert!(
            require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Obsidian).is_err()
        );
    }

    #[test]
    fn require_git_repo_passes_for_filesystem_backend_inside_git() {
        let tmp = tempdir().unwrap();
        GitRepo::init(tmp.path()).unwrap();
        require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Git).unwrap();
        require_git_repo_for_filesystem_backend(tmp.path(), BackendKind::Obsidian).unwrap();
    }

    /// `resolve_backend_interactive` short-circuits only on an explicit flag.
    /// Every flag-less call drops into the interactive menu (with the current
    /// backend pre-selected), so the user always sees what's set and can
    /// switch. We can't drive the menu in a unit test, so we cover the flag
    /// branch only.
    #[test]
    fn resolve_backend_short_circuits_on_explicit_flag() {
        assert_eq!(
            resolve_backend_interactive(Some(BackendKind::Notion), BackendKind::Git).unwrap(),
            BackendKind::Notion,
        );
        assert_eq!(
            resolve_backend_interactive(Some(BackendKind::Git), BackendKind::Notion).unwrap(),
            BackendKind::Git,
        );
    }
}
