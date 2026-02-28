use anyhow::Result;
use clap::Parser;

pub mod agents;
mod cli;
mod commands;
mod config;
mod git_ops;
mod hooks;
mod version;

use cli::{AiCommands, ProfileCommands, ThoughtsCommands};
use commands::ai::{configure as ai_configure, reinstall as ai_reinstall, status as ai_status};
use commands::thoughts::profile::{
    create as profile_create, delete as profile_delete, list as profile_list, show as profile_show,
};
use commands::thoughts::{config_cmd, init, status, sync, uninit};

fn main() -> Result<()> {
    // Check for updates before running command
    version::maybe_check_for_updates();

    match cli::Cli::parse() {
        cli::Cli::Thoughts { command } => match command {
            ThoughtsCommands::Init(args) => init::init(args)?,
            ThoughtsCommands::Uninit(args) => uninit::uninit(args)?,
            ThoughtsCommands::Sync(args) => sync::sync(args)?,
            ThoughtsCommands::Status(args) => status::status(args)?,
            ThoughtsCommands::Config(args) => config_cmd::config(args)?,
            ThoughtsCommands::Profile { command } => match command {
                ProfileCommands::Create(args) => profile_create::create(args)?,
                ProfileCommands::List(args) => profile_list::list(args)?,
                ProfileCommands::Show(args) => profile_show::show(args)?,
                ProfileCommands::Delete(args) => profile_delete::delete(args)?,
            },
        },
        cli::Cli::Ai { command } => match command {
            AiCommands::Configure(args) => ai_configure::configure(args)?,
            AiCommands::Status(args) => ai_status::status(args)?,
            AiCommands::Reinstall(args) => ai_reinstall::reinstall(args)?,
        },
    }

    Ok(())
}
