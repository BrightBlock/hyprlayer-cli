use anyhow::Result;
use clap::Parser;

pub mod agents;
mod backends;
mod cli;
mod commands;
mod config;
mod git_ops;
mod hooks;
mod version;

use cli::{AiCommands, CodexCommands, ProfileCommands, StorageCommands, ThoughtsCommands};
use commands::ai::{configure as ai_configure, reinstall as ai_reinstall, status as ai_status};
use commands::codex::stream as codex_stream;
use commands::storage::{
    info as storage_info, set_database_id as storage_set_database_id,
    set_type_id as storage_set_type_id,
};
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
        cli::Cli::Storage { command } => match command {
            StorageCommands::Info(args) => storage_info::info(args)?,
            StorageCommands::SetDatabaseId(args) => storage_set_database_id::set_database_id(args)?,
            StorageCommands::SetTypeId(args) => storage_set_type_id::set_type_id(args)?,
        },
        cli::Cli::Codex { command } => match command {
            CodexCommands::Stream(args) => codex_stream::stream(args)?,
        },
    }

    Ok(())
}
