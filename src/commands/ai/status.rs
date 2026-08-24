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
        let mut value = agent_tool.status_json(ai_config, &hyprlayer_config);
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

/// Render which assets bundle is installed, whether it is pinned, and the
/// legacy SHA + last-check timestamp, under the per-tool status block.
///
/// The version triple is the human counterpart of `status_json`'s
/// `assetsVersion` / `pinnedVersion` / `binaryVersion`, and is what makes a
/// skew visible: a pin held across a binary upgrade shows an assets version
/// that is deliberately not the binary's own.
///
/// The `Pinned:` line is omitted when there is no pin, and the SHA and
/// last-check lines when there is nothing cached — a pre-1.6.0 install
/// carries a SHA and no version, a 1.6.0 one the reverse, and neither
/// should show empty placeholders for the other's state.
fn print_bundle_freshness(config: &HyprlayerConfig) {
    println!();
    println!(
        "  Assets version: {}",
        config
            .agents_installed_version
            .as_deref()
            .unwrap_or("unknown")
            .cyan()
    );
    if let Some(pinned) = config.agents_pinned_version.as_deref() {
        println!("  Pinned: {}", pinned.cyan());
    }
    println!("  Binary version: {}", env!("CARGO_PKG_VERSION").cyan());

    if let Some(sha) = config.agents_installed_sha.as_deref() {
        println!("  Bundle SHA: {}", sha.get(..7).unwrap_or(sha).cyan());
    }

    let last_check = config.last_agent_check.and_then(|t| {
        u64::try_from(t)
            .ok()
            .map(|s| HumanTime::from(UNIX_EPOCH + Duration::from_secs(s)))
    });
    if let Some(ht) = last_check {
        println!(
            "  Last check: {}",
            ht.to_text_en(Accuracy::Rough, Tense::Past).cyan()
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::agents::AgentTool;
    use crate::config::{AiConfig, HyprlayerConfig};

    fn config_with(installed: Option<&str>, pinned: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            ai: Some(AiConfig {
                agent_tool: Some(AgentTool::Claude),
                ..Default::default()
            }),
            agents_installed_version: installed.map(str::to_string),
            agents_pinned_version: pinned.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn status_json_reports_version_fields() {
        let config = config_with(Some("1.5.9"), Some("1.5.9"));
        let ai = config.ai.clone().unwrap();

        let value = AgentTool::Claude.status_json(&ai, &config);

        assert_eq!(value["assetsVersion"], "1.5.9");
        assert_eq!(value["pinnedVersion"], "1.5.9");
        assert_eq!(value["binaryVersion"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn status_json_reports_version_fields_for_every_harness() {
        // The OpenCode arm carries extra provider/model keys; the version
        // triple must be on it too, or the desktop would have to special-
        // case the harness to learn what is installed.
        let config = config_with(Some("1.6.0"), None);
        let ai = config.ai.clone().unwrap();

        for tool in [AgentTool::Claude, AgentTool::Copilot, AgentTool::OpenCode] {
            let value = tool.status_json(&ai, &config);
            let object = value.as_object().expect("status json is an object");
            for key in ["assetsVersion", "pinnedVersion", "binaryVersion"] {
                assert!(object.contains_key(key), "{tool} status json lacks {key}");
            }
            assert_eq!(value["assetsVersion"], "1.6.0");
            assert_eq!(value["pinnedVersion"], serde_json::Value::Null);
        }
    }

    #[test]
    fn status_json_reports_an_unversioned_config_as_null() {
        // A config written before 1.6.0 records a SHA and no version.
        let config = config_with(None, None);
        let ai = config.ai.clone().unwrap();

        let value = AgentTool::Claude.status_json(&ai, &config);

        assert_eq!(value["assetsVersion"], serde_json::Value::Null);
        assert_eq!(value["pinnedVersion"], serde_json::Value::Null);
        assert_eq!(value["binaryVersion"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn status_json_keeps_the_opencode_settings() {
        let mut config = config_with(Some("1.6.0"), None);
        let ai = config.ai_mut();
        ai.agent_tool = Some(AgentTool::OpenCode);
        ai.opencode_sonnet_model = Some("anthropic/claude-sonnet-5".to_string());
        let ai = config.ai.clone().unwrap();

        let value = AgentTool::OpenCode.status_json(&ai, &config);

        assert_eq!(value["opencodeSonnetModel"], "anthropic/claude-sonnet-5");
        assert_eq!(value["assetsVersion"], "1.6.0");
    }
}
