use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod config;
mod git_ops;

use commands::thoughts::profile::{
    create as profile_create, delete as profile_delete, list as profile_list, show as profile_show,
};
use commands::thoughts::{config_cmd, init, status, sync, uninit};

fn main() -> Result<()> {
    match cli::Cli::parse() {
        cli::Cli::Init(args) => init::init(args.force, args.directory, args.profile, args.config)?,
        cli::Cli::Uninit(args) => uninit::uninit(args.force, args.config)?,
        cli::Cli::Sync(args) => sync::sync(args.message, args.config)?,
        cli::Cli::Status(args) => status::status(args.config)?,
        cli::Cli::Config(args) => config_cmd::config(args.edit, args.json, args.config)?,
        cli::Cli::ProfileCreate(args) => profile_create::create(
            args.name,
            args.repo,
            args.repos_dir,
            args.global_dir,
            args.config,
        )?,
        cli::Cli::ProfileList(args) => profile_list::list(args.json, args.config)?,
        cli::Cli::ProfileShow(args) => profile_show::show(args.name, args.json, args.config)?,
        cli::Cli::ProfileDelete(args) => {
            profile_delete::delete(args.name, args.force, args.config)?
        }
    }

    Ok(())
}
