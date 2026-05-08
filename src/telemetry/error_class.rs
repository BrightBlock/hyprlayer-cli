//! Stable, message-text-free error classification for telemetry.
//!
//! Error messages can leak file paths, repo names, or user input; we never
//! send them. The classifier reads the error chain looking for known
//! types and keywords and emits a short stable string. Anything we can't
//! classify becomes `unknown`.

use std::error::Error;

/// Map an error chain to a short stable string. Walks `source()` so an
/// `anyhow::Error` wrapping an `io::Error` still classifies on the inner
/// kind rather than the wrapping prose.
pub fn classify_error(err: &(dyn Error + 'static)) -> String {
    if let Some(io) = find_in_chain::<std::io::Error>(err) {
        return classify_io(io);
    }
    if let Some(json) = find_in_chain::<serde_json::Error>(err) {
        return classify_json(json);
    }
    classify_message(&err.to_string())
}

fn find_in_chain<'a, T: Error + 'static>(err: &'a (dyn Error + 'static)) -> Option<&'a T> {
    let mut cur: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if let Some(t) = e.downcast_ref::<T>() {
            return Some(t);
        }
        cur = e.source();
    }
    None
}

fn classify_io(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "io_not_found",
        PermissionDenied => "io_permission_denied",
        ConnectionRefused => "network_connection_refused",
        ConnectionReset => "network_connection_reset",
        ConnectionAborted => "network_connection_aborted",
        TimedOut => "network_timeout",
        AlreadyExists => "io_already_exists",
        InvalidInput | InvalidData => "io_invalid_data",
        Interrupted => "io_interrupted",
        UnexpectedEof => "io_unexpected_eof",
        WriteZero => "io_write_zero",
        Other => "io_other",
        _ => "io_unknown",
    }
    .to_string()
}

fn classify_json(_e: &serde_json::Error) -> String {
    "json_parse_failed".to_string()
}

/// Last-resort keyword classifier for errors whose chain didn't carry a
/// typed root cause. Matches against lowercased substrings only — never
/// echoes the original message to the spool.
fn classify_message(msg: &str) -> String {
    let lower = msg.to_lowercase();
    let cls = if lower.contains("config") && lower.contains("load") {
        "config_load_failed"
    } else if lower.contains("config") && lower.contains("save") {
        "config_save_failed"
    } else if lower.contains("migrat") {
        "config_migration_failed"
    } else if lower.contains("not authenticated") || lower.contains("not logged in") {
        "github_unauthenticated"
    } else if lower.contains("gh: command not found") || lower.contains("gh not found") {
        "gh_not_installed"
    } else if lower.contains("404") || lower.contains("not found") {
        "github_404"
    } else if lower.contains("rate limit") {
        "github_rate_limited"
    } else if lower.contains("network") {
        "network_error"
    } else if lower.contains("timeout") {
        "network_timeout"
    } else if lower.contains("permission") {
        "io_permission_denied"
    } else if lower.contains("posthog") {
        "posthog_error"
    } else if lower.contains("spool") {
        "spool_io_error"
    } else if lower.contains("cancelled") || lower.contains("aborted") {
        "user_aborted"
    } else if lower.contains("not configured") || lower.contains("run 'hyprlayer thoughts init'") {
        "not_configured"
    } else {
        "unknown"
    };
    cls.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Debug)]
    struct Wrap(io::Error);
    impl std::fmt::Display for Wrap {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "wrapping {}", self.0)
        }
    }
    impl Error for Wrap {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn io_kinds_map_to_stable_strings() {
        let e: io::Error = io::ErrorKind::NotFound.into();
        assert_eq!(classify_error(&e), "io_not_found");
        let e: io::Error = io::ErrorKind::PermissionDenied.into();
        assert_eq!(classify_error(&e), "io_permission_denied");
        let e: io::Error = io::ErrorKind::TimedOut.into();
        assert_eq!(classify_error(&e), "network_timeout");
    }

    #[test]
    fn classifier_walks_the_source_chain() {
        let inner: io::Error = io::ErrorKind::NotFound.into();
        let outer = Wrap(inner);
        assert_eq!(classify_error(&outer), "io_not_found");
    }

    #[test]
    fn keyword_classifier_handles_unknown_errors() {
        let anyhow_err: anyhow::Error = anyhow::anyhow!("config load failed: bad json");
        let cls = classify_error(anyhow_err.as_ref());
        assert_eq!(cls, "config_load_failed");
    }

    #[test]
    fn unknown_messages_get_stable_fallback() {
        let anyhow_err: anyhow::Error = anyhow::anyhow!("something nobody could anticipate");
        assert_eq!(classify_error(anyhow_err.as_ref()), "unknown");
    }

    /// Privacy contract: classifier never echoes the original message back.
    /// Verifies our output set is bounded — anyone reading the spool only
    /// sees stable enums, never user text.
    #[test]
    fn classifier_output_is_in_the_known_set() {
        let known: &[&str] = &[
            "io_not_found",
            "io_permission_denied",
            "io_already_exists",
            "io_invalid_data",
            "io_interrupted",
            "io_unexpected_eof",
            "io_write_zero",
            "io_other",
            "io_unknown",
            "network_connection_refused",
            "network_connection_reset",
            "network_connection_aborted",
            "network_timeout",
            "network_error",
            "json_parse_failed",
            "config_load_failed",
            "config_save_failed",
            "config_migration_failed",
            "github_unauthenticated",
            "gh_not_installed",
            "github_404",
            "github_rate_limited",
            "posthog_error",
            "spool_io_error",
            "user_aborted",
            "not_configured",
            "unknown",
        ];
        for msg in &[
            "secret /home/alice/.ssh/id_rsa leaked",
            "rm -rf / would have happened",
            "user@example.com tried to do X",
        ] {
            let err: anyhow::Error = anyhow::anyhow!("{msg}");
            let cls = classify_error(err.as_ref());
            assert!(
                known.contains(&cls.as_str()),
                "leaked message text via class={cls}"
            );
            assert!(!cls.contains('@'), "class {cls} contained an email char");
            assert!(!cls.contains('/'), "class {cls} contained a path char");
        }
    }
}
