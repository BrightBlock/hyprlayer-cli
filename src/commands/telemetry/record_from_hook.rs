//! Privacy invariant: only typed fields (`usage` integers, the `model`
//! identifier, ISO timestamps, `<command-name>` markers) are read from the
//! transcript. Pinned by `transcript_content_never_leaks_to_spool`.
//!
//! Exit-code invariant: always `Ok(())`. Stop hook treats exit 2 as
//! blocking and surfaces stderr's first line on any other non-zero exit.

use anyhow::Result;
use std::io::{BufRead, Read};
use std::path::PathBuf;

use crate::cli::TelemetryRecordFromHookArgs;
use crate::telemetry::event::{Event, Outcome};
use crate::telemetry::spool;

/// Hook payloads are documented as a single small JSON object; cap the
/// stdin read so a misbehaving hook caller can't drive hyprlayer to OOM.
const STDIN_CAP_BYTES: u64 = 64 * 1024;

pub fn record_from_hook(args: TelemetryRecordFromHookArgs) -> Result<()> {
    let TelemetryRecordFromHookArgs { config } = args;

    let mut stdin_buf = String::new();
    if std::io::stdin()
        .take(STDIN_CAP_BYTES)
        .read_to_string(&mut stdin_buf)
        .is_err()
    {
        return Ok(());
    }
    let Some(payload) = parse_hook_payload(&stdin_buf) else {
        return Ok(());
    };
    let Ok(Some(cfg)) = config.load_if_exists() else {
        return Ok(());
    };
    if !cfg.telemetry.is_recording() {
        return Ok(());
    }
    let Ok(meta) = std::fs::metadata(&payload.transcript_path) else {
        return Ok(());
    };
    if !meta.is_file() {
        return Ok(());
    }
    let Ok(file) = std::fs::File::open(&payload.transcript_path) else {
        return Ok(());
    };
    let Some(summary) = summarize_turn(std::io::BufReader::new(file)) else {
        return Ok(());
    };

    let session_id = (!payload.session_id.is_empty()).then_some(payload.session_id);
    let mut event = Event::skill_run(
        &summary.skill,
        session_id,
        summary.duration_ms,
        Outcome::Success,
        None,
        &cfg,
    );
    event.input_tokens = summary.input_tokens;
    event.output_tokens = summary.output_tokens;
    event.cache_read_tokens = summary.cache_read_tokens;
    event.cache_creation_tokens = summary.cache_creation_tokens;
    event.model = summary.model;
    if let Some(ts) = summary.started_at {
        event.event_timestamp = ts;
    }
    let _ = spool::append(&event);
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct HookPayload {
    pub session_id: String,
    pub transcript_path: PathBuf,
}

pub(crate) fn parse_hook_payload(s: &str) -> Option<HookPayload> {
    let v: serde_json::Value = serde_json::from_str(s.trim()).ok()?;
    let transcript_path = v.get("transcript_path")?.as_str()?.to_string();
    if transcript_path.is_empty() {
        return None;
    }
    let session_id = v
        .get("session_id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some(HookPayload {
        session_id,
        transcript_path: PathBuf::from(transcript_path),
    })
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TurnSummary {
    pub skill: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    /// Model from the most recent *real* assistant message in the skill
    /// turn (`message.model`), ignoring Claude Code's `<synthetic>` sentinel
    /// for locally-generated messages. Reflects per-skill model overrides;
    /// the last real model wins on a mid-turn switch.
    pub model: Option<String>,
    pub duration_ms: Option<u64>,
    /// First assistant timestamp after the skill marker, used as the
    /// PostHog `event_timestamp` so dashboards bucket events by when
    /// the turn started rather than when the hook fired.
    pub started_at: Option<String>,
}

/// Returns `Some` only when the **most recent real-user message** carries
/// a `<command-name>` marker. A real-user message excludes Claude Code's
/// `isMeta: true` SKILL-expansion records and tool-result messages.
/// This means a `/commit` invocation followed by chat turns produces a
/// single skill_run event on the skill's own Stop, not a duplicate event
/// per follow-up turn.
pub(crate) fn summarize_turn<R: BufRead>(reader: R) -> Option<TurnSummary> {
    let mut active: Option<TurnSummary> = None;
    let mut first_ts: Option<String> = None;
    let mut last_ms: Option<i64> = None;
    let mut first_ms: Option<i64> = None;

    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|x| x.as_str()) {
            Some("user") => {
                if !is_real_user_message(&v) {
                    continue;
                }
                // Real user message: a new turn boundary. Reset state.
                // If this user message has a marker, start a new active
                // summary; if not, leave active as None so the trailing
                // assistant work isn't attributed to a stale skill.
                first_ts = None;
                first_ms = None;
                last_ms = None;
                active = extract_command_name(&v).map(|name| TurnSummary {
                    skill: name,
                    ..Default::default()
                });
            }
            Some("assistant") => {
                let Some(s) = active.as_mut() else { continue };
                accumulate_usage(&v, s);
                // `message.model` is the model that produced this turn and
                // reflects per-skill overrides (a sonnet-pinned skill records
                // `claude-sonnet-*` even inside an opus session). Skip Claude
                // Code's `<synthetic>` sentinel — and any `<…>` placeholder for
                // locally-generated messages — plus the empty string, so we keep
                // the last *real* model rather than a trailing synthetic/blank one.
                if let Some(model) = v.pointer("/message/model").and_then(|m| m.as_str())
                    && !model.is_empty()
                    && !model.starts_with('<')
                {
                    s.model = Some(model.to_string());
                }
                if let Some(ts_str) = v.get("timestamp").and_then(|x| x.as_str())
                    && let Some(ms) = parse_ts_ms(ts_str)
                {
                    if first_ms.is_none() {
                        first_ms = Some(ms);
                        first_ts = Some(ts_str.to_string());
                    }
                    last_ms = Some(ms);
                }
            }
            _ => {}
        }
    }

    active.map(|mut s| {
        if let (Some(a), Some(b)) = (first_ms, last_ms) {
            s.duration_ms = u64::try_from(b.saturating_sub(a)).ok();
        }
        s.started_at = first_ts;
        s
    })
}

