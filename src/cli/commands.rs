use super::args::ConfigArgs;
use clap::Args;

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
