pub mod configure;
pub mod reinstall;
pub mod status;

use anyhow::Result;
use std::path::Path;

use crate::agents::AgentTool;
use crate::commands::telemetry::hook as telemetry_hook;
use crate::config::HyprlayerConfig;
use crate::telemetry;

/// Persist the SHA after a successful `AgentTool::install` and clear
/// `last_agent_check` so the next startup-time check re-evaluates
/// immediately instead of waiting for the throttle window.
///
/// `sha = None` (commits API was unreachable) leaves the cached SHA
/// untouched but still clears the throttle, so the next startup check
/// will retry the SHA fetch.
pub(crate) fn record_install(
    config: &mut HyprlayerConfig,
    config_path: &Path,
    sha: Option<String>,
) -> Result<()> {
    let sha_for_event = sha.clone().or_else(|| config.agents_installed_sha.clone());
    if sha.is_some() {
        config.agents_installed_sha = sha;
    }
    config.last_agent_check = None;
    config.save(config_path)?;

    spool_install_event(config, sha_for_event.as_deref());
    Ok(())
}

/// Build the install event payload if telemetry is enabled and an
/// `agent_tool` is configured. Pure — no I/O side effects, so it's
/// directly unit-testable. The spool append happens at the call site.
fn build_install_event_if_recording(
    config: &HyprlayerConfig,
    sha: Option<&str>,
) -> Option<telemetry::event::Event> {
    if !config.telemetry.is_recording() {
        return None;
    }
    let tool = config.ai.as_ref().and_then(|ai| ai.agent_tool.as_ref())?;
    Some(telemetry::event::Event::install(
        telemetry::event::Harness::from(tool),
        sha.unwrap_or(""),
        config,
    ))
}

/// Best-effort `install` event. Silent on every failure path. Skipped
/// when telemetry is disabled or no agent_tool is configured (we only
/// emit events for actual installs, not partial saves).
fn spool_install_event(config: &HyprlayerConfig, sha: Option<&str>) {
    if let Some(event) = build_install_event_if_recording(config, sha) {
        let _ = telemetry::spool::append(&event);
    }
}

/// Decide whether the Stop hook should be present in
/// `~/.claude/settings.json`. The hook only earns its keep when the
/// user is on Claude *and* opted into telemetry — otherwise every Stop
/// event spawns `hyprlayer telemetry record-from-hook` just to read
/// stdin and bail on `is_recording()`.
fn should_install_hook(agent: AgentTool, config: &HyprlayerConfig) -> bool {
    agent == AgentTool::Claude && config.telemetry.is_recording()
}

/// On a Claude install with telemetry enabled, write the Stop-hook
/// entry to `~/.claude/settings.json`; otherwise scrub any prior hook
/// entry so it doesn't keep firing (covers switching to a non-Claude
/// harness, or staying on Claude with telemetry off). Failure is
/// non-fatal (one-line stderr warning).
pub(crate) fn install_claude_hook_if_applicable(agent: AgentTool, config: &HyprlayerConfig) {
    let Ok(path) = telemetry_hook::settings_path() else {
        return;
    };
    let result = if should_install_hook(agent, config) {
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
    use crate::agents::AgentTool;
    use crate::config::{AiConfig, TelemetryConfig, TelemetryMode};
    use crate::telemetry::event::EventType;
    use std::fs;

    fn cfg_recording_with_claude() -> HyprlayerConfig {
        HyprlayerConfig {
            ai: Some(AiConfig {
                agent_tool: Some(AgentTool::Claude),
                ..Default::default()
            }),
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("install-uuid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn should_install_hook_only_when_claude_and_recording() {
        let cfg = cfg_recording_with_claude();
        assert!(should_install_hook(AgentTool::Claude, &cfg));
    }

    #[test]
    fn should_not_install_hook_when_telemetry_off() {
        let mut cfg = cfg_recording_with_claude();
        cfg.telemetry.mode = TelemetryMode::Off;
        assert!(!should_install_hook(AgentTool::Claude, &cfg));
    }

    #[test]
    fn should_not_install_hook_for_non_claude_agent() {
        let cfg = cfg_recording_with_claude();
        assert!(!should_install_hook(AgentTool::OpenCode, &cfg));
        assert!(!should_install_hook(AgentTool::Copilot, &cfg));
    }

    #[test]
    fn should_not_install_hook_without_installation_id() {
        let mut cfg = cfg_recording_with_claude();
        cfg.telemetry.installation_id = None;
        assert!(!should_install_hook(AgentTool::Claude, &cfg));
    }

    #[test]
    fn install_event_built_when_recording_and_agent_tool_set() {
        let cfg = cfg_recording_with_claude();
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
        let mut cfg = cfg_recording_with_claude();
        cfg.telemetry.mode = TelemetryMode::Off;
        assert!(build_install_event_if_recording(&cfg, Some("abc")).is_none());
    }

    #[test]
    fn install_event_skipped_when_no_agent_tool() {
        let mut cfg = cfg_recording_with_claude();
        cfg.ai = None;
        assert!(build_install_event_if_recording(&cfg, Some("abc")).is_none());
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
