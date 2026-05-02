use anyhow::Result;
use chrono_humanize::{Accuracy, HumanTime, Tense};
use colored::Colorize;
use std::time::{Duration, UNIX_EPOCH};

use crate::cli::AiStatusArgs;
use crate::config::HyprlayerConfig;

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
        let mut value = agent_tool.status_json(ai_config);
        if let Some(map) = value.as_object_mut() {
            map.insert(
                "agentsInstalledSha".to_string(),
                hyprlayer_config
                    .agents_installed_sha
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            );
            map.insert(
                "lastAgentCheck".to_string(),
                hyprlayer_config
                    .last_agent_check
                    .map(|t| serde_json::Value::Number(t.into()))
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    agent_tool.print_status(ai_config);
    print_bundle_freshness(&hyprlayer_config);

    println!();
    println!(
        "  Config file: {}",
        config_path.display().to_string().bright_black()
    );

    Ok(())
}

/// Render the cached bundle SHA + last-check timestamp under the per-tool
/// status block. Skipped entirely when no SHA is cached, so users who
/// configured an AI tool but haven't yet hit an auto-reinstall window
/// don't see empty placeholder lines.
fn print_bundle_freshness(config: &HyprlayerConfig) {
    let Some(sha) = config.agents_installed_sha.as_deref() else {
        return;
    };
    let short = sha.get(..7).unwrap_or(sha);

    let last_check = config.last_agent_check.and_then(|t| {
        u64::try_from(t)
            .ok()
            .map(|s| HumanTime::from(UNIX_EPOCH + Duration::from_secs(s)))
    });

    println!();
    println!("  Bundle SHA: {}", short.cyan());
    if let Some(ht) = last_check {
        println!(
            "  Last check: {}",
            ht.to_text_en(Accuracy::Rough, Tense::Past).cyan()
        );
    }
}
