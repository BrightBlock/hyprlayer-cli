use super::DEFAULT_API_KEY;
use super::event::Event;

/// When sending to the hardcoded community key, strip every field that
/// could correlate one community deployment to another. The org-key path
/// keeps these fields — the org has accountability for its own users.
pub fn redact_for_default_key(event: &mut Event, api_key: &str) {
    if api_key == DEFAULT_API_KEY {
        event.user_id = None;
        event.org_id = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HyprlayerConfig, TelemetryConfig, TelemetryMode};
    use crate::telemetry::event::Outcome;

    fn cfg(mode: TelemetryMode) -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode,
                installation_id: Some("anon".to_string()),
                org_id: Some("acme".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn default_key_strips_user_and_org() {
        let mut e = Event::cli_command(
            "ai.status",
            0,
            Outcome::Success,
            None,
            &cfg(TelemetryMode::Identified),
        );
        e.user_id = Some("alice".to_string());

        redact_for_default_key(&mut e, DEFAULT_API_KEY);

        assert!(e.user_id.is_none());
        assert!(e.org_id.is_none());
        assert_eq!(e.installation_id, "anon");
    }

    #[test]
    fn non_default_key_preserves_fields() {
        let mut e = Event::cli_command(
            "ai.status",
            0,
            Outcome::Success,
            None,
            &cfg(TelemetryMode::Identified),
        );
        e.user_id = Some("alice".to_string());

        redact_for_default_key(&mut e, "phc_org_key");

        assert_eq!(e.user_id.as_deref(), Some("alice"));
        assert_eq!(e.org_id.as_deref(), Some("acme"));
    }
}
