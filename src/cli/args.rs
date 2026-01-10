use clap::Args;

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}
