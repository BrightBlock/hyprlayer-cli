use serde_json::{Value, json};

use super::event::{Event, EventType};

/// Build a PostHog Capture batch payload from a slice of events.
pub fn build_capture_payload(events: &[Event], api_key: &str) -> Value {
    let batch: Vec<Value> = events.iter().map(event_to_capture).collect();
    json!({ "api_key": api_key, "batch": batch })
}

fn event_to_capture(event: &Event) -> Value {
    let distinct_id = event.user_id.as_deref().unwrap_or(&event.installation_id);

    let mut props = json!({
        "$lib": "hyprlayer-cli",
        "$lib_version": event.hyprlayer_version,
        "$geoip_disable": true,
        "$ip": "0.0.0.0",
        "schema_version": event.schema_version,
        "harness": event.harness,
        "os": event.os,
        "arch": event.arch,
        "session_id": event.session_id,
        "command": event.command,
        "skill": event.skill,
        "duration_ms": event.duration_ms,
        "outcome": event.outcome,
        "error_class": event.error_class,
    });

    if let Some(v) = event.input_tokens {
        props["input_tokens"] = json!(v);
    }
    if let Some(v) = event.output_tokens {
        props["output_tokens"] = json!(v);
    }
    if let Some(v) = event.cache_read_tokens {
        props["cache_read_tokens"] = json!(v);
    }
    if let Some(v) = event.cache_creation_tokens {
        props["cache_creation_tokens"] = json!(v);
    }
    if event.event_type == EventType::Identify {
        props["$anon_distinct_id"] = json!(event.installation_id);
    }
    if !event.extra.is_null() {
        props["extra"] = event.extra.clone();
    }
    if let Some(org) = &event.org_id {
        props["$groups"] = json!({ "org": org });
    }

    json!({
        "event": event.event_type.as_str(),
        "distinct_id": distinct_id,
        "properties": props,
        "timestamp": event.event_timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HyprlayerConfig, TelemetryConfig, TelemetryMode};
    use crate::telemetry::event::Outcome;

    fn anonymous_config() -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                installation_id: Some("anon-uuid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn capture_payload_has_api_key_and_batch() {
        let cfg = anonymous_config();
        let ev = Event::cli_command("ai.status", 5, Outcome::Success, None, &cfg);
        let payload = build_capture_payload(&[ev], "phc_test");
        assert_eq!(payload["api_key"], "phc_test");
        assert!(payload["batch"].is_array());
        assert_eq!(payload["batch"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn cli_command_uses_installation_id_as_distinct_id_in_anonymous_mode() {
        let cfg = anonymous_config();
        let ev = Event::cli_command("ai.status", 0, Outcome::Success, None, &cfg);
        let payload = build_capture_payload(&[ev], "phc_test");
        assert_eq!(payload["batch"][0]["distinct_id"], "anon-uuid");
        assert_eq!(payload["batch"][0]["event"], "cli_command");
        assert_eq!(payload["batch"][0]["properties"]["$lib"], "hyprlayer-cli");
        assert_eq!(payload["batch"][0]["properties"]["command"], "ai.status");
    }

    #[test]
    fn identify_event_carries_anon_distinct_id() {
        let cfg = anonymous_config();
        let ev = Event::identify("anon-uuid", "alice", &cfg);
        let payload = build_capture_payload(&[ev], "phc_test");
        assert_eq!(payload["batch"][0]["event"], "$identify");
        assert_eq!(payload["batch"][0]["distinct_id"], "alice");
        assert_eq!(
            payload["batch"][0]["properties"]["$anon_distinct_id"],
            "anon-uuid"
        );
    }

    #[test]
    fn org_id_emits_groups_property() {
        let mut cfg = anonymous_config();
        cfg.telemetry.org_id = Some("acme".to_string());
        let ev = Event::cli_command("ai.status", 0, Outcome::Success, None, &cfg);
        let payload = build_capture_payload(&[ev], "phc_test");
        assert_eq!(payload["batch"][0]["properties"]["$groups"]["org"], "acme");
    }
}
