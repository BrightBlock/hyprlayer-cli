use anyhow::Result;
use clap::Args;
use std::fs;
use std::path::PathBuf;

use crate::config::{BackendKind, HyprlayerConfig, expand_path, get_default_config_path};

/// Common config file argument shared across commands
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

impl ConfigArgs {
    /// Resolve the config file path (from arg or default)
    pub fn path(&self) -> Result<PathBuf> {
        self.config_file
            .as_ref()
            .map_or_else(get_default_config_path, |p| Ok(expand_path(p)))
    }

    /// Load existing config, error if not found or incomplete
    pub fn load(&self) -> Result<HyprlayerConfig> {
        let config = self.load_if_exists()?.ok_or_else(|| {
            anyhow::anyhow!("No configuration found. Run 'hyprlayer thoughts init' first.")
        })?;
        if config
            .thoughts
            .as_ref()
            .is_none_or(|t| !t.is_thoughts_configured())
        {
            return Err(anyhow::anyhow!(
                "Thoughts not fully configured. Run 'hyprlayer thoughts init' to complete setup."
            ));
        }
        Ok(config)
    }

    /// Load config if exists, returns None if config file doesn't exist
    pub fn load_if_exists(&self) -> Result<Option<HyprlayerConfig>> {
        let path = self.path()?;
        if !path.exists() {
            return Ok(None);
        }
        HyprlayerConfig::load(&path).map(Some)
    }

    /// Load raw JSON config, error if not found
    pub fn load_raw(&self) -> Result<(PathBuf, serde_json::Value)> {
        let path = self.path()?;
        if !path.exists() {
            return Err(anyhow::anyhow!("No thoughts configuration found"));
        }
        let content = fs::read_to_string(&path)?;
        let value = serde_json::from_str(&content)?;
        Ok((path, value))
    }
}

#[derive(Debug, Args)]
#[command(name = "init", about = "Initialize thoughts for current repository")]
pub struct InitArgs {
    #[arg(long, help = "Force reconfiguration even if already set up")]
    pub force: bool,
    #[arg(
        long,
        help = "Specify the repository directory name (skips interactive prompt)"
    )]
    pub directory: Option<String>,
    #[arg(long, help = "Use a specific thoughts profile")]
    pub profile: Option<String>,
    #[arg(long, value_enum, help = "Storage backend for thoughts")]
    pub backend: Option<BackendKind>,
    #[arg(
        long,
        help = "Obsidian vault path (required when --backend obsidian with --yes)"
    )]
    pub vault_path: Option<String>,
    #[arg(
        long,
        help = "Subfolder within the Obsidian vault for hyprlayer content (default: hyprlayer)"
    )]
    pub vault_subpath: Option<String>,
    #[arg(
        long,
        help = "Notion parent page ID (required when --backend notion with --yes)"
    )]
    pub parent_page_id: Option<String>,
    #[arg(
        long,
        help = "Existing Notion database ID to reuse (skips lazy creation)"
    )]
    pub database_id: Option<String>,
    #[arg(
        long,
        help = "Anytype space ID (required when --backend anytype with --yes)"
    )]
    pub space_id: Option<String>,
    #[arg(long, help = "Existing Anytype type ID to reuse (skips lazy creation)")]
    pub type_id: Option<String>,
    #[arg(
        long,
        help = "Env var name holding the Anytype API token (default: ANYTYPE_API_KEY). \
                Ignored for notion (uses agent tool's connector)."
    )]
    pub api_token_env: Option<String>,
    #[arg(
        long,
        short = 'y',
        help = "Run without interactive prompts (requires existing config and --directory)"
    )]
    pub yes: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "uninit",
    about = "Remove thoughts setup from current repository"
)]
pub struct UninitArgs {
    #[arg(long, help = "Force removal even if not in configuration")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "sync", about = "Manually sync thoughts to thoughts repository")]
pub struct SyncArgs {
    #[arg(short, long, help = "Commit message for sync")]
    pub message: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "status", about = "Show status of thoughts repository")]
pub struct StatusArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "config", about = "View or edit thoughts configuration")]
pub struct ConfigArgsCmd {
    #[arg(long, help = "Open configuration in editor")]
    pub edit: bool,
    #[arg(long, help = "Output configuration as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "create", about = "Create a new thoughts profile")]
pub struct ProfileCreateArgs {
    pub name: String,
    #[arg(long, help = "Thoughts repository path")]
    pub repo: Option<String>,
    #[arg(long, help = "Repos directory name")]
    pub repos_dir: Option<String>,
    #[arg(long, help = "Global directory name")]
    pub global_dir: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "list", about = "List all thoughts profiles")]
pub struct ProfileListArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "show", about = "Show details of a specific profile")]
pub struct ProfileShowArgs {
    pub name: String,
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "delete", about = "Delete a thoughts profile")]
pub struct ProfileDeleteArgs {
    pub name: String,
    #[arg(long, help = "Force deletion even if in use")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

// AI command argument structs

#[derive(Debug, Args)]
#[command(
    name = "configure",
    about = "Configure AI tool and install agent files"
)]
pub struct AiConfigureArgs {
    #[arg(long, help = "Force reconfiguration even if already set up")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "status", about = "Show current AI tool configuration")]
pub struct AiStatusArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "reinstall", about = "Reinstall AI agent files")]
pub struct AiReinstallArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "info",
    about = "Show the active storage backend and its settings"
)]
pub struct StorageInfoArgs {
    #[arg(long, help = "Output as JSON for slash-command consumption")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "set-database-id",
    about = "Persist a Notion database ID to the active profile's backend settings"
)]
pub struct StorageSetDatabaseIdArgs {
    pub id: String,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "set-type-id",
    about = "Persist an Anytype type ID to the active profile's backend settings"
)]
pub struct StorageSetTypeIdArgs {
    pub id: String,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "stream",
    about = "Read codex --json output on stdin, write formatted lines to stdout"
)]
pub struct CodexStreamArgs {
    /// Suppress [codex thinking] reasoning lines
    #[arg(long)]
    pub no_thinking: bool,
    /// Suppress [codex ran] command execution lines
    #[arg(long)]
    pub no_tool_calls: bool,
}
