// Parser for OpenAI Codex CLI --json (JSONL) output.
//
// Tested against codex-cli 0.128.0.
// Event mapping is stable across recent versions but OpenAI may evolve it.
// All field access uses .get().and_then() chains, so missing fields produce
// silent skips rather than panics.

use std::io::{self, BufRead, ErrorKind, Write};

use anyhow::Result;
use serde_json::Value;

use crate::cli::CodexStreamArgs;

pub fn stream(args: CodexStreamArgs) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();
    parse_stream(
        stdin.lock(),
        stdout.lock(),
        stderr.lock(),
        StreamOpts {
            include_thinking: !args.no_thinking,
            include_tool_calls: !args.no_tool_calls,
        },
    )
}

#[derive(Debug, Clone, Copy)]
pub struct StreamOpts {
    pub include_thinking: bool,
    pub include_tool_calls: bool,
}

// Treat `Err(BrokenPipe)` as a clean shutdown — downstream consumer (e.g. `head`)
// closed early. Any other write error propagates.
macro_rules! tryw {
    ($out:expr, $($arg:tt)*) => {
        match writeln!($out, $($arg)*) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    };
}

pub fn parse_stream<R, O, E>(mut reader: R, mut out: O, mut err: E, opts: StreamOpts) -> Result<()>
where
    R: BufRead,
    O: Write,
    E: Write,
{
    let mut saw_any_event = false;
    let mut turn_completed_count: u32 = 0;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        // `from_utf8_lossy` replaces invalid bytes with U+FFFD instead of dropping
        // the entire line — losing one character is better than losing one event.
        let line = String::from_utf8_lossy(&buf[..n]);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type.is_empty() {
            continue;
        }
        saw_any_event = true;

        match event_type {
            "thread.started" => {
                if let Some(tid) = event.get("thread_id").and_then(|v| v.as_str()) {
                    tryw!(out, "SESSION_ID:{}", tid);
                }
            }
            "item.completed" => {
                let Some(item) = event.get("item") else {
                    continue;
                };
                let itype = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                match itype {
                    "reasoning" if opts.include_thinking && !text.is_empty() => {
                        tryw!(out, "[codex thinking] {}", text);
                        tryw!(out, "");
                    }
                    "agent_message" if !text.is_empty() => {
                        tryw!(out, "{}", text);
                    }
                    "command_execution" if opts.include_tool_calls => {
                        if let Some(cmd) = item.get("command").and_then(|v| v.as_str())
                            && !cmd.is_empty()
                        {
                            tryw!(out, "[codex ran] {}", cmd);
                        }
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                turn_completed_count += 1;
                if let Some(usage) = event.get("usage") {
                    let input = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let total = input.saturating_add(output);
                    if total > 0 {
                        tryw!(out, "");
                        tryw!(out, "tokens: {}", total);
                    }
                }
            }
            _ => {}
        }
    }

    // Only warn when we processed events but never saw a turn boundary —
    // a fully empty stream stays silent so empty pipelines don't get noisy.
    if saw_any_event && turn_completed_count == 0 {
        match writeln!(
            err,
            "[warning] no turn.completed event — possible mid-stream disconnect"
        ) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(thinking: bool, tools: bool) -> StreamOpts {
        StreamOpts {
            include_thinking: thinking,
            include_tool_calls: tools,
        }
    }

    fn run(input: &str, opts: StreamOpts) -> (String, String) {
        run_bytes(input.as_bytes(), opts)
    }

    fn run_bytes(input: &[u8], opts: StreamOpts) -> (String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        parse_stream(input, &mut out, &mut err, opts).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    const HAPPY_PATH: &str = r#"{"type":"thread.started","thread_id":"thr_abc"}
{"type":"item.completed","item":{"type":"reasoning","text":"Looking at the diff"}}
{"type":"item.completed","item":{"type":"command_execution","command":"git diff origin/main"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Found a race condition in foo.rs:42"}}
{"type":"turn.completed","usage":{"input_tokens":1000,"output_tokens":500}}
"#;

    #[test]
    fn happy_path_emits_session_thinking_tool_message_and_tokens() {
        let (out, err) = run(HAPPY_PATH, opts(true, true));
        let expected = "SESSION_ID:thr_abc\n\
                        [codex thinking] Looking at the diff\n\
                        \n\
                        [codex ran] git diff origin/main\n\
                        Found a race condition in foo.rs:42\n\
                        \n\
                        tokens: 1500\n";
        assert_eq!(out, expected);
        assert!(err.is_empty());
    }

    #[test]
    fn no_thinking_suppresses_reasoning_lines() {
        let (out, _) = run(HAPPY_PATH, opts(false, true));
        assert!(!out.contains("[codex thinking]"));
        assert!(out.contains("[codex ran]"));
        assert!(out.contains("Found a race condition"));
    }

    #[test]
    fn no_tool_calls_suppresses_command_execution() {
        let (out, _) = run(HAPPY_PATH, opts(true, false));
        assert!(out.contains("[codex thinking]"));
        assert!(!out.contains("[codex ran]"));
        assert!(out.contains("Found a race condition"));
    }

    #[test]
    fn missing_turn_completed_emits_stderr_warning() {
        let input = r#"{"type":"thread.started","thread_id":"thr_abc"}
{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}
"#;
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("partial"));
        assert!(err.contains("no turn.completed"));
    }

    #[test]
    fn unknown_event_types_are_silently_skipped() {
        let input = r#"{"type":"some.future.event","data":{"foo":"bar"}}
{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("hi"));
        assert!(out.contains("tokens: 2"));
        assert!(!out.contains("some.future.event"));
        assert!(err.is_empty());
    }

    #[test]
    fn malformed_json_lines_are_skipped() {
        let input = r#"this is not json
{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}
{ broken json
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("hi"));
        assert!(out.contains("tokens: 2"));
        assert!(err.is_empty());
    }

    #[test]
    fn missing_usage_fields_default_to_zero_tokens() {
        let input = r#"{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}
{"type":"turn.completed"}
"#;
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("hi"));
        assert!(!out.contains("tokens:"));
    }

    #[test]
    fn zero_token_usage_with_both_fields_present_emits_no_tokens_line() {
        let input = r#"{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}
{"type":"turn.completed","usage":{"input_tokens":0,"output_tokens":0}}
"#;
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("hi"));
        assert!(!out.contains("tokens:"));
    }

    #[test]
    fn empty_text_in_reasoning_does_not_emit() {
        let input = r#"{"type":"item.completed","item":{"type":"reasoning","text":""}}
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let (out, _) = run(input, opts(true, true));
        assert!(!out.contains("[codex thinking]"));
    }

    #[test]
    fn empty_input_does_not_warn() {
        let (out, err) = run("", opts(true, true));
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn whitespace_only_input_does_not_warn() {
        let (out, err) = run("\n\n  \n", opts(true, true));
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi\"}}\r\n\
                     {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\r\n";
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("hi"));
        assert!(out.contains("tokens: 2"));
        assert!(err.is_empty());
    }

    #[test]
    fn invalid_utf8_does_not_drop_surrounding_events() {
        // Line 1 has invalid UTF-8 (0xff 0xfe) embedded in a non-JSON line; lines 2 and 3 are valid.
        let mut input: Vec<u8> = Vec::new();
        input.extend_from_slice(b"garbage \xff\xfe more garbage\n");
        input.extend_from_slice(
            b"{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"survived\"}}\n",
        );
        input.extend_from_slice(
            b"{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n",
        );
        let (out, err) = run_bytes(&input, opts(true, true));
        assert!(out.contains("survived"));
        assert!(out.contains("tokens: 2"));
        assert!(err.is_empty());
    }

    #[test]
    fn token_overflow_saturates_instead_of_panicking() {
        let input = format!(
            "{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"x\"}}}}\n\
             {{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":{m},\"output_tokens\":{m}}}}}\n",
            m = u64::MAX
        );
        let (out, _) = run(&input, opts(true, true));
        assert!(out.contains(&format!("tokens: {}", u64::MAX)));
    }

    /// Writer that returns BrokenPipe after `n` successful writes.
    struct BrokenPipeAfter {
        remaining: usize,
        written: Vec<u8>,
    }

    impl Write for BrokenPipeAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(ErrorKind::BrokenPipe, "downstream closed"));
            }
            self.remaining -= 1;
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_on_stdout_exits_cleanly() {
        let out = BrokenPipeAfter {
            remaining: 1,
            written: Vec::new(),
        };
        let mut err = Vec::new();
        let result = parse_stream(HAPPY_PATH.as_bytes(), out, &mut err, opts(true, true));
        assert!(
            result.is_ok(),
            "broken pipe on stdout should be a clean exit, got {:?}",
            result
        );
    }
}
