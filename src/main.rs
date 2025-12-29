use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod config;
mod git_ops;

use commands::thoughts::{init, sync, status, uninit, config_cmd};
use commands::thoughts::profile::{create as profile_create, list as profile_list, show as profile_show, delete as profile_delete};

#[derive(Parser, Debug)]
#[command(name = "hyprlayer")]
#[command(about = "Manage developer thoughts and notes", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage developer thoughts and notes
    Thoughts {
        #[command(subcommand)]
        subcommand: ThoughtsCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ThoughtsCommands {
    /// Initialize thoughts for current repository
    Init {
        #[arg(long, help = "Force reconfiguration even if already set up")]
        force: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
        #[arg(long, help = "Specify the repository directory name (skips interactive prompt)")]
        directory: Option<String>,
        #[arg(long, help = "Use a specific thoughts profile")]
        profile: Option<String>,
    },
    /// Remove thoughts setup from current repository
    Uninit {
        #[arg(long, help = "Force removal even if not in configuration")]
        force: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// Manually sync thoughts to thoughts repository
    Sync {
        #[arg(short, long, help = "Commit message for sync")]
        message: Option<String>,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// Show status of thoughts repository
    Status {
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// View or edit thoughts configuration
    Config {
        #[arg(long, help = "Open configuration in editor")]
        edit: bool,
        #[arg(long, help = "Output configuration as JSON")]
        json: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// Manage thoughts profiles
    Profile {
        #[command(subcommand)]
        subcommand: ProfileCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ProfileCommands {
    /// Create a new thoughts profile
    Create {
        name: String,
        #[arg(long, help = "Thoughts repository path")]
        repo: Option<String>,
        #[arg(long, help = "Repos directory name")]
        repos_dir: Option<String>,
        #[arg(long, help = "Global directory name")]
        global_dir: Option<String>,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// List all thoughts profiles
    List {
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// Show details of a specific profile
    Show {
        name: String,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
    /// Delete a thoughts profile
    Delete {
        name: String,
        #[arg(long, help = "Force deletion even if in use")]
        force: bool,
        #[arg(long, help = "Path to config file")]
        config_file: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Thoughts { subcommand } => match subcommand {
            ThoughtsCommands::Init { force, config_file, directory, profile } => {
                init::init(init::InitOptions { force, config_file, directory, profile })?;
            }
            ThoughtsCommands::Uninit { force, config_file } => {
                uninit::uninit(uninit::UninitOptions { force, config_file })?;
            }
            ThoughtsCommands::Sync { message, config_file } => {
                sync::sync(sync::SyncOptions { message, config_file })?;
            }
            ThoughtsCommands::Status { config_file } => {
                status::status(status::StatusOptions { config_file })?;
            }
            ThoughtsCommands::Config { edit, json, config_file } => {
                config_cmd::config(config_cmd::ConfigOptions { edit, json, config_file })?;
            }
            ThoughtsCommands::Profile { subcommand } => match subcommand {
                ProfileCommands::Create { name, repo, repos_dir, global_dir, config_file } => {
                    profile_create::create(name, profile_create::CreateOptions { repo, repos_dir, global_dir, config_file })?;
                }
                ProfileCommands::List { json, config_file } => {
                    profile_list::list(profile_list::ListOptions { json, config_file })?;
                }
                ProfileCommands::Show { name, json, config_file } => {
                    profile_show::show(name, profile_show::ShowOptions { json, config_file })?;
                }
                ProfileCommands::Delete { name, force, config_file } => {
                    profile_delete::delete(name, profile_delete::DeleteOptions { force, config_file })?;
                }
            }
        }
    }

    Ok(())
}