/// True if `v` is a fresh user prompt (typed text or slash command),
/// false for Claude Code's `isMeta: true` SKILL-expansion records and
/// tool-result messages (which use the `user` role to deliver tool
/// output back to the model).
fn is_real_user_message(v: &serde_json::Value) -> bool {
    if v.get("isMeta").and_then(|x| x.as_bool()).unwrap_or(false) {
        return false;
    }
    let Some(content) = v.pointer("/message/content") else {
        return false;
    };
    match content {
        serde_json::Value::String(_) => true,
        serde_json::Value::Array(items) => !items
            .iter()
            .any(|item| item.get("type").and_then(|x| x.as_str()) == Some("tool_result")),
        _ => false,
    }
}

fn accumulate_usage(v: &serde_json::Value, s: &mut TurnSummary) {
    let Some(usage) = v.pointer("/message/usage") else {
        return;
    };
    add_to(&mut s.input_tokens, usage.get("input_tokens"));
    add_to(&mut s.output_tokens, usage.get("output_tokens"));
    add_to(
        &mut s.cache_read_tokens,
        usage.get("cache_read_input_tokens"),
    );
    add_to(
        &mut s.cache_creation_tokens,
        usage.get("cache_creation_input_tokens"),
    );
}

fn add_to(slot: &mut Option<u64>, val: Option<&serde_json::Value>) {
    let Some(n) = val.and_then(|v| v.as_u64()) else {
        return;
    };
    *slot = Some(slot.unwrap_or(0).saturating_add(n));
}

fn extract_command_name(v: &serde_json::Value) -> Option<String> {
    let content = v.pointer("/message/content")?;
    match content {
        serde_json::Value::String(s) => parse_command_marker(s),
        serde_json::Value::Array(items) => items.iter().find_map(|item| {
            item.get("text")
                .and_then(|x| x.as_str())
                .and_then(parse_command_marker)
        }),
        _ => None,
    }
}

/// Strip the leading `/` so the spooled `skill` matches the bare name
/// the in-skill beacon and PostHog dashboards already key on. Requires
/// `<command-message>` co-presence: pasted text containing only
/// `<command-name>...` doesn't get attributed as a skill invocation.
fn parse_command_marker(s: &str) -> Option<String> {
    if !s.contains("<command-message>") {
        return None;
    }
    const OPEN: &str = "<command-name>";
    const CLOSE: &str = "</command-name>";
    let start = s.find(OPEN)? + OPEN.len();
    let rest = &s[start..];
    let end = rest.find(CLOSE)?;
    let raw = rest[..end].trim();
    let name = raw.strip_prefix('/').unwrap_or(raw).trim();
    if name.is_empty() || !name.chars().all(is_skill_name_char) {
        return None;
    }
    Some(name.to_string())
}

fn is_skill_name_char(c: char) -> bool {
    // Skill names: alphanumeric, underscore, hyphen, plus `:` for
    // plugin-namespaced commands (`engineering:code-review`) and `.`
    // for dotted-namespace conventions.
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')
}

