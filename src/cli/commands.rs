use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::config::{expand_path, get_default_config_path, ThoughtsConfig};

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

    /// Load existing config, error if not found
    pub fn load(&self) -> Result<ThoughtsConfig> {
        let path = self.path()?;
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "No thoughts configuration found. Run 'hyprlayer thoughts init' first."
            ));
        }
        ThoughtsConfig::load(&path)
    }

    /// Load config if exists, returns None if config file doesn't exist
    pub fn load_if_exists(&self) -> Result<Option<ThoughtsConfig>> {
        let path = self.path()?;
        if !path.exists() {
            return Ok(None);
        }
        ThoughtsConfig::load(&path).map(Some)
    }
}

#[derive(Debug, Args)]
#[command(
    name = "init",
    about = "Initialize thoughts for current repository",
    long_about = "Initialize thoughts for current repository"
)]
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
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "uninit",
    about = "Remove thoughts setup from current repository",
    long_about = "Remove thoughts setup from current repository"
)]
pub struct UninitArgs {
    #[arg(long, help = "Force removal even if not in configuration")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "sync",
    about = "Manually sync thoughts to thoughts repository",
    long_about = "Manually sync thoughts to thoughts repository"
)]
pub struct SyncArgs {
    #[arg(short, long, help = "Commit message for sync")]
    pub message: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "status",
    about = "Show status of thoughts repository",
    long_about = "Show status of thoughts repository"
)]
pub struct StatusArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "config",
    about = "View or edit thoughts configuration",
    long_about = "View or edit thoughts configuration"
)]
pub struct ConfigArgsCmd {
    #[arg(long, help = "Open configuration in editor")]
    pub edit: bool,
    #[arg(long, help = "Output configuration as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "create",
    about = "Create a new thoughts profile",
    long_about = "Create a new thoughts profile"
)]
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
#[command(
    name = "list",
    about = "List all thoughts profiles",
    long_about = "List all thoughts profiles"
)]
pub struct ProfileListArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "show",
    about = "Show details of a specific profile",
    long_about = "Show details of a specific profile"
)]
pub struct ProfileShowArgs {
    pub name: String,
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "delete",
    about = "Delete a thoughts profile",
    long_about = "Delete a thoughts profile"
)]
pub struct ProfileDeleteArgs {
    pub name: String,
    #[arg(long, help = "Force deletion even if in use")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}
