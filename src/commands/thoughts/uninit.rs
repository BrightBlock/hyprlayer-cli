use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cli::UninitArgs;
use crate::config::{HyprlayerConfig, get_current_repo_path};

fn remove_from_config(config_path: &Path, repo_key: &str) -> Result<()> {
    let mut config = HyprlayerConfig::load(config_path)?;
    config.thoughts_mut().repo_mappings.remove(repo_key);
    config.save(config_path)?;
    Ok(())
}

pub fn uninit(args: UninitArgs) -> Result<()> {
    let UninitArgs { force, config } = args;
    let current_repo = get_current_repo_path()?;
    let thoughts_dir = current_repo.join("thoughts");

    let config_path = config.path()?;
    let hyprlayer_config = config.load_if_exists()?;
    let current_repo_str = current_repo.display().to_string();

    let is_mapped = hyprlayer_config
        .as_ref()
        .and_then(|c| c.thoughts.as_ref())
        .map(|t| t.effective_config_for(&current_repo_str))
        .is_some_and(|e| e.mapped_name.is_some());

    // Filesystem backends leave a `thoughts/` directory; Notion/Anytype don't.
    // Treat either as evidence that this repo was set up.
    if !force && !is_mapped && !thoughts_dir.exists() {
        return Err(anyhow::anyhow!(
            "Thoughts not configured for this repository. Use --force to override."
        ));
    }

    if thoughts_dir.exists() {
        let searchable_dir = thoughts_dir.join("searchable");
        if searchable_dir.exists() {
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("chmod")
                    .args(["-R", "755"])
                    .arg(&searchable_dir)
                    .output();
            }
            fs::remove_dir_all(&searchable_dir)?;
        }
        fs::remove_dir_all(&thoughts_dir)?;
    }

    if is_mapped && config_path.exists() {
        remove_from_config(&config_path, &current_repo_str)?;
    }

    Ok(())
}
