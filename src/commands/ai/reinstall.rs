use anyhow::Result;
use colored::Colorize;

use crate::cli::AiReinstallArgs;

pub fn reinstall(args: AiReinstallArgs) -> Result<()> {
    let AiReinstallArgs { config } = args;

    // Load config with descriptive error using combinator
    let thoughts_config = config.load().map_err(|_| {
        anyhow::anyhow!("No configuration found. Run 'hyprlayer ai configure' first.")
    })?;

    // Extract agent tool with ok_or_else for Option -> Result conversion
    let agent_tool = thoughts_config.agent_tool.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No AI tool configured. Run 'hyprlayer ai configure' first.")
    })?;

    println!(
        "{}",
        format!("Reinstalling {} agent files...", agent_tool).yellow()
    );

    agent_tool.install(thoughts_config.opencode_provider.as_ref())?;

    println!(
        "{}",
        format!(
            "{} agent files reinstalled to {}",
            agent_tool,
            agent_tool.dest_display()
        )
        .green()
    );

    // Display provider info using if-let pattern
    if let Some(ref p) = thoughts_config.opencode_provider {
        println!("{}", format!("Configured for {} provider", p).green());
    }

    Ok(())
}
