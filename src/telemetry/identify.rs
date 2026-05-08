use crate::config::HyprlayerConfig;

use super::event::Event;
use super::spool;

/// Spool a `$identify` event linking the existing anonymous installation_id
/// to the now-known user_id. Caller has already flipped `mode = Identified`
/// and saved config.
pub fn record_identify(config: &HyprlayerConfig) -> std::io::Result<()> {
    let installation_id = match config.telemetry.installation_id.as_deref() {
        Some(id) => id,
        None => return Ok(()),
    };
    let user_id = config
        .thoughts
        .as_ref()
        .map(|t| t.user.as_str())
        .filter(|u| !u.is_empty());
    let Some(user_id) = user_id else {
        eprintln!(
            "warning: telemetry mode 'identified' set but thoughts.user is empty; falling back to anonymous behavior."
        );
        return Ok(());
    };
    let event = Event::identify(installation_id, user_id, config);
    spool::append(&event)
}

/// Test-only variant that writes to an explicit spool path so unit tests
/// don't have to mutate the shared `HOME`/`XDG_CONFIG_HOME` env vars.
#[cfg(test)]
fn record_identify_at(config: &HyprlayerConfig, path: &std::path::Path) -> std::io::Result<bool> {
    let installation_id = match config.telemetry.installation_id.as_deref() {
        Some(id) => id,
        None => return Ok(false),
    };
    let user_id = config
        .thoughts
        .as_ref()
        .map(|t| t.user.as_str())
        .filter(|u| !u.is_empty());
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    let event = Event::identify(installation_id, user_id, config);
    spool::append_to(path, &event)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TelemetryConfig, TelemetryMode, ThoughtsConfig};

    #[test]
    fn record_identify_writes_one_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spool.jsonl");
        let cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                installation_id: Some("anon-uuid".to_string()),
                ..Default::default()
            },
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let wrote = record_identify_at(&cfg, &path).unwrap();
        assert!(wrote);

        let events = spool::drain_at(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_id.as_deref(), Some("alice"));
        assert_eq!(events[0].installation_id, "anon-uuid");
    }

    #[test]
    fn record_identify_noops_with_empty_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spool.jsonl");
        let cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Identified,
                installation_id: Some("anon-uuid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let wrote = record_identify_at(&cfg, &path).unwrap();
        assert!(!wrote);

        let events = spool::drain_at(&path).unwrap();
        assert!(events.is_empty());
    }
}