fn parse_ts_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn summarize(jsonl: &str) -> Option<TurnSummary> {
        summarize_turn(Cursor::new(jsonl))
    }

    #[test]
    fn parse_payload_accepts_minimal_schema() {
        let json = r#"{"session_id":"abc123","transcript_path":"/tmp/x.jsonl","cwd":"/work","hook_event_name":"Stop"}"#;
        let p = parse_hook_payload(json).expect("must parse");
        assert_eq!(p.session_id, "abc123");
        assert_eq!(p.transcript_path, PathBuf::from("/tmp/x.jsonl"));
    }

    #[test]
    fn parse_payload_silent_on_garbage() {
        assert!(parse_hook_payload("not json").is_none());
        assert!(parse_hook_payload("").is_none());
        assert!(parse_hook_payload("{}").is_none());
        assert!(parse_hook_payload(r#"{"transcript_path": ""}"#).is_none());
    }

    #[test]
    fn parse_payload_tolerates_extra_fields() {
        let json = r#"{"session_id":"abc","transcript_path":"/t.jsonl","future_field":"xyz","permission_mode":"default"}"#;
        assert!(parse_hook_payload(json).is_some());
    }

    #[test]
    fn parse_command_marker_strips_leading_slash() {
        assert_eq!(
            parse_command_marker(
                "<command-message>research_codebase</command-message>\n<command-name>/research_codebase</command-name>"
            )
            .as_deref(),
            Some("research_codebase")
        );
    }

    #[test]
    fn parse_command_marker_handles_no_slash_prefix() {
        assert_eq!(
            parse_command_marker(
                "<command-message>commit</command-message><command-name>commit</command-name>"
            )
            .as_deref(),
            Some("commit")
        );
    }

    #[test]
    fn parse_command_marker_returns_none_when_missing() {
        assert!(parse_command_marker("plain user prompt").is_none());
        assert!(
            parse_command_marker(
                "<command-message>x</command-message><command-name></command-name>"
            )
            .is_none()
        );
    }

    #[test]
    fn parse_command_marker_rejects_pasted_command_name_without_command_message() {
        // Pasted text or buggy MCP output containing only the
        // <command-name> tag must not attribute a skill.
        assert!(
            parse_command_marker("look at <command-name>/iterate_plan</command-name>").is_none()
        );
    }

    #[test]
    fn parse_command_marker_accepts_namespaced_skill_names() {
        // Plugin-namespaced skills use `:`, dotted namespaces `.`.
        assert_eq!(
            parse_command_marker(
                "<command-message>code-review</command-message><command-name>/engineering:code-review</command-name>"
            )
            .as_deref(),
            Some("engineering:code-review")
        );
    }

    #[test]
    fn parse_command_marker_rejects_pathological_chars() {
        // Pathological nested markers must not produce junk skill names
        // — the inner `<command-name>` brackets contain `<` and `>`,
        // neither of which are valid skill-name chars.
        assert!(
            parse_command_marker(
                "<command-message>x</command-message><command-name>/A<command-name>/B</command-name>"
            )
            .is_none()
        );
    }

    #[test]
    fn summarize_turn_extracts_skill_and_token_totals() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>implement_plan</command-message>\n<command-name>/implement_plan</command-name>"},"timestamp":"2026-05-08T00:57:57.086Z"}
{"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-1","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":5000,"cache_creation_input_tokens":250}},"timestamp":"2026-05-08T00:58:00.000Z"}
{"type":"assistant","message":{"id":"m2","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"there"}],"usage":{"input_tokens":50,"output_tokens":10,"cache_read_input_tokens":3000,"cache_creation_input_tokens":0}},"timestamp":"2026-05-08T00:58:05.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "implement_plan");
        assert_eq!(s.input_tokens, Some(150));
        assert_eq!(s.output_tokens, Some(30));
        assert_eq!(s.cache_read_tokens, Some(8000));
        assert_eq!(s.cache_creation_tokens, Some(250));
        // Last assistant message's model wins (m2 over m1).
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(s.duration_ms, Some(5_000));
        assert_eq!(s.started_at.as_deref(), Some("2026-05-08T00:58:00.000Z"));
    }

    #[test]
    fn summarize_turn_skips_synthetic_and_empty_model_keeps_last_real() {
        // Claude Code tags locally-generated assistant messages with model
        // "<synthetic>" (no real API call), and some entries carry an empty
        // model string. The captured model must be the last *real* model,
        // never a trailing synthetic sentinel or a blank value.
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":10}},"timestamp":"2026-05-08T00:58:00.000Z"}
{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0}},"timestamp":"2026-05-08T00:58:01.000Z"}
{"type":"assistant","message":{"model":"","usage":{"input_tokens":0}},"timestamp":"2026-05-08T00:58:02.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "commit");
        // last real model wins; trailing <synthetic> and "" are both ignored.
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn summarize_turn_returns_none_for_non_skill_turn() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"just a question"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"id":"m","usage":{"input_tokens":1,"output_tokens":1}},"timestamp":"2026-05-08T00:58:00.000Z"}
"#;
        assert!(summarize(jsonl).is_none());
    }

    #[test]
    fn summarize_turn_returns_none_for_followup_after_skill() {
        // Critical: the Stop hook fires on every assistant turn, not
        // just skill turns. After /commit, a chat follow-up must NOT
        // produce another skill_run event for `commit`.
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":100}},"timestamp":"2026-05-08T00:58:00.000Z"}
{"type":"user","message":{"role":"user","content":"thanks, also explain the diff"},"timestamp":"2026-05-08T00:58:30.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":50}},"timestamp":"2026-05-08T00:59:00.000Z"}
"#;
        assert!(
            summarize(jsonl).is_none(),
            "follow-up turn after a skill must not re-emit"
        );
    }

    #[test]
    fn summarize_turn_ignores_tool_result_user_messages() {
        // Tool results are encoded as user-role messages with
        // type:"tool_result" content. They're part of the assistant's
        // turn, not new user input.
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>cost_estimate</command-message>\n<command-name>/cost_estimate</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":10}},"timestamp":"2026-05-08T00:58:00.000Z"}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"ok"}]},"timestamp":"2026-05-08T00:58:01.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":20}},"timestamp":"2026-05-08T00:58:05.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "cost_estimate");
        assert_eq!(s.input_tokens, Some(30));
    }

    #[test]
    fn summarize_turn_ignores_meta_user_messages() {
        // isMeta:true is the SKILL.md expansion injected by Claude
        // Code right after the slash command — same turn, not a new
        // user input.
        let jsonl = r##"
{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"# Commit\nDo a thing."}]},"timestamp":"2026-05-08T00:57:57.100Z"}
{"type":"assistant","message":{"usage":{"input_tokens":7,"output_tokens":3}},"timestamp":"2026-05-08T00:58:00.000Z"}
"##;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "commit");
        assert_eq!(s.input_tokens, Some(7));
    }

    #[test]
    fn summarize_turn_tolerates_malformed_lines() {
        let jsonl = r#"this is not json
{"type":"user","message":{"role":"user","content":"<command-message>commit</command-message>\n<command-name>/commit</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{garbage}
{"type":"assistant","message":{"usage":{"input_tokens":7}},"timestamp":"2026-05-08T00:58:00.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "commit");
        assert_eq!(s.input_tokens, Some(7));
    }

    #[test]
    fn summarize_turn_handles_array_content() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<command-message>cost_estimate</command-message><command-name>/cost_estimate</command-name> please"}]},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"output_tokens":42}},"timestamp":"2026-05-08T00:58:00.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "cost_estimate");
        assert_eq!(s.output_tokens, Some(42));
    }

    #[test]
    fn summarize_turn_partial_usage_fields() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>x</command-message>\n<command-name>/x</command-name>"},"timestamp":"2026-05-08T00:57:57.000Z"}
{"type":"assistant","message":{"usage":{"input_tokens":10,"output_tokens":5}},"timestamp":"2026-05-08T00:58:00.000Z"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.input_tokens, Some(10));
        assert_eq!(s.output_tokens, Some(5));
        assert!(s.cache_read_tokens.is_none());
        assert!(s.cache_creation_tokens.is_none());
        assert!(s.model.is_none());
    }

    #[test]
    fn summarize_turn_unparseable_timestamps_leave_duration_none() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"<command-message>x</command-message>\n<command-name>/x</command-name>"},"timestamp":"not-a-real-timestamp"}
{"type":"assistant","message":{"usage":{"input_tokens":7}},"timestamp":"also bad"}
{"type":"assistant","message":{"usage":{"input_tokens":3}},"timestamp":"and bad"}
"#;
        let s = summarize(jsonl).expect("must summarize");
        assert_eq!(s.skill, "x");
        assert_eq!(s.input_tokens, Some(10));
        assert!(s.duration_ms.is_none());
        assert!(s.started_at.is_none());
    }

    #[test]
    fn parse_ts_ms_round_trip() {
        assert_eq!(parse_ts_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_ts_ms("1970-01-02T00:00:00.000Z"), Some(86_400_000));
        assert_eq!(parse_ts_ms("not a date"), None);
    }
}
