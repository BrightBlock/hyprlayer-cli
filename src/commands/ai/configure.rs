use anyhow::Result;
use colored::Colorize;
use dialoguer::{Select, theme::ColorfulTheme};
use std::fs;
use std::path::Path;

use crate::agents::{AgentTool, OpenCodeProvider};
use crate::cli::AiConfigureArgs;
use crate::config::ThoughtsConfig;

pub fn configure(args: AiConfigureArgs) -> Result<()> {
    let AiConfigureArgs { force, config } = args;
    let config_path = config.path()?;

    let mut thoughts_config = load_or_create_minimal_config(&config_path)?;

    if let Some(ref agent) = thoughts_config.agent_tool
        && !force
    {
        println!(
            "{}",
            format!("AI tool already configured: {}", agent).yellow()
        );
        println!("{}", "Use --force to reconfigure.".bright_black());

        if !agent.is_installed() {
            println!();
            println!("{}", "Agent files not found. Installing...".yellow());
            agent.install(thoughts_config.opencode_provider.as_ref())?;
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

    thoughts_config.agent_tool = Some(agent_tool.clone());
    thoughts_config.opencode_provider = opencode_provider;
    thoughts_config.opencode_sonnet_model = opencode_sonnet_model;
    thoughts_config.opencode_opus_model = opencode_opus_model;

    save_config(&config_path, &thoughts_config)?;
    println!();
    println!("{}", "Configuration saved.".green());

    println!();
    agent_tool.install(thoughts_config.opencode_provider.as_ref())?;
    println!(
        "{}",
        format!(
            "{} agent files installed to {}",
            agent_tool,
            agent_tool.dest_display()
        )
        .green()
    );

    if let Some(ref p) = thoughts_config.opencode_provider {
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

    Ok(AgentTool::ALL[selection].clone())
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

fn load_or_create_minimal_config(config_path: &Path) -> Result<ThoughtsConfig> {
    if config_path.exists() {
        let content = fs::read_to_string(config_path)?;
        let config_file: serde_json::Value = serde_json::from_str(&content)?;

        if let Some(thoughts) = config_file.get("thoughts") {
            return serde_json::from_value(thoughts.clone())
                .map_err(|e| anyhow::anyhow!("Failed to parse thoughts config: {}", e));
        }
    }

    Ok(ThoughtsConfig {
        thoughts_repo: String::new(),
        repos_dir: String::new(),
        global_dir: String::new(),
        user: String::new(),
        agent_tool: None,
        opencode_provider: None,
        opencode_sonnet_model: None,
        opencode_opus_model: None,
        repo_mappings: Default::default(),
        profiles: Default::default(),
        last_version_check: None,
        disable_update_check: false,
    })
}

fn save_config(config_path: &Path, thoughts_config: &ThoughtsConfig) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut config_json: serde_json::Value = config_path
        .exists()
        .then(|| fs::read_to_string(config_path))
        .transpose()?
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    config_json["thoughts"] = serde_json::to_value(thoughts_config)?;

    let json = serde_json::to_string_pretty(&config_json)?;
    fs::write(config_path, json)?;

    Ok(())
}
