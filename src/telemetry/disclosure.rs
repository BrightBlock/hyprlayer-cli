//! One-time privacy disclosure printed on first auto-enrollment to the
//! community PostHog project. Suppressed when an org override resolves
//! (the org has handled comms internally) and when telemetry is disabled
//! before `ai configure` runs.

/// Print the privacy contract to stderr. Caller has already decided this
/// is the first run (`installation_id == None`) and that we resolved to
/// the hardcoded community key (not an org override).
pub fn print_telemetry_disclosure() {
    let lines: &[&str] = &[
        "",
        "hyprlayer telemetry enabled (anonymous mode).",
        "  We don't store personally identifiable information.",
        "  Opt out anytime: hyprlayer telemetry off",
    ];
    for line in lines {
        eprintln!("{line}");
    }
}
