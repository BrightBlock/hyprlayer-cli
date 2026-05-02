pub mod commands;

use clap::{Parser, Subcommand};
pub use commands::*;

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_COMMIT"), ")");

#[derive(Parser, Debug)]
#[command(name = "hyprlayer")]
#[command(version = VERSION)]
#[command(about = "Manage developer thoughts and notes", long_about = None)]
pub enum Cli {
    /// Manage developer thoughts and notes
    Thoughts {
        #[command(subcommand)]
        command: ThoughtsCommands,
    },
    /// Manage AI tool configuration
    Ai {
        #[command(subcommand)]
        command: AiCommands,
    },
    /// Inspect the active storage backend for thoughts content
    Storage {
        #[command(subcommand)]
        command: StorageCommands,
    },
    /// Process OpenAI Codex CLI JSONL output
    Codex {
        #[command(subcommand)]
        command: CodexCommands,
    },
}

impl Cli {
    /// The `ConfigArgs` of whichever leaf subcommand was selected, or
    /// `None` for subcommands that don't read config (e.g. `codex stream`,
    /// a stdin/stdout filter). Used by startup checks to honor
    /// `--config-file` and per-config `disableUpdateCheck` settings.
    pub fn config_args(&self) -> Option<&ConfigArgs> {
        match self {
            Cli::Thoughts { command } => Some(match command {
                ThoughtsCommands::Init(a) => &a.config,
                ThoughtsCommands::Uninit(a) => &a.config,
                ThoughtsCommands::Sync(a) => &a.config,
                ThoughtsCommands::Status(a) => &a.config,
                ThoughtsCommands::Config(a) => &a.config,
                ThoughtsCommands::Profile { command } => match command {
                    ProfileCommands::Create(a) => &a.config,
                    ProfileCommands::List(a) => &a.config,
                    ProfileCommands::Show(a) => &a.config,
                    ProfileCommands::Delete(a) => &a.config,
                },
            }),
            Cli::Ai { command } => Some(match command {
                AiCommands::Configure(a) => &a.config,
                AiCommands::Status(a) => &a.config,
                AiCommands::Reinstall(a) => &a.config,
            }),
            Cli::Storage { command } => Some(match command {
                StorageCommands::Info(a) => &a.config,
                StorageCommands::SetDatabaseId(a) => &a.config,
                StorageCommands::SetTypeId(a) => &a.config,
            }),
            Cli::Codex { .. } => None,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum AiCommands {
    Configure(AiConfigureArgs),
    Status(AiStatusArgs),
    Reinstall(AiReinstallArgs),
}

#[derive(Subcommand, Debug)]
pub enum ThoughtsCommands {
    Init(InitArgs),
    Uninit(UninitArgs),
    Sync(SyncArgs),
    Status(StatusArgs),
    Config(ConfigArgsCmd),
    /// Manage thoughts profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommands {
    Create(ProfileCreateArgs),
    List(ProfileListArgs),
    Show(ProfileShowArgs),
    Delete(ProfileDeleteArgs),
}

#[derive(Subcommand, Debug)]
pub enum StorageCommands {
    Info(StorageInfoArgs),
    SetDatabaseId(StorageSetDatabaseIdArgs),
    SetTypeId(StorageSetTypeIdArgs),
}

#[derive(Subcommand, Debug)]
pub enum CodexCommands {
    /// Read codex --json output on stdin, write formatted lines to stdout
    Stream(CodexStreamArgs),
}
