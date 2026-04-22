use anyhow::Result;
use colored::Colorize;

use crate::backends::{self, BackendContext};
use crate::cli::SyncArgs;
use crate::config::get_current_repo_path;

pub fn sync(args: SyncArgs) -> Result<()> {
    let SyncArgs { message, config } = args;
    println!("{}", "Syncing thoughts...".blue());

    let hyprlayer_config = config.load()?;
    let thoughts_config = hyprlayer_config.thoughts.as_ref().unwrap();

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();
    let effective = thoughts_config.effective_config_for(&current_repo_str);

    let ctx = BackendContext::new(&current_repo, &effective);
    let backend = backends::for_kind(effective.backend);
    backend.sync(&ctx, message.as_deref())?;

    Ok(())
}
