use anyhow::Result;
use colored::Colorize;
use dialoguer::{Select, theme::ColorfulTheme};

use crate::agents::{AgentTool, OpenCodeProvider};
use crate::cli::AiConfigureArgs;
use crate::config::HyprlayerConfig;

pub fn configure(args: AiConfigureArgs) -> Result<()> {
    let AiConfigureArgs { force, config } = args;
    let config_path = config.path()?;

    let mut hyprlayer_config = load_or_create_minimal_config(&config_path)?;

    let existing_agent = hyprlayer_config
        .ai
        .as_ref()
        .and_then(|ai| ai.agent_tool.as_ref());

    if let (Some(agent), false) = (existing_agent, force) {
        println!(
            "{}",
            format!("AI tool already configured: {}", agent).yellow()
        );
        println!("{}", "Use --force to reconfigure.".bright_black());

        if !agent.is_installed() {
            println!();
            println!("{}", "Agent files not found. Installing...".yellow());
            let opencode_provider = hyprlayer_config
                .ai
                .as_ref()
                .and_then(|ai| ai.opencode_provider.as_ref());
            agent.install(opencode_provider)?;
            println!(
                "{}",
                format!("Agent files installed to {}", agent.dest_display()).green()
            );
        }
        return Ok(());
    }

    let theme = ColorfulTheme::default();
    println!("{}", "=== AI Tool Configuration ===".blue());
    println!();

    let agent_tool = prompt_for_agent_tool(&theme)?;

    let (opencode_provider, opencode_sonnet_model, opencode_opus_model) =
        if agent_tool == AgentTool::OpenCode {
            let provider = prompt_for_opencode_provider(&theme)?;
            (
                Some(provider.clone()),
                Some(provider.default_sonnet_model().to_string()),
                Some(provider.default_opus_model().to_string()),
            )
        } else {
            (None, None, None)
        };

    let ai = hyprlayer_config.ai_mut();
    ai.agent_tool = Some(agent_tool);
    ai.opencode_provider = opencode_provider;
    ai.opencode_sonnet_model = opencode_sonnet_model;
    ai.opencode_opus_model = opencode_opus_model;

    hyprlayer_config.save(&config_path)?;
    println!();
    println!("{}", "Configuration saved.".green());

    println!();
    let opencode_provider_ref = hyprlayer_config
        .ai
        .as_ref()
        .and_then(|ai| ai.opencode_provider.as_ref());
    agent_tool.install(opencode_provider_ref)?;
    println!(
        "{}",
        format!(
            "{} agent files installed to {}",
            agent_tool,
            agent_tool.dest_display()
        )
        .green()
    );

    if let Some(ref p) = hyprlayer_config
        .ai
        .as_ref()
        .and_then(|ai| ai.opencode_provider.as_ref())
    {
        println!("{}", format!("Configured for {} provider", p).green());
    }

    println!();
    println!("{}", "AI configuration complete!".green());
    println!(
        "{}",
        "Run 'hyprlayer thoughts init' to set up thoughts for a repository.".bright_black()
    );

    Ok(())
}

fn prompt_for_agent_tool(theme: &ColorfulTheme) -> Result<AgentTool> {
    let options: Vec<String> = AgentTool::ALL.iter().map(|t| t.to_string()).collect();
    let selection = Select::with_theme(theme)
        .with_prompt("Which AI tool do you use?")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(AgentTool::ALL[selection])
}

fn prompt_for_opencode_provider(theme: &ColorfulTheme) -> Result<OpenCodeProvider> {
    let options: Vec<String> = OpenCodeProvider::ALL
        .iter()
        .map(|p| p.to_string())
        .collect();
    let selection = Select::with_theme(theme)
        .with_prompt("Which OpenCode provider do you want to use?")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(OpenCodeProvider::ALL[selection].clone())
}

fn load_or_create_minimal_config(config_path: &std::path::Path) -> Result<HyprlayerConfig> {
    if config_path.exists() {
        return HyprlayerConfig::load(config_path);
    }
    Ok(HyprlayerConfig::default())
}
