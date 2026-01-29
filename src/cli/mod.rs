pub mod args;
pub mod commands;

use clap::{Parser, Subcommand};
use commands::*;

#[derive(Parser, Debug)]
#[command(name = "hyprlayer")]
#[command(about = "Manage developer thoughts and notes", long_about = None)]
pub enum Cli {
    /// Manage developer thoughts and notes
    Thoughts {
        #[command(subcommand)]
        command: ThoughtsCommands,
    },
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
