use anyhow::Result;
use colored::Colorize;
use std::path::MAIN_SEPARATOR_STR as SEP;

use crate::backends::{self, BackendContext};
use crate::cli::StatusArgs;
use crate::config::{BackendConfig, get_current_repo_path};

pub fn status(args: StatusArgs) -> Result<()> {
    let hyprlayer_config = args.config.load()?;
    let thoughts_config = hyprlayer_config.thoughts.as_ref().unwrap();

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();
    let effective = thoughts_config.effective_config_for(&current_repo_str);

    println!("{}", "Configuration:".yellow());
    println!("  Backend: {}", effective.backend.kind().as_str().cyan());
    match &effective.backend {
        BackendConfig::Git(g) => {
            println!("  Thoughts repo: {}", g.thoughts_repo.cyan());
            println!("  Repos directory: {}", g.repos_dir.cyan());
            println!("  Global directory: {}", g.global_dir.cyan());
        }
        BackendConfig::Obsidian(o) => {
            println!("  Vault path: {}", o.vault_path.cyan());
            if let Some(sub) = &o.vault_subpath {
                println!("  Vault subpath: {}", sub.cyan());
            }
            println!("  Repos directory: {}", o.repos_dir.cyan());
            println!("  Global directory: {}", o.global_dir.cyan());
        }
        BackendConfig::Notion(_) | BackendConfig::Anytype(_) => {}
    }
    println!("  User: {}", thoughts_config.user.cyan());
    if let Some(ref profile) = effective.profile_name {
        println!("  Profile: {}", profile.cyan());
    }
    println!(
        "  Mapped repos: {}",
        thoughts_config.repo_mappings.len().to_string().cyan()
    );
    println!();

    if let Some(ref mapped_name) = effective.mapped_name {
        println!("{}", "Current Repository:".yellow());
        println!("  Path: {}", current_repo_str.cyan());

        if let Some(repos_dir) = effective.backend.filesystem_repos_dir() {
            println!(
                "  Thoughts directory: {}{SEP}{}",
                repos_dir.cyan(),
                mapped_name.cyan()
            );

            let thoughts_dir = current_repo.join("thoughts");
            if thoughts_dir.exists() {
                println!("  Status: {}", "Initialized".green());
            } else {
                println!("  Status: {}", "Not initialized".red());
            }
        }
    } else {
        println!("{}", "Current repository not mapped to thoughts".yellow());
    }
    println!();

    let agent_tool = hyprlayer_config.ai.as_ref().and_then(|a| a.agent_tool);
    let ctx = BackendContext::new(&current_repo, &effective).with_agent_tool(agent_tool);
    let backend = backends::for_kind(effective.backend.kind());
    let report = backend.status(&ctx)?;
    for line in report.lines {
        println!("{}", line);
    }

    Ok(())
}
