//! Integration tests for `hyprlayer telemetry record-from-hook`.

mod common;
use common::*;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run_with_stdin(xdg: &Path, stdin_payload: &str) -> std::process::Output {
    let mut cmd = Command::new(hyprlayer_bin())
        .args(["telemetry", "record-from-hook"])
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", xdg)
        .env("HYPRLAYER_DISABLE_BACKGROUND_FLUSH", "1")
        .env_remove("HYPRLAYER_TELEMETRY_KEY")
        .env_remove("HYPRLAYER_ORG_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hyprlayer should spawn");
    cmd.stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_payload.as_bytes())
        .unwrap();
    cmd.wait_with_output().unwrap()
}

const SECRET: &str = "SECRET_LEAK_MARKER_x9k2";
const EMAIL: &str = "alice-leak@example.com";
const PATH_LEAK: &str = "/users-secret-path/foo.txt";

fn fixture_transcript() -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":"<command-message>cost_estimate</command-message>\n<command-name>/cost_estimate</command-name>\n<command-args>{SECRET}</command-args>"}},"timestamp":"2026-05-08T00:57:57.086Z"}}
{{"type":"assistant","message":{{"id":"m1","role":"assistant","model":"claude-opus-4-1","content":[{{"type":"text","text":"hello {EMAIL}"}}],"usage":{{"input_tokens":120,"output_tokens":40,"cache_read_input_tokens":2000,"cache_creation_input_tokens":100}}}},"timestamp":"2026-05-08T00:58:00.000Z"}}
{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","content":"file contents at {PATH_LEAK}"}}]}},"timestamp":"2026-05-08T00:58:01.000Z"}}
{{"type":"assistant","message":{{"id":"m2","role":"assistant","model":"claude-sonnet-4-5","content":[{{"type":"text","text":"acknowledged"}}],"usage":{{"input_tokens":80,"output_tokens":20,"cache_read_input_tokens":1500,"cache_creation_input_tokens":50}}}},"timestamp":"2026-05-08T00:58:05.000Z"}}
"#
    )
}

#[test]
fn round_trip_extracts_skill_and_token_totals() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(&transcript, fixture_transcript()).unwrap();

    let payload = serde_json::json!({
        "session_id": "sess-abc",
        "transcript_path": transcript,
        "cwd": xdg,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success(), "expected exit 0, got {:?}", out);
    assert!(out.stdout.is_empty());
    assert!(
        out.stderr.is_empty(),
        "stderr leak: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );

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
        Some("cost_estimate")
    );
    assert_eq!(
        ev.get("session_id").and_then(|v| v.as_str()),
        Some("sess-abc")
    );
    assert_eq!(ev.get("input_tokens").and_then(|v| v.as_u64()), Some(200));
    assert_eq!(ev.get("output_tokens").and_then(|v| v.as_u64()), Some(60));
    assert_eq!(
        ev.get("cache_read_tokens").and_then(|v| v.as_u64()),
        Some(3500)
    );
    assert_eq!(
        ev.get("cache_creation_tokens").and_then(|v| v.as_u64()),
        Some(150)
    );
    assert_eq!(
        ev.get("model").and_then(|v| v.as_str()),
        Some("claude-sonnet-4-5")
    );
    assert_eq!(ev.get("duration_ms").and_then(|v| v.as_u64()), Some(5_000));
}

#[test]
fn transcript_content_never_leaks_to_spool() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(&transcript, fixture_transcript()).unwrap();

    let payload = serde_json::json!({
        "session_id": "sess-priv",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());

    let spool_path = locate_spool(&xdg);
    let spool = std::fs::read(&spool_path).unwrap();
    let spool_text = String::from_utf8_lossy(&spool);
    for needle in [SECRET, EMAIL, PATH_LEAK, "tool_result", "acknowledged"] {
        assert!(
            !spool_text.contains(needle),
            "spool leaked transcript content `{needle}`:\n{spool_text}"
        );
    }
}

