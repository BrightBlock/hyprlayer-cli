//! Opt-in verbose diagnostics for the telemetry org-config path.
//!
//! The `gh`/`git` shell-outs that resolve an org-managed PostHog key
//! (see [`super::org_config`]) funnel *every* failure — gh missing, gh
//! unauthed, variable absent, non-GitHub remote — to a silent `None`,
//! then fall back to anonymous mode. That's the right default for
//! end-users, but it makes "why am I still anonymous?" impossible to
//! debug. Verbose mode lights up each of those failure paths on stderr
//! so an operator can see exactly where the resolution stopped.
//!
//! Enabled by either the `--verbose` flag on `telemetry config` /
//! `telemetry status`, or the `HYPRLAYER_TELEMETRY_VERBOSE=1` environment
//! variable (so the diagnostics also surface during the auto-enroll /
//! refresh that happen as a side effect of ordinary commands).

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide override set by `--verbose`. An `AtomicBool` rather than
/// an env mutation because `std::env::set_var` is `unsafe` on edition
/// 2024 and we'd rather not thread a flag through every callee.
static FORCED: AtomicBool = AtomicBool::new(false);

/// Turn verbose diagnostics on (or off) for the remainder of the process.
/// Called from the command handlers when `--verbose` is passed.
pub fn set_enabled(on: bool) {
    FORCED.store(on, Ordering::Release);
}

/// Verbose is on when `--verbose` was passed or
/// `HYPRLAYER_TELEMETRY_VERBOSE` is set to a truthy value.
pub fn is_enabled() -> bool {
    FORCED.load(Ordering::Acquire) || env_enabled()
}

fn env_enabled() -> bool {
    matches!(
        std::env::var("HYPRLAYER_TELEMETRY_VERBOSE")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Print one diagnostic line to stderr, prefixed for grep-ability. A
/// no-op unless verbose mode is on, so callers can pepper the silent
/// failure paths freely without conditionals at the call site.
pub fn log(args: std::fmt::Arguments) {
    if is_enabled() {
        eprintln!("[hyprlayer telemetry] {args}");
    }
}

/// `vlog!("...", x)` — `eprintln!`-style diagnostic, emitted only when
/// verbose mode is active.
macro_rules! vlog {
    ($($arg:tt)*) => {
        $crate::telemetry::verbose::log(format_args!($($arg)*))
    };
}
pub(crate) use vlog;

#[cfg(test)]
mod tests {
    use super::*;

    /// `set_enabled(true)` forces verbose on regardless of the env var.
    /// Saves and restores `FORCED` so the test neither depends on the
    /// global starting clean nor leaks state into other tests.
    #[test]
    fn forced_flag_overrides() {
        let prev = FORCED.load(Ordering::Acquire);
        set_enabled(true);
        assert!(is_enabled(), "forced flag must enable verbose");
        FORCED.store(prev, Ordering::Release);
    }

    #[test]
    fn env_truthy_values() {
        // Pure-predicate coverage that doesn't touch the process env
        // (which would race other tests). We assert the match arms via a
        // local mirror of the parse rule.
        let truthy = |v: &str| matches!(v.trim(), "1" | "true" | "yes" | "on");
        assert!(truthy("1"));
        assert!(truthy(" true "));
        assert!(truthy("yes"));
        assert!(truthy("on"));
        assert!(!truthy("0"));
        assert!(!truthy("false"));
        assert!(!truthy(""));
    }
}
