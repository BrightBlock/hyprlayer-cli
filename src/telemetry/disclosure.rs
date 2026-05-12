//! One-time privacy disclosure printed on first auto-enrollment to the
//! community PostHog project. Suppressed when an org override resolves
//! (the org has handled comms internally) and when telemetry is disabled
//! before `ai configure` runs.

fn print_lines(lines: &[&str]) {
    for line in lines {
        eprintln!("{line}");
    }
}

/// Print the privacy contract to stderr. Caller has already decided this
/// is the first run (`installation_id == None`) and that we resolved to
/// the hardcoded community key (not an org override).
pub fn print_telemetry_disclosure() {
    print_lines(&[
        "",
        "hyprlayer telemetry enabled (anonymous mode).",
        "  We don't store personally identifiable information.",
        "  Opt out anytime: hyprlayer telemetry off",
    ]);
}

/// Fired once on the non-locked → locked transition; the org owns
/// subsequent user-facing comms.
pub fn print_corporate_lock_disclosure(owner_repo: &str, org_id: Option<&str>) {
    let source_line = match org_id {
        Some(o) => format!("  Org: {o} ({owner_repo})"),
        None => format!("  Source: {owner_repo} (HYPRLAYER_TELEMETRY_KEY)"),
    };
    print_lines(&[
        "",
        "hyprlayer telemetry enabled by your organization (identified mode).",
        &source_line,
        "  Opt-out is disabled by policy. Contact your org admin to release the lock.",
    ]);
}
