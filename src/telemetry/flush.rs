use std::time::Duration;

use crate::config::{HyprlayerConfig, KeySource, TelemetryMode};
use crate::http;

use super::{DEFAULT_API_KEY, posthog, privacy, spool};

/// PostHog Capture API host. The same `/i/v0/e/` path works for the
/// community us.i.posthog.com cloud and any future self-hosted variant
/// that ships its own transport, so this is the only endpoint string
/// we keep around.
const POSTHOG_ENDPOINT: &str = "https://us.i.posthog.com";

#[derive(Debug)]
pub enum FlushError {
    Io(std::io::Error),
    /// HTTP transport error. `dropped` events failed to land back on disk
    /// after the failure and have been lost.
    Http {
        source: http::HttpError,
        dropped: usize,
    },
    /// PostHog returned a non-2xx status. `dropped` is as above.
    PostHogReturned {
        status: u16,
        dropped: usize,
    },
}

impl std::fmt::Display for FlushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlushError::Io(e) => write!(f, "io: {e}"),
            FlushError::Http { source, dropped: 0 } => write!(f, "http: {source}"),
            FlushError::Http { source, dropped } => {
                write!(f, "http: {source} ({dropped} event(s) lost on re-spool)")
            }
            FlushError::PostHogReturned { status, dropped: 0 } => {
                write!(f, "posthog returned {status}")
            }
            FlushError::PostHogReturned { status, dropped } => write!(
                f,
                "posthog returned {status} ({dropped} event(s) lost on re-spool)"
            ),
        }
    }
}

impl std::error::Error for FlushError {}

impl From<std::io::Error> for FlushError {
    fn from(e: std::io::Error) -> Self {
        FlushError::Io(e)
    }
}

/// Resolve the API key without copying. Returns a borrow of the configured
/// override or the static community key.
pub fn resolve_api_key(config: &HyprlayerConfig) -> &str {
    match config.telemetry.api_key_source {
        KeySource::Manual | KeySource::Github => config
            .telemetry
            .api_key
            .as_deref()
            .unwrap_or(DEFAULT_API_KEY),
        KeySource::Default => DEFAULT_API_KEY,
    }
}

/// Drain the spool, redact for the default key if applicable, and POST a
/// single batch to PostHog Capture. On HTTP failure, re-spool the events
/// for a future retry; if the re-spool itself fails for some events, the
/// returned error reports how many were lost so the caller can surface
/// it (the previous version silently dropped them).
pub fn flush(config: &HyprlayerConfig) -> Result<usize, FlushError> {
    if config.telemetry.mode == TelemetryMode::Off {
        return Ok(0);
    }
    let api_key = resolve_api_key(config);
    let mut events = spool::drain()?;
    if events.is_empty() {
        return Ok(0);
    }

    // Redact in place. `redact_for_default_key` is a no-op when the key
    // isn't the community default, so the redacted batch is safe to
    // re-spool on failure regardless of which key resolved.
    for ev in &mut events {
        privacy::redact_for_default_key(ev, api_key);
    }
    let count = events.len();
    let payload = posthog::build_capture_payload(&events, api_key);

    let url = format!("{POSTHOG_ENDPOINT}/i/v0/e/");
    match http::post_json_no_response(&url, &payload, Duration::from_secs(10)) {
        Ok(status) if (200..300).contains(&status) => Ok(count),
        Ok(status) => Err(FlushError::PostHogReturned {
            status,
            dropped: respool(events),
        }),
        Err(e) => Err(FlushError::Http {
            source: e,
            dropped: respool(events),
        }),
    }
}

/// Push a failed batch back into the spool one event at a time. Returns
/// the number that failed to land — the caller surfaces this so silent
/// disk-full / permission-denied / rotate-failed conditions don't disappear
/// into the log.
fn respool(events: Vec<crate::telemetry::event::Event>) -> usize {
    events
        .into_iter()
        .filter(|ev| spool::append(ev).is_err())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TelemetryConfig;

    fn config(source: KeySource, api_key: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Anonymous,
                api_key: api_key.map(String::from),
                api_key_source: source,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn resolve_default_uses_hardcoded_key_even_with_stale_override() {
        assert_eq!(
            resolve_api_key(&config(KeySource::Default, None)),
            DEFAULT_API_KEY
        );
        assert_eq!(
            resolve_api_key(&config(KeySource::Default, Some("phc_stale"))),
            DEFAULT_API_KEY,
            "Default source must ignore a leftover api_key field"
        );
    }

    #[test]
    fn resolve_github_returns_org_key_or_falls_back() {
        assert_eq!(
            resolve_api_key(&config(KeySource::Github, Some("phc_org"))),
            "phc_org"
        );
        assert_eq!(
            resolve_api_key(&config(KeySource::Github, None)),
            DEFAULT_API_KEY,
            "Github source with cleared key falls back to community default"
        );
    }

    #[test]
    fn resolve_manual_returns_manual_key() {
        let cfg = config(KeySource::Manual, Some("phc_manual"));
        assert_eq!(resolve_api_key(&cfg), "phc_manual");
    }

    #[test]
    fn off_mode_short_circuits() {
        let cfg = HyprlayerConfig {
            telemetry: TelemetryConfig {
                mode: TelemetryMode::Off,
                ..Default::default()
            },
            ..Default::default()
        };
        let count = flush(&cfg).unwrap();
        assert_eq!(count, 0);
    }
}
