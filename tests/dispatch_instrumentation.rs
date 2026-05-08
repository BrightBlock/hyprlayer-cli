//! Integration tests for the cli_command dispatch wrapper.

mod common;
use common::*;

#[test]
fn opted_in_subcommand_appends_one_cli_command_event() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);

    let out = run(&xdg, &["telemetry", "status"]);
    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let cli_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("cli_command"))
        .collect();
    assert_eq!(cli_events.len(), 1, "events: {:?}", events);
    let ev = cli_events[0];
    assert_eq!(
        ev.get("command").and_then(|v| v.as_str()),
        Some("telemetry.status")
    );
    assert_eq!(ev.get("outcome").and_then(|v| v.as_str()), Some("success"));
}

#[test]
fn opted_out_subcommand_appends_no_events() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, false);

    let out = run(&xdg, &["telemetry", "status"]);
    assert!(out.status.success());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    assert!(
        events.is_empty(),
        "spool must be empty when telemetry is off, got {events:?}"
    );
}

#[test]
fn skill_end_does_not_emit_dispatch_event() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);

    let start = run(&xdg, &["telemetry", "skill-start", "--skill", "test_skill"]);
    assert!(start.status.success());
    let token = String::from_utf8(start.stdout).unwrap().trim().to_string();

    let out = run(
        &xdg,
        &[
            "telemetry",
            "skill-end",
            "--skill",
            "test_skill",
            "--session",
            &token,
            "--outcome",
            "success",
        ],
    );
    assert!(out.status.success());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);

    let dispatch_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("cli_command"))
        .filter(|e| {
            e.get("command")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("telemetry.skill_"))
        })
        .collect();
    assert!(
        dispatch_events.is_empty(),
        "telemetry.skill_* must not emit its own cli_command event"
    );

    let skill_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("skill_run"))
        .collect();
    assert_eq!(skill_events.len(), 1);
    assert_eq!(
        skill_events[0].get("skill").and_then(|v| v.as_str()),
        Some("test_skill")
    );
}

#[test]
fn skill_start_prints_session_token_and_nothing_else() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let out = run(
        &xdg,
        &["telemetry", "skill-start", "--skill", "code_review"],
    );
    assert!(out.status.success(), "expected exit 0");
    assert!(
        out.stderr.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let token = stdout.trim();
    assert!(
        looks_like_session_token(token),
        "stdout doesn't look like a session token: `{stdout}`"
    );
}

#[test]
fn skill_start_then_skill_end_records_event_with_duration() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);

    let start = run(
        &xdg,
        &["telemetry", "skill-start", "--skill", "research_codebase"],
    );
    assert!(start.status.success());
    let token = String::from_utf8(start.stdout)
        .expect("token should be utf-8")
        .trim()
        .to_string();
    assert!(!token.is_empty(), "skill-start produced empty stdout");

    std::thread::sleep(std::time::Duration::from_millis(50));

    let end = run(
        &xdg,
        &[
            "telemetry",
            "skill-end",
            "--skill",
            "research_codebase",
            "--session",
            &token,
        ],
    );
    assert!(end.status.success());
    assert!(end.stdout.is_empty());
    assert!(end.stderr.is_empty());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let skill_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("skill_run"))
        .collect();
    assert_eq!(skill_events.len(), 1, "events: {events:?}");
    let ev = skill_events[0];
    assert_eq!(
        ev.get("skill").and_then(|v| v.as_str()),
        Some("research_codebase")
    );
    assert_eq!(
        ev.get("session_id").and_then(|v| v.as_str()),
        Some(token.as_str())
    );
    let duration = ev
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .expect("duration_ms must be populated");
    assert!(
        (10..30_000).contains(&duration),
        "duration_ms out of plausible range for a 50ms sleep: {duration}"
    );
}

#[test]
fn skill_end_with_garbage_session_still_emits_event_no_duration() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);

    let out = run(
        &xdg,
        &[
            "telemetry",
            "skill-end",
            "--skill",
            "code_review",
            "--session",
            "not-a-real-token",
        ],
    );
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let skill_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("skill_run"))
        .collect();
    assert_eq!(skill_events.len(), 1);
    assert!(
        skill_events[0]
            .get("duration_ms")
            .map(|v| v.is_null())
            .unwrap_or(true),
        "duration_ms should be null/missing for malformed session: {:?}",
        skill_events[0]
    );
}

#[test]
fn skill_start_and_skill_end_are_silent_when_opted_out() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, false);

    let start = run(&xdg, &["telemetry", "skill-start", "--skill", "x"]);
    assert!(start.status.success());
    // skill-start always prints; the opt-out gate is at skill-end.
    assert!(!start.stdout.is_empty());

    let token = String::from_utf8(start.stdout).unwrap().trim().to_string();
    let end = run(
        &xdg,
        &[
            "telemetry",
            "skill-end",
            "--skill",
            "x",
            "--session",
            &token,
        ],
    );
    assert!(end.status.success());
    assert!(end.stdout.is_empty());
    assert!(end.stderr.is_empty());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let skill_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("skill_run"))
        .collect();
    assert!(
        skill_events.is_empty(),
        "opted-out skill-end must not spool: {events:?}"
    );
}

fn looks_like_session_token(s: &str) -> bool {
    let Some((head, tail)) = s.split_once('-') else {
        return false;
    };
    !head.is_empty()
        && head.chars().all(|c| c.is_ascii_digit())
        && tail.len() == 8
        && tail.chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn skill_beacon_skips_startup_checks() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let start = run(&xdg, &["telemetry", "skill-start", "--skill", "smoke"]);
    let token = String::from_utf8(start.stdout).unwrap().trim().to_string();
    let out = run(
        &xdg,
        &[
            "telemetry",
            "skill-end",
            "--skill",
            "smoke",
            "--session",
            &token,
        ],
    );
    assert!(
        out.stderr.is_empty(),
        "stderr leak: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn failed_subcommand_emits_failure_outcome_with_error_class() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);

    let out = run(&xdg, &["thoughts", "status"]);
    assert!(!out.status.success(), "expected failure, got success");

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let failed: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("cli_command"))
        .filter(|e| e.get("outcome").and_then(|v| v.as_str()) == Some("failure"))
        .collect();
    assert_eq!(failed.len(), 1, "events: {events:?}");
    let class = failed[0]
        .get("error_class")
        .and_then(|v| v.as_str())
        .expect("error_class must be set on failure");
    assert!(
        !class.contains(' '),
        "error_class not a stable id: `{class}`"
    );
    assert!(
        !class.contains('/') && !class.contains('@'),
        "error_class leaks path/email chars: `{class}`"
    );
}
