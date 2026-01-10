pub mod args;
pub mod commands;

use clap::Parser;
use commands::*;

#[derive(Parser, Debug)]
#[command(name = "hyprlayer")]
#[command(about = "Manage developer thoughts and notes", long_about = None)]
pub enum Cli {
    Init(InitArgs),
    Uninit(UninitArgs),
    Sync(SyncArgs),
    Status(StatusArgs),
    Config(ConfigArgsCmd),
    ProfileCreate(ProfileCreateArgs),
    ProfileList(ProfileListArgs),
    ProfileShow(ProfileShowArgs),
    ProfileDelete(ProfileDeleteArgs),
}
