//! `hyprlayer telemetry skill-end` — emit one `skill_run` event with
//! duration computed from the session token's embedded start timestamp.
//!
//! Silent on every expected failure path: opted-out, missing config,
//! malformed session token, spool I/O failure. A genuinely bogus token
//! still produces an event — just without `duration_ms` populated —
//! so we never lose count of skill invocations to a parsing edge case.

use anyhow::Result;

use crate::cli::TelemetrySkillEndArgs;
use crate::telemetry::event::{Event, Outcome};
use crate::telemetry::spool;

pub fn skill_end(args: TelemetrySkillEndArgs) -> Result<()> {
    let TelemetrySkillEndArgs {
        skill,
        session,
        outcome,
        error_class,
        config,
    } = args;

    let Ok(Some(cfg)) = config.load_if_exists() else {
        return Ok(());
    };
    if !cfg.telemetry.is_recording() {
        return Ok(());
    }

    let session = session.trim().to_string();
    let duration_ms = parse_session_duration_ms(&session);
    let outcome = parse_outcome(outcome.as_deref());

    let event = Event::skill_run(
        skill.as_str(),
        Some(session),
        duration_ms,
        outcome,
        error_class,
        &cfg,
    );
    let _ = spool::append(&event);
    Ok(())
}

/// Parse `<unix_ms>-<random>` and compute `now_ms - start_ms`. Returns
/// `None` for malformed tokens (caller emits the event without
/// `duration_ms`) and saturates at `u64::MAX` for absurd deltas.
pub(crate) fn parse_session_duration_ms(session: &str) -> Option<u64> {
    let (head, _tail) = session.split_once('-')?;
    let start_ms: u128 = head.parse().ok()?;
    let now_ms = system_time_ms();
    let delta = now_ms.saturating_sub(start_ms);
    Some(u64::try_from(delta).unwrap_or(u64::MAX))
}

fn parse_outcome(s: Option<&str>) -> Outcome {
    use std::str::FromStr;
    s.and_then(|s| Outcome::from_str(s).ok())
        .unwrap_or_default()
}

fn system_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip with a token created "just now": duration is small
    /// (well under 5 seconds for a tight test loop).
    #[test]
    fn duration_is_small_when_session_is_recent() {
        let token = super::super::skill_start::generate_session_token();
        let d = parse_session_duration_ms(&token).expect("recent token must parse");
        assert!(
            d < 5_000,
            "fresh token produced unrealistic duration: {d}ms"
        );
    }

    #[test]
    fn malformed_session_returns_none() {
        // Tokens whose head can't parse as u128 → None. Caller emits
        // the event without `duration_ms` rather than crashing.
        assert_eq!(parse_session_duration_ms("not-a-real-token"), None);
        assert_eq!(parse_session_duration_ms(""), None);
        assert_eq!(parse_session_duration_ms("nodash"), None);
        assert_eq!(parse_session_duration_ms("xx-yy"), None);
    }

    #[test]
    fn future_timestamp_saturates_to_zero_not_negative() {
        // Token claims to start in the year 3000 — saturating_sub
        // floors at 0 instead of underflowing.
        let far_future = "32503680000000-deadbeef"; // ~year 3000 in ms
        let d = parse_session_duration_ms(far_future).expect("must parse");
        assert_eq!(d, 0);
    }

    #[test]
    fn whitespace_in_token_is_stripped() {
        let token = super::super::skill_start::generate_session_token();
        let padded = format!("  {token}\n");
        // The CLI handler trims before parsing — reproduce that here.
        let trimmed = padded.trim();
        assert!(parse_session_duration_ms(trimmed).is_some());
    }
}
