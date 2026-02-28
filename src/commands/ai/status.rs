use anyhow::Result;
use colored::Colorize;

use crate::cli::AiStatusArgs;

fn print_not_configured(json: bool) -> Result<()> {
    if json {
        println!("{{}}");
    } else {
        println!("{}", "No AI tool configured.".yellow());
        println!(
            "{}",
            "Run 'hyprlayer ai configure' to set up AI tools.".bright_black()
        );
    }
    Ok(())
}

pub fn status(args: AiStatusArgs) -> Result<()> {
    let AiStatusArgs { json, config } = args;
    let config_path = config.path()?;

    let Some(thoughts_config) = config.load_if_exists()? else {
        return print_not_configured(json);
    };

    let Some(ref agent_tool) = thoughts_config.agent_tool else {
        return print_not_configured(json);
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&agent_tool.status_json(&thoughts_config))?
        );
        return Ok(());
    }

    println!("{}", "AI Tool Configuration".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    agent_tool.print_status(&thoughts_config);

    println!();
    println!(
        "  Config file: {}",
        config_path.display().to_string().bright_black()
    );

    Ok(())
}