#[test]
fn non_skill_turn_emits_nothing() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"user","message":{"role":"user","content":"plain question"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":10,"output_tokens":2}},"timestamp":"2026-05-08T00:58:00.000Z"}
"#,
    )
    .unwrap();

    let payload = serde_json::json!({
        "session_id": "sess-non-skill",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    assert!(
        events.is_empty(),
        "non-skill turn produced events: {events:?}"
    );
}

#[test]
fn malformed_stdin_silent_no_op() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let out = run_with_stdin(&xdg, "{not even json}");
    assert!(out.status.success(), "must exit 0, got {:?}", out);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn empty_stdin_silent_no_op() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let out = run_with_stdin(&xdg, "");
    assert!(
        out.status.success(),
        "empty stdin must exit 0, got {:?}",
        out
    );
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn missing_transcript_silent_no_op() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let payload = serde_json::json!({
        "session_id": "sess-x",
        "transcript_path": "/no/such/file/transcript.jsonl",
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());
    let spool = locate_spool(&xdg);
    assert!(read_spool_events(&spool).is_empty());
}

#[test]
fn malformed_jsonl_silent_no_op() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(&transcript, b"\x00\x00garbage line\n{not json}\n{}\n").unwrap();
    let payload = serde_json::json!({
        "session_id": "sess-y",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());
}

#[test]
fn opted_out_skips_event_even_with_rich_transcript() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, false);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(&transcript, fixture_transcript()).unwrap();

    let payload = serde_json::json!({
        "session_id": "sess-off",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());
    let spool = locate_spool(&xdg);
    assert!(
        read_spool_events(&spool).is_empty(),
        "opted-out user must not spool events"
    );
}

#[test]
fn followup_turn_after_skill_emits_nothing() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":100}},"timestamp":"2026-05-08T00:58:00.000Z"}
{"type":"user","message":{"role":"user","content":"thanks, also explain the diff"},"timestamp":"2026-05-08T00:58:30.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":50}},"timestamp":"2026-05-08T00:59:00.000Z"}
"#,
    )
    .unwrap();

    let payload = serde_json::json!({
        "session_id": "sess-followup",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    assert!(
        events.is_empty(),
        "follow-up turn after a skill must not re-emit: {events:?}"
    );
}

#[test]
fn always_exits_zero_under_pathological_inputs() {
    // Exit 2 from a Stop hook blocks Claude's turn — no failure mode
    // here may exit non-zero.
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let cases: &[&str] = &[
        "",
        "garbage",
        "{}",
        r#"{"transcript_path": ""}"#,
        r#"{"transcript_path": "/no/such/file"}"#,
        &serde_json::json!({"transcript_path": std::env::temp_dir()}).to_string(),
    ];
    for case in cases {
        let out = run_with_stdin(&xdg, case);
        assert!(
            out.status.success(),
            "non-zero exit for input `{case}`: {:?}",
            out
        );
    }
}

#[test]
fn slash_invocation_in_meta_user_message_still_resolves_skill() {
    let (_dir, xdg) = isolated_dirs();
    write_config_in_both_layouts(&xdg, true);
    let transcript = xdg.join("transcript.jsonl");
    let body = r##"{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"# Commit\nDo a thing."}]},"timestamp":"2026-05-08T00:57:57.100Z"}
{"type":"assistant","message":{"usage":{"input_tokens":7,"output_tokens":3}},"timestamp":"2026-05-08T00:58:00.000Z"}
"##;
    std::fs::write(&transcript, body).unwrap();
    let payload = serde_json::json!({
        "session_id": "sess-z",
        "transcript_path": transcript,
        "hook_event_name": "Stop",
    });
    let out = run_with_stdin(&xdg, &serde_json::to_string(&payload).unwrap());
    assert!(out.status.success());

    let spool = locate_spool(&xdg);
    let events = read_spool_events(&spool);
    let skill_events: Vec<_> = events
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("skill_run"))
        .collect();
    assert_eq!(skill_events.len(), 1);
    assert_eq!(
        skill_events[0].get("skill").and_then(|v| v.as_str()),
        Some("commit")
    );
}
