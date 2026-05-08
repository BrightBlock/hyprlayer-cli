//! `hyprlayer telemetry skill-start` — emit a session token to stdout.
//!
//! Pure-output subcommand. No event is spooled; no filesystem state is
//! created; no network call is made. The session token is the function's
//! return value, like `mktemp` printing its path. The matching
//! `skill-end` call parses the embedded timestamp to compute duration.
//!
//! Format: `<unix_ms>-<8 hex chars>` — printable ASCII, identical bytes
//! across bash / cmd / PowerShell, splittable on the first `-`. The
//! random suffix avoids collisions across parallel skill runs.

use anyhow::Result;
use std::fmt::Write;

use crate::cli::TelemetrySkillStartArgs;

pub fn skill_start(_args: TelemetrySkillStartArgs) -> Result<()> {
    let token = generate_session_token();
    println!("{token}");
    Ok(())
}

pub(crate) fn generate_session_token() -> String {
    let now_ms = system_time_ms();
    let mut rand = [0u8; 4];
    let _ = getrandom::getrandom(&mut rand);
    let mut hex = String::with_capacity(8);
    for b in rand {
        let _ = write!(&mut hex, "{b:02x}");
    }
    format!("{now_ms}-{hex}")
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

    #[test]
    fn token_format_is_ms_dash_8hex() {
        let t = generate_session_token();
        let (head, tail) = t.split_once('-').expect("must contain `-`");
        let _ms: u128 = head.parse().expect("head must parse as integer ms");
        assert_eq!(tail.len(), 8, "tail must be 8 hex chars: `{t}`");
        assert!(
            tail.chars().all(|c| c.is_ascii_hexdigit()),
            "tail must be hex: `{t}`"
        );
    }

    #[test]
    fn tokens_are_unique_across_calls() {
        // Within a single millisecond, the random suffix must vary.
        let a = generate_session_token();
        let b = generate_session_token();
        assert_ne!(a, b, "back-to-back tokens collided: a=`{a}` b=`{b}`");
    }

    #[test]
    fn token_uses_only_shell_safe_ascii() {
        let t = generate_session_token();
        // No quoting, escaping, or character-class surprises in any shell.
        assert!(
            t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "token must be `[0-9a-z-]+`: `{t}`"
        );
    }
}
