use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod config;
mod git_ops;

use cli::{ProfileCommands, ThoughtsCommands};
use commands::thoughts::profile::{
    create as profile_create, delete as profile_delete, list as profile_list, show as profile_show,
};
use commands::thoughts::{config_cmd, init, status, sync, uninit};

fn main() -> Result<()> {
    match cli::Cli::parse() {
        cli::Cli::Thoughts { command } => match command {
            ThoughtsCommands::Init(args) => {
                init::init(args.force, args.directory, args.profile, args.config)?
            }
            ThoughtsCommands::Uninit(args) => uninit::uninit(args.force, args.config)?,
            ThoughtsCommands::Sync(args) => sync::sync(args.message, args.config)?,
            ThoughtsCommands::Status(args) => status::status(args.config)?,
            ThoughtsCommands::Config(args) => config_cmd::config(args.edit, args.json, args.config)?,
            ThoughtsCommands::Profile { command } => match command {
                ProfileCommands::Create(args) => profile_create::create(
                    args.name,
                    args.repo,
                    args.repos_dir,
                    args.global_dir,
                    args.config,
                )?,
                ProfileCommands::List(args) => profile_list::list(args.json, args.config)?,
                ProfileCommands::Show(args) => profile_show::show(args.name, args.json, args.config)?,
                ProfileCommands::Delete(args) => {
                    profile_delete::delete(args.name, args.force, args.config)?
                }
            },
        },
    }

    Ok(())
}
