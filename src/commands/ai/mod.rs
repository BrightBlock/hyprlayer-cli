pub mod reinstall;
pub mod status;
pub mod versions;

use anyhow::Result;
use std::path::Path;

use crate::commands::telemetry::hook as telemetry_hook;
use crate::config::HyprlayerConfig;
use crate::telemetry;

/// Persist what a successful Claude + Codex bundle-set install produced: the SHA, and the
/// assets version the install resolved to.
///
/// Recording the version is what stops the startup auto-refresh from
/// immediately reinstalling on top of an explicit `ai reinstall` — that
/// check compares exactly this field against
/// `desired_assets_version`. `record_assets_version` also clears
/// `last_agent_check`, the failed-refresh backoff.
///
/// `sha = None` (the normal release-bundle path) leaves a cached legacy SHA
/// untouched; the install itself still happened, so the version is recorded.
pub(crate) fn record_install(
    config: &mut HyprlayerConfig,
    config_path: &Path,
    sha: Option<String>,
) -> Result<()> {
    let sha_for_event = sha.clone().or_else(|| config.agents_installed_sha.clone());
    if sha.is_some() {
        config.agents_installed_sha = sha;
    }
    let version = config.desired_assets_version().to_string();
    config.record_assets_version(&version);
    config.save(config_path)?;

    spool_install_event(config, sha_for_event.as_deref());
    Ok(())
}

/// Build the pair-install event payload when telemetry is enabled. Pure —
/// no I/O side effects, so it's directly unit-testable. The spool append
/// happens at the call site.
fn build_install_event_if_recording(
    config: &HyprlayerConfig,
    sha: Option<&str>,
) -> Option<telemetry::event::Event> {
    if !config.telemetry.is_recording() {
        return None;
    }
    Some(telemetry::event::Event::install(
        telemetry::event::Harness::ClaudeCodex,
        sha.unwrap_or(""),
        config,
    ))
}

/// Best-effort `install` event. Silent on every failure path. Skipped
/// when telemetry is disabled (we only emit events for actual installs,
/// not partial saves).
fn spool_install_event(config: &HyprlayerConfig, sha: Option<&str>) {
    if let Some(event) = build_install_event_if_recording(config, sha) {
        let _ = telemetry::spool::append(&event);
    }
}

/// Decide whether the Claude Stop hook should be present. Claude is always
/// one of the two supported base platforms, so only telemetry policy matters.
fn should_install_hook(config: &HyprlayerConfig) -> bool {
    config.telemetry.is_recording()
}

/// With telemetry enabled, write the Stop-hook entry to
/// `~/.claude/settings.json`; otherwise scrub a prior entry. Failure is
/// non-fatal (one-line stderr warning).
pub(crate) fn install_claude_hook_if_applicable(config: &HyprlayerConfig) {
    let Ok(path) = telemetry_hook::settings_path() else {
        return;
    };
    let result = if should_install_hook(config) {
        telemetry_hook::install_at(&path)
    } else {
        telemetry_hook::uninstall_at(&path)
    };
    if let Err(e) = result {
        eprintln!(
            "warning: could not update Claude Code Stop hook at {}: {}",
            path.display(),
            e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TelemetryConfig, TelemetryMode};
    use crate::telemetry::event::EventType;
    use std::fs;

    fn cfg_recording() -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("install-uuid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn should_install_hook_when_recording() {
        let cfg = cfg_recording();
        assert!(should_install_hook(&cfg));
    }

    #[test]
    fn should_not_install_hook_when_telemetry_off() {
        let mut cfg = cfg_recording();
        cfg.telemetry.mode = TelemetryMode::Off;
        assert!(!should_install_hook(&cfg));
    }

    #[test]
    fn should_not_install_hook_without_installation_id() {
        let mut cfg = cfg_recording();
        cfg.telemetry.installation_id = None;
        assert!(!should_install_hook(&cfg));
    }

    #[test]
    fn install_event_built_when_recording_and_agent_tool_set() {
        let cfg = cfg_recording();
        let event = build_install_event_if_recording(&cfg, Some("abc123"))
            .expect("install event should be built when recording");
        assert_eq!(event.event_type, EventType::Install);
        assert_eq!(
            event.extra.get("sha").and_then(|v| v.as_str()),
            Some("abc123")
        );
    }

    #[test]
    fn install_event_skipped_when_telemetry_off() {
        let mut cfg = cfg_recording();
        cfg.telemetry.mode = TelemetryMode::Off;
        assert!(build_install_event_if_recording(&cfg, Some("abc")).is_none());
    }

    #[test]
    fn install_event_is_the_combined_pair() {
        let cfg = cfg_recording();
        let event = build_install_event_if_recording(&cfg, Some("abc")).unwrap();
        assert_eq!(event.harness, telemetry::event::Harness::ClaudeCodex);
    }

    #[test]
    fn record_install_persists_sha_and_clears_throttle() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_record_install_test");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("config.json");

        let mut cfg = HyprlayerConfig {
            agents_installed_sha: Some("old".to_string()),
            last_agent_check: Some(1_700_000_000),
            ..Default::default()
        };
        cfg.save(&config_path).unwrap();

        record_install(&mut cfg, &config_path, Some("new".to_string())).unwrap();

        assert_eq!(cfg.agents_installed_sha.as_deref(), Some("new"));
        assert!(cfg.last_agent_check.is_none());

        let reloaded = HyprlayerConfig::load(&config_path).unwrap();
        assert_eq!(reloaded.agents_installed_sha.as_deref(), Some("new"));
        assert!(reloaded.last_agent_check.is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn record_install_with_none_keeps_existing_sha() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_record_install_none_test");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("config.json");

        let mut cfg = HyprlayerConfig {
            agents_installed_sha: Some("existing".to_string()),
            last_agent_check: Some(1_700_000_000),
            ..Default::default()
        };
        cfg.save(&config_path).unwrap();

        record_install(&mut cfg, &config_path, None).unwrap();

        assert_eq!(cfg.agents_installed_sha.as_deref(), Some("existing"));
        assert!(cfg.last_agent_check.is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }
}
