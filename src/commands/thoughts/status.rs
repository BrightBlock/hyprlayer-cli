use anyhow::Result;
use colored::Colorize;
use std::path::MAIN_SEPARATOR_STR as SEP;

use crate::backends::{self, BackendContext};
use crate::cli::StatusArgs;
use crate::config::get_current_repo_path;

pub fn status(args: StatusArgs) -> Result<()> {
    println!("{}", "Thoughts Repository Status".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    let hyprlayer_config = args.config.load()?;
    let thoughts_config = hyprlayer_config.thoughts.as_ref().unwrap();

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();
    let effective = thoughts_config.effective_config_for(&current_repo_str);

    println!("{}", "Configuration:".yellow());
    println!("  Backend: {}", effective.backend.as_str().cyan());
    println!("  Repos directory: {}", effective.repos_dir.cyan());
    println!("  Global directory: {}", effective.global_dir.cyan());
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
        println!(
            "  Thoughts directory: {}{SEP}{}",
            effective.repos_dir.cyan(),
            mapped_name.cyan()
        );

        let thoughts_dir = current_repo.join("thoughts");
        if thoughts_dir.exists() {
            println!("  Status: {}", "✓ Initialized".green());
        } else {
            println!("  Status: {}", "✗ Not initialized".red());
        }
    } else {
        println!("{}", "Current repository not mapped to thoughts".yellow());
    }
    println!();

    let ctx = BackendContext::new(&current_repo, &effective);
    let backend = backends::for_kind(effective.backend);
    let report = backend.status(&ctx)?;
    for line in report.lines {
        println!("{}", line);
    }

    Ok(())
}
