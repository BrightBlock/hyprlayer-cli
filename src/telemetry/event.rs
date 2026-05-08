use serde::{Deserialize, Serialize};

use crate::agents::AgentTool;
use crate::config::{HyprlayerConfig, TelemetryMode};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    CliCommand,
    SkillRun,
    Install,
    Error,
    /// PostHog `$identify` event for anonymous→identified merges. Carried in
    /// the spool as the same Event shape; the capture serializer rewrites
    /// the wire-level event name to `$identify`.
    Identify,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::CliCommand => "cli_command",
            EventType::SkillRun => "skill_run",
            EventType::Install => "install",
            EventType::Error => "error",
            EventType::Identify => "$identify",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Success,
    Failure,
    Aborted,
}

impl std::str::FromStr for Outcome {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(Outcome::Success),
            "failure" => Ok(Outcome::Failure),
            "aborted" => Ok(Outcome::Aborted),
            other => Err(format!("unknown outcome: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    Claude,
    Copilot,
    Opencode,
    #[default]
    Unknown,
}

impl From<&AgentTool> for Harness {
    fn from(tool: &AgentTool) -> Self {
        match tool {
            AgentTool::Claude => Harness::Claude,
            AgentTool::Copilot => Harness::Copilot,
            AgentTool::OpenCode => Harness::Opencode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub schema_version: u32,
    pub event_type: EventType,
    pub event_timestamp: String,
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub hyprlayer_version: String,
    pub harness: Harness,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extra: serde_json::Value,
}

impl Event {
    fn base(event_type: EventType, config: &HyprlayerConfig) -> Self {
        let installation_id = config.telemetry.installation_id.clone().unwrap_or_default();
        let user_id = match config.telemetry.mode {
            TelemetryMode::Identified => config
                .thoughts
                .as_ref()
                .map(|t| t.user.clone())
                .filter(|u| !u.is_empty()),
            _ => None,
        };
        let harness = config
            .ai
            .as_ref()
            .and_then(|a| a.agent_tool.as_ref())
            .map(Harness::from)
            .unwrap_or_default();

        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            schema_version: SCHEMA_VERSION,
            event_type,
            event_timestamp: super::rfc3339_now(),
            installation_id,
            user_id,
            org_id: config.telemetry.org_id.clone(),
            session_id: None,
            hyprlayer_version: env!("CARGO_PKG_VERSION").to_string(),
            harness,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            command: None,
            skill: None,
            duration_ms: None,
            outcome: None,
            error_class: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            extra: serde_json::Value::Null,
        }
    }

    pub fn cli_command(
        command: &str,
        duration_ms: u64,
        outcome: Outcome,
        error_class: Option<String>,
        config: &HyprlayerConfig,
    ) -> Self {
        let mut e = Self::base(EventType::CliCommand, config);
        e.command = Some(command.to_string());
        e.duration_ms = Some(duration_ms);
        e.outcome = Some(outcome);
        e.error_class = error_class;
        e
    }

    pub fn skill_run(
        skill: &str,
        session_id: Option<String>,
        duration_ms: Option<u64>,
        outcome: Outcome,
        error_class: Option<String>,
        config: &HyprlayerConfig,
    ) -> Self {
        let mut e = Self::base(EventType::SkillRun, config);
        e.skill = Some(skill.to_string());
        e.session_id = session_id;
        e.duration_ms = duration_ms;
        e.outcome = Some(outcome);
        e.error_class = error_class;
        e
    }

    #[allow(dead_code)]
    pub fn install(harness: Harness, sha: &str, config: &HyprlayerConfig) -> Self {
        let mut e = Self::base(EventType::Install, config);
        e.harness = harness;
        e.extra = serde_json::json!({ "sha": sha });
        e
    }

    #[allow(dead_code)]
    pub fn error(error_class: &str, command: Option<String>, config: &HyprlayerConfig) -> Self {
        let mut e = Self::base(EventType::Error, config);
        e.error_class = Some(error_class.to_string());
        e.command = command;
        e.outcome = Some(Outcome::Failure);
        e
    }

    /// PostHog `$identify` — links the existing anonymous installation_id
    /// to a now-known user_id. Emitted once on the anonymous→identified
    /// transition.
    pub fn identify(installation_id: &str, user_id: &str, config: &HyprlayerConfig) -> Self {
        let mut e = Self::base(EventType::Identify, config);
        e.installation_id = installation_id.to_string();
        e.user_id = Some(user_id.to_string());
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TelemetryConfig;

    fn config_with_mode(mode: TelemetryMode) -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode,
                installation_id: Some("install-uuid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn cli_command_event_carries_command_and_duration() {
        let cfg = config_with_mode(TelemetryMode::Anonymous);
        let e = Event::cli_command("ai.status", 42, Outcome::Success, None, &cfg);
        assert_eq!(e.command.as_deref(), Some("ai.status"));
        assert_eq!(e.duration_ms, Some(42));
        assert_eq!(e.outcome, Some(Outcome::Success));
        assert_eq!(e.installation_id, "install-uuid");
        assert!(e.user_id.is_none());
    }

    #[test]
    fn identified_mode_populates_user_id() {
        let mut cfg = config_with_mode(TelemetryMode::Identified);
        cfg.thoughts = Some(crate::config::ThoughtsConfig {
            user: "alice".to_string(),
            ..Default::default()
        });
        let e = Event::cli_command("ai.status", 0, Outcome::Success, None, &cfg);
        assert_eq!(e.user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn anonymous_mode_strips_user_id() {
        let mut cfg = config_with_mode(TelemetryMode::Anonymous);
        cfg.thoughts = Some(crate::config::ThoughtsConfig {
            user: "alice".to_string(),
            ..Default::default()
        });
        let e = Event::cli_command("ai.status", 0, Outcome::Success, None, &cfg);
        assert!(e.user_id.is_none());
    }

    #[test]
    fn event_type_string_repr_is_stable() {
        assert_eq!(EventType::CliCommand.as_str(), "cli_command");
        assert_eq!(EventType::SkillRun.as_str(), "skill_run");
        assert_eq!(EventType::Install.as_str(), "install");
        assert_eq!(EventType::Error.as_str(), "error");
        assert_eq!(EventType::Identify.as_str(), "$identify");
    }

    #[test]
    fn outcome_from_str_round_trip() {
        use std::str::FromStr;
        assert_eq!(Outcome::from_str("success").unwrap(), Outcome::Success);
        assert_eq!(Outcome::from_str("failure").unwrap(), Outcome::Failure);
        assert_eq!(Outcome::from_str("aborted").unwrap(), Outcome::Aborted);
        assert!(Outcome::from_str("nope").is_err());
    }
}
