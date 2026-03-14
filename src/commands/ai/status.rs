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

    let Some(hyprlayer_config) = config.load_if_exists()? else {
        return print_not_configured(json);
    };

    let Some(ref ai_config) = hyprlayer_config.ai else {
        return print_not_configured(json);
    };

    let Some(ref agent_tool) = ai_config.agent_tool else {
        return print_not_configured(json);
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&agent_tool.status_json(ai_config))?
        );
        return Ok(());
    }

    println!("{}", "AI Tool Configuration".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();

    agent_tool.print_status(ai_config);

    println!();
    println!(
        "  Config file: {}",
        config_path.display().to_string().bright_black()
    );

    Ok(())
}
