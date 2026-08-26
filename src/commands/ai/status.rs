use anyhow::Result;
use chrono_humanize::{Accuracy, HumanTime, Tense};
use colored::Colorize;
use std::time::{Duration, UNIX_EPOCH};

use crate::cli::AiStatusArgs;
use crate::config::HyprlayerConfig;

pub fn status(args: AiStatusArgs) -> Result<()> {
    let AiStatusArgs { json, config } = args;
    let config_path = config.path()?;

    let hyprlayer_config = config.load_if_exists()?.unwrap_or_default();

    if json {
        let mut value = crate::agents::bundle_set_status_json(&hyprlayer_config);
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

    crate::agents::print_bundle_set_status(&hyprlayer_config);
    print_bundle_freshness(&hyprlayer_config);

    println!();
    println!(
        "  Config file: {}",
        config_path.display().to_string().bright_black()
    );

    Ok(())
}

/// Render which assets bundle is installed, whether it is pinned, and the
/// legacy SHA + last-check timestamp, under the paired-platform status block.
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
    use crate::config::HyprlayerConfig;

    fn config_with(installed: Option<&str>, pinned: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            agents_installed_version: installed.map(str::to_string),
            agents_pinned_version: pinned.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn status_json_reports_version_fields() {
        let config = config_with(Some("1.5.9"), Some("1.5.9"));
        let value = crate::agents::bundle_set_status_json(&config);

        assert_eq!(value["agentTool"], "Claude + Codex");
        assert_eq!(value["assetsVersion"], "1.5.9");
        assert_eq!(value["pinnedVersion"], "1.5.9");
        assert_eq!(value["binaryVersion"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn status_json_reports_both_platforms() {
        let config = config_with(Some("1.6.0"), None);
        let value = crate::agents::bundle_set_status_json(&config);
        let platforms = value["platforms"].as_array().unwrap();
        assert_eq!(platforms.len(), 2);
        assert_eq!(platforms[0]["id"], "claude");
        assert_eq!(platforms[1]["id"], "codex");
        assert_eq!(value["assetsVersion"], "1.6.0");
        assert_eq!(value["pinnedVersion"], serde_json::Value::Null);
    }

    #[test]
    fn status_json_reports_an_unversioned_config_as_null() {
        // A config written before 1.6.0 records a SHA and no version.
        let config = config_with(None, None);
        let value = crate::agents::bundle_set_status_json(&config);

        assert_eq!(value["assetsVersion"], serde_json::Value::Null);
        assert_eq!(value["pinnedVersion"], serde_json::Value::Null);
        assert_eq!(value["binaryVersion"], env!("CARGO_PKG_VERSION"));
    }
}
