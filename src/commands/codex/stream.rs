// Parser for OpenAI Codex CLI --json (JSONL) output.
//
// Tested against codex-cli 0.128.0.
// Event mapping is stable across recent versions but OpenAI may evolve it.
// All field access uses .get().and_then() chains, so missing fields produce
// silent skips rather than panics.

use std::borrow::Cow;
use std::io::{self, BufRead, ErrorKind, Read, Write};

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::cli::CodexStreamArgs;

// 8 MiB cap per JSONL line. Codex events are well under 1 MiB in practice;
// this is a safety net against a stuck producer or malformed input that omits
// newlines and would otherwise grow the buffer until OOM.
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

// After processing an oversized line we shrink the read buffer back to this
// size so a single 8 MiB pathological event does not pin that capacity for
// the rest of the run.
const POST_OVERSIZE_CAPACITY: usize = 64 * 1024;

pub fn stream(args: CodexStreamArgs) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let failed = parse_stream(
        stdin.lock(),
        stdout.lock(),
        stderr.lock(),
        StreamOpts {
            include_thinking: !args.no_thinking,
            include_tool_calls: !args.no_tool_calls,
        },
    )?;
    if failed {
        // Non-zero exit so shell pipelines can detect failure via $? /
        // $PIPESTATUS without inspecting stderr.
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        std::process::exit(1);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct StreamOpts {
    pub include_thinking: bool,
    pub include_tool_calls: bool,
}

// On a broken pipe (downstream closed), set $broken and break out of the read
// loop. The loop exits cleanly so the returned failure flag still reflects any
// turn.failed observed before the pipe closed.
macro_rules! tryw {
    ($out:expr, $broken:ident, $($arg:tt)*) => {
        match writeln!($out, $($arg)*) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe => { $broken = true; break; }
            Err(e) => return Err(e.into()),
        }
    };
}

// Codex JSONL events we care about. Internally tagged on the "type" field;
// unknown event types (codex emits many incremental delta variants) fall into
// `Unknown` so we never pay for parsing their payload. Borrowed string fields
// avoid allocation when the JSON value contains no escapes.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum Event<'a> {
    #[serde(rename = "thread.started")]
    ThreadStarted {
        #[serde(borrow, default)]
        thread_id: Option<Cow<'a, str>>,
    },
    #[serde(rename = "item.completed")]
    ItemCompleted {
        #[serde(borrow, default)]
        item: Option<EventItem<'a>>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        #[serde(default)]
        usage: Option<Usage>,
    },
    #[serde(rename = "turn.failed")]
    TurnFailed {
        // Permissive: codex has shipped both `error` and the top-level
        // `message` as a string, an object, a number, or absent. Accept any
        // JSON value and probe inside `extract_failure_message`. Strict typing
        // here would silently drop the entire failure event — and the non-zero
        // exit it implies — when codex emits an unexpected shape.
        #[serde(default)]
        error: Option<Value>,
        #[serde(default)]
        message: Option<Value>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct EventItem<'a> {
    #[serde(rename = "type", borrow)]
    ty: Cow<'a, str>,
    #[serde(borrow, default)]
    text: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    command: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

// `turn.failed` schema has drifted across codex versions. Probe the two
// observed shapes for `error`, then fall back to a top-level `message` field.
// Both inputs are `Option<&Value>` (not `Option<&str>`) so a non-string field
// just returns `None` instead of failing deserialization upstream.
fn extract_failure_message<'a>(
    error: Option<&'a Value>,
    message: Option<&'a Value>,
) -> Option<Cow<'a, str>> {
    if let Some(e) = error {
        if let Some(m) = e.get("message").and_then(|v| v.as_str()) {
            return Some(Cow::Borrowed(m));
        }
        if let Some(s) = e.as_str() {
            return Some(Cow::Borrowed(s));
        }
    }
    message.and_then(|m| m.as_str()).map(Cow::Borrowed)
}

// Read up to and including the next `\n` from `reader` into `buf`, capped at
// `max` bytes. Returns `(bytes_in_buf, truncated)`. When `truncated`, the rest
// of the overlong line is consumed via `skip_until` (no copies, no allocation)
// and `buf` is shrunk so an 8 MiB pathological event does not pin that
// capacity for the rest of the run.
//
// "Truncated" means we hit the byte cap before finding a newline. A line that
// reaches EOF without a trailing newline is **not** truncated — codex's final
// JSONL record is sometimes flushed without a newline, and dropping it would
// silently swallow a final `turn.failed` or `turn.completed`.
fn read_line_capped<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: u64,
) -> io::Result<(usize, bool)> {
    buf.clear();
    let (n, hit_cap) = {
        let mut take = reader.by_ref().take(max);
        let n = take.read_until(b'\n', buf)?;
        (n, take.limit() == 0)
    };
    if n == 0 {
        return Ok((0, false));
    }
    // Truncated only when we exhausted the cap AND no newline was found within
    // it. A trailing newline (even at the exact cap byte) means a complete line.
    let truncated = hit_cap && !buf.ends_with(b"\n");
    if truncated {
        reader.skip_until(b'\n')?;
        buf.clear();
        buf.shrink_to(POST_OVERSIZE_CAPACITY);
    }
    Ok((buf.len(), truncated))
}

// Strip ANSI/OSC escape sequences and most C0 control characters from text
// before printing. Codex output passes through model responses, which a hostile
// repository could craft to contain terminal escapes (clear screen, redefine
// title, OSC 52 clipboard write, hyperlink spoofing, etc.).
//
// Preserved: tab (\t), newline (\n), carriage return (\r), and printable text.
// Stripped: ESC[…] CSI sequences, ESC]…BEL/ESC\ OSC sequences, lone ESC,
// other C0 control bytes (0x00-0x1f minus the three above), and DEL (0x7f).
//
// Single-pass: a fast scan locates the first byte that needs handling. The
// clean prefix is appended as one slice, then a byte-level loop handles
// escapes and copies subsequent clean runs as slices (no `chars()` decoding).
fn sanitize_text(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    let Some(first) = bytes.iter().position(|&b| needs_sanitize(b)) else {
        return Cow::Borrowed(s);
    };

    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..first]);

    let mut i = first;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x1b => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    // CSI: parameter bytes 0x30-0x3f, intermediate 0x20-0x2f,
                    // final byte 0x40-0x7e. Skip up to and including the final.
                    b'[' => {
                        i += 1;
                        while i < bytes.len() && !matches!(bytes[i], 0x40..=0x7e) {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1;
                        }
                    }
                    // OSC: terminated by BEL (\x07) or by ST (ESC \).
                    b']' => {
                        i += 1;
                        let mut prev_esc = false;
                        while i < bytes.len() {
                            let c = bytes[i];
                            i += 1;
                            if c == 0x07 || (prev_esc && c == b'\\') {
                                break;
                            }
                            prev_esc = c == 0x1b;
                        }
                    }
                    // Bare ESC + unknown introducer: drop the introducer too.
                    _ => i += 1,
                }
            }
            b'\t' | b'\n' | b'\r' => {
                out.push(b as char);
                i += 1;
            }
            b if b < 0x20 || b == 0x7f => {
                i += 1;
            }
            _ => {
                // Clean run: copy as a slice, no per-char decoding. Splitting
                // at sanitize-bytes is safe because they are all < 0x80, so
                // the boundary is never inside a multi-byte UTF-8 scalar.
                let start = i;
                let advance = bytes[i..]
                    .iter()
                    .position(|&c| needs_sanitize(c))
                    .unwrap_or(bytes.len() - i);
                out.push_str(&s[start..start + advance]);
                i = start + advance;
            }
        }
    }
    Cow::Owned(out)
}

fn needs_sanitize(b: u8) -> bool {
    b == 0x1b || b == 0x7f || (b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r'))
}

/// Returns `true` if any `turn.failed` event was observed in the stream.
pub fn parse_stream<R, O, E>(
    mut reader: R,
    mut out: O,
    mut err: E,
    opts: StreamOpts,
) -> Result<bool>
where
    R: BufRead,
    O: Write,
    E: Write,
{
    let mut saw_any_event = false;
    let mut saw_known_event = false;
    let mut saw_turn_boundary = false;
    let mut saw_failure = false;
    let mut saw_oversize_line = false;
    let mut broken = false;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        let (n, truncated) = read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES)?;
        if n == 0 && !truncated {
            break;
        }
        if truncated {
            saw_oversize_line = true;
            continue;
        }

        // Trim ASCII whitespace at the byte level — JSONL terminators are
        // always ASCII, so no need to decode UTF-8 just to call `str::trim`.
        let trimmed = buf.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }

        // Fast path: input is valid UTF-8 (the overwhelming case for codex
        // output). Borrow the bytes as `&str` and let serde parse zero-copy.
        // Slow path: a stray bad byte inside an otherwise valid JSON envelope
        // would be rejected by `from_slice`, dropping the whole event. Repair
        // via `from_utf8_lossy` so a rogue byte in `agent_message.text`
        // becomes U+FFFD and the human still sees the message.
        let lossy_storage: String;
        let json_str: &str = match std::str::from_utf8(trimmed) {
            Ok(s) => s,
            Err(_) => {
                lossy_storage = String::from_utf8_lossy(trimmed).into_owned();
                &lossy_storage
            }
        };
        let event: Event<'_> = match serde_json::from_str(json_str) {
            Ok(e) => e,
            Err(_) => continue,
        };
        saw_any_event = true;

        match event {
            Event::Unknown => {}
            Event::ThreadStarted { thread_id } => {
                saw_known_event = true;
                if let Some(tid) = thread_id.as_deref() {
                    tryw!(out, broken, "SESSION_ID:{}", sanitize_text(tid));
                }
            }
            Event::ItemCompleted { item: None } => {
                saw_known_event = true;
            }
            Event::ItemCompleted { item: Some(item) } => {
                saw_known_event = true;
                let text = item.text.as_deref().unwrap_or("");
                match item.ty.as_ref() {
                    "reasoning" if opts.include_thinking && !text.is_empty() => {
                        tryw!(out, broken, "[codex thinking] {}", sanitize_text(text));
                        tryw!(out, broken, "");
                    }
                    "agent_message" if !text.is_empty() => {
                        tryw!(out, broken, "{}", sanitize_text(text));
                    }
                    "command_execution" if opts.include_tool_calls => {
                        if let Some(cmd) = item.command.as_deref()
                            && !cmd.is_empty()
                        {
                            tryw!(out, broken, "[codex ran] {}", sanitize_text(cmd));
                        }
                    }
                    _ => {}
                }
            }
            Event::TurnCompleted { usage } => {
                saw_known_event = true;
                saw_turn_boundary = true;
                if let Some(u) = usage {
                    let total = u.input_tokens.saturating_add(u.output_tokens);
                    if total > 0 {
                        tryw!(out, broken, "");
                        tryw!(out, broken, "tokens: {}", total);
                    }
                }
            }
            Event::TurnFailed { error, message } => {
                saw_known_event = true;
                saw_turn_boundary = true;
                saw_failure = true;
                let msg = extract_failure_message(error.as_ref(), message.as_ref());
                let line = match msg {
                    Some(m) => format!("[codex turn.failed] {}", sanitize_text(&m)),
                    None => "[codex turn.failed]".to_string(),
                };
                match writeln!(err, "{}", line) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    // Suppress secondary warnings if we already broke out due to a closed
    // downstream pipe — the user will not see them anyway.
    if !broken {
        if saw_oversize_line {
            let _ = writeln!(
                err,
                "[warning] dropped at least one JSONL line longer than {} bytes",
                MAX_LINE_BYTES
            );
        }
        // Warn when we got input but no recognized event type — likely codex
        // schema drift or a non-JSON producer. Emitting empty output silently
        // is exactly the failure mode the warning is here to surface.
        if saw_any_event && !saw_known_event {
            let _ = writeln!(
                err,
                "[warning] no recognized codex event types in stream — possible schema drift"
            );
        }
        // Only warn about a missing terminator when we processed events but
        // never saw a turn boundary. A fully empty stream stays silent.
        if saw_any_event && !saw_turn_boundary {
            let _ = writeln!(
                err,
                "[warning] no turn.completed event — possible mid-stream disconnect"
            );
        }
    }

    Ok(saw_failure)
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

    fn run_outcome(input: &str, opts: StreamOpts) -> (String, String, bool) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let failed = parse_stream(input.as_bytes(), &mut out, &mut err, opts).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
            failed,
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
    fn happy_path_outcome_is_not_failed() {
        let (_, _, failed) = run_outcome(HAPPY_PATH, opts(true, true));
        assert!(!failed);
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
    fn unknown_event_types_are_silently_skipped_when_known_events_present() {
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

    /// Reader that yields `prefix` bytes once, then fails with the given error
    /// on the next read. Lets us exercise the mid-stream-read-failure path.
    struct FailingReader {
        prefix: Vec<u8>,
        error: Option<io::Error>,
    }

    impl io::Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.prefix.is_empty() {
                let n = self.prefix.len().min(buf.len());
                buf[..n].copy_from_slice(&self.prefix[..n]);
                self.prefix.drain(..n);
                return Ok(n);
            }
            match self.error.take() {
                Some(e) => Err(e),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn stdin_read_error_propagates() {
        // No newline in prefix → read_until consumes the prefix, then hits the error
        // before finding a delimiter. The error must surface as Err, not a silent Ok.
        let reader = io::BufReader::new(FailingReader {
            prefix: b"{\"type\":\"item.completed\"".to_vec(),
            error: Some(io::Error::other("disk gone")),
        });
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = parse_stream(reader, &mut out, &mut err, opts(true, true));
        assert!(
            result.is_err(),
            "stdin read failure must propagate, got {:?}",
            result
        );
    }

    #[test]
    fn turn_failed_emits_error_and_suppresses_disconnect_warning() {
        let input = r#"{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}
{"type":"turn.failed","error":{"message":"rate limited"}}
"#;
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("partial"));
        assert!(
            err.contains("[codex turn.failed] rate limited"),
            "expected failure message on stderr, got: {:?}",
            err
        );
        assert!(
            !err.contains("no turn.completed"),
            "turn.failed should count as a turn boundary, got: {:?}",
            err
        );
    }

    #[test]
    fn turn_failed_with_string_error_field() {
        let input = r#"{"type":"turn.failed","error":"context_length_exceeded"}
"#;
        let (_, err) = run(input, opts(true, true));
        assert!(
            err.contains("[codex turn.failed] context_length_exceeded"),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn turn_failed_with_no_known_message_field_still_prints_marker() {
        let input = r#"{"type":"turn.failed","code":42}
"#;
        let (_, err) = run(input, opts(true, true));
        assert!(err.contains("[codex turn.failed]"), "got: {:?}", err);
        assert!(!err.contains("no turn.completed"), "got: {:?}", err);
    }

    #[test]
    fn turn_failed_sets_failed_outcome() {
        let input = r#"{"type":"item.completed","item":{"type":"agent_message","text":"partial"}}
{"type":"turn.failed","error":{"message":"rate limited"}}
"#;
        let (_, _, failed) = run_outcome(input, opts(true, true));
        assert!(
            failed,
            "turn.failed must set the failed flag so callers can propagate non-zero exit"
        );
    }

    #[test]
    fn turn_completed_after_turn_failed_does_not_clear_failed_flag() {
        // If codex emits a recovery/cleanup turn.completed after turn.failed,
        // the run still failed — the failure flag must not be reset.
        let input = r#"{"type":"turn.failed","error":{"message":"rate limited"}}
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let (_, _, failed) = run_outcome(input, opts(true, true));
        assert!(failed);
    }

    #[test]
    fn schema_drift_warning_when_only_unknown_event_types() {
        // No recognized event types at all — likely codex schema drift.
        let input = r#"{"type":"some.future.event","data":{"foo":"bar"}}
{"type":"another.unknown.event"}
"#;
        let (_, err) = run(input, opts(true, true));
        assert!(
            err.contains("possible schema drift"),
            "expected schema drift warning, got: {:?}",
            err
        );
    }

    #[test]
    fn no_schema_drift_warning_when_at_least_one_known_event_seen() {
        let input = r#"{"type":"some.future.event"}
{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let (_, err) = run(input, opts(true, true));
        assert!(
            !err.contains("schema drift"),
            "schema-drift warning should not fire when known events were seen, got: {:?}",
            err
        );
    }

    #[test]
    fn oversize_line_is_dropped_and_warned() {
        // Build a line larger than MAX_LINE_BYTES with no newline, followed by a
        // valid event and a turn.completed. The oversize line must be discarded
        // (not buffered to OOM), the surrounding events must still parse, and a
        // single warning must fire on stderr.
        let oversize = "a".repeat((MAX_LINE_BYTES as usize) + 1024);
        let input = format!(
            "{oversize}\n\
             {{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"after\"}}}}\n\
             {{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\n"
        );
        let (out, err) = run(&input, opts(true, true));
        assert!(
            out.contains("after"),
            "post-oversize event must still parse"
        );
        assert!(out.contains("tokens: 2"));
        assert!(
            err.contains("dropped at least one JSONL line longer than"),
            "expected oversize-line warning, got: {:?}",
            err
        );
    }

    #[test]
    fn ansi_csi_sequences_are_stripped_from_agent_message() {
        // \u001b in the JSON source parses to U+001B (ESC); the resulting
        // agent_message text contains real CSI sequences for sanitize_text
        // to strip. JSON strings cannot contain raw control bytes, so we
        // must escape them this way.
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"plain \\u001b[31mRED\\u001b[0m end\"}}\n\
                     {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("plain RED end"), "got: {:?}", out);
        assert!(
            !out.contains("\x1b["),
            "ANSI CSI must be stripped, got: {:?}",
            out
        );
    }

    #[test]
    fn ansi_osc_clipboard_sequence_is_stripped() {
        // OSC 52 (clipboard write) is the dangerous one — terminal scrapers can
        // exfiltrate or inject. Must be removed entirely from passthrough.
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"before\\u001b]52;c;cGF5bG9hZA==\\u0007after\"}}\n\
                     {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("beforeafter"), "got: {:?}", out);
        assert!(!out.contains("\x1b]"), "OSC must be stripped");
        assert!(!out.contains("52;"), "OSC parameters must not leak through");
    }

    #[test]
    fn ansi_in_command_execution_is_stripped() {
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"command\":\"echo \\u001b[31mhi\\u001b[0m\"}}\n\
                     {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("[codex ran] echo hi"), "got: {:?}", out);
        assert!(
            !out.contains("\x1b["),
            "ANSI must be stripped from command, got: {:?}",
            out
        );
    }

    #[test]
    fn ansi_in_thread_id_is_stripped() {
        let input =
            "{\"type\":\"thread.started\",\"thread_id\":\"thr_\\u001b]0;evil\\u0007abc\"}\n";
        let (out, _) = run(input, opts(true, true));
        // The session id line must contain only sanitized characters.
        assert!(out.starts_with("SESSION_ID:thr_abc"), "got: {:?}", out);
        assert!(!out.contains("\x1b]"));
        assert!(!out.contains('\x07'));
    }

    #[test]
    fn ansi_in_turn_failed_message_is_stripped() {
        let input =
            "{\"type\":\"turn.failed\",\"error\":{\"message\":\"\\u001b[2Jevil\\u001b[H\"}}\n";
        let (_, err) = run(input, opts(true, true));
        assert!(err.contains("[codex turn.failed] evil"), "got: {:?}", err);
        assert!(
            !err.contains("\x1b["),
            "ANSI must be stripped from failure msg"
        );
    }

    #[test]
    fn lone_c0_control_chars_are_dropped() {
        // BEL (0x07), VT (0x0b), FF (0x0c), and DEL (0x7f) should all vanish.
        // Tab/newline/CR are preserved (newline ends the line so we test \t).
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"a\\u0007b\\u000bc\\td\\u007fe\"}}\n\
                     {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        let (out, _) = run(input, opts(true, true));
        assert!(out.contains("abc\tde"), "got: {:?}", out);
    }

    #[test]
    fn sanitize_text_is_a_noop_for_clean_input() {
        let s = "ordinary text with unicode: 你好 🎉 and tab\there";
        assert_eq!(sanitize_text(s), s);
    }

    #[test]
    fn final_line_without_trailing_newline_still_processed() {
        // Codex sometimes flushes its last JSONL record without a newline.
        // The parser must treat EOF-without-newline as a complete line, not
        // as truncation — otherwise a final turn.failed silently exits 0.
        let input = "{\"type\":\"turn.failed\",\"error\":\"rate limited\"}";
        let (_, err, failed) = run_outcome(input, opts(true, true));
        assert!(
            err.contains("[codex turn.failed] rate limited"),
            "expected failure marker on stderr, got: {:?}",
            err
        );
        assert!(
            !err.contains("dropped at least one JSONL line"),
            "EOF-no-newline must not look like an oversize line, got: {:?}",
            err
        );
        assert!(failed, "turn.failed at EOF must still set the failed flag");
    }

    #[test]
    fn final_agent_message_without_trailing_newline_is_emitted() {
        let input = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final\"}}";
        let (out, err) = run(input, opts(true, true));
        assert!(out.contains("final"), "got: {:?}", out);
        assert!(
            !err.contains("dropped at least one JSONL line"),
            "got: {:?}",
            err
        );
    }

    #[test]
    fn turn_failed_with_non_string_top_level_message_falls_back_to_error_message() {
        // Codex schema drift: top-level `message` field with an unexpected type
        // must not poison the deserialize. The valid `error.message` should
        // still be extracted, the marker emitted, and the failure flag set.
        let input = r#"{"type":"turn.failed","error":{"message":"rate limited"},"message":42}
"#;
        let (_, err, failed) = run_outcome(input, opts(true, true));
        assert!(
            err.contains("[codex turn.failed] rate limited"),
            "expected error.message to be extracted despite wrong-typed top-level message, got: {:?}",
            err
        );
        assert!(failed);
    }

    #[test]
    fn turn_failed_with_object_error_field_no_message_still_marks_failure() {
        // `error` shape we don't recognize (object without `message`) must not
        // drop the failure marker — codex still failed, the user still needs
        // a non-zero exit.
        let input = r#"{"type":"turn.failed","error":{"code":42}}
"#;
        let (_, err, failed) = run_outcome(input, opts(true, true));
        assert!(err.contains("[codex turn.failed]"), "got: {:?}", err);
        assert!(failed);
    }

    #[test]
    fn invalid_utf8_inside_agent_message_falls_back_to_lossy() {
        // Bad byte inside an otherwise-valid JSON envelope. The strict
        // `from_slice` path rejects this, so we must fall back to
        // `from_utf8_lossy` and substitute U+FFFD instead of dropping the
        // event. Surrounding events must still parse.
        let mut input: Vec<u8> = Vec::new();
        input.extend_from_slice(
            b"{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi\xff\xfethere\"}}\n",
        );
        input.extend_from_slice(
            b"{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n",
        );
        let (out, _) = run_bytes(&input, opts(true, true));
        // U+FFFD ("\u{FFFD}") substitutes for each invalid byte.
        assert!(
            out.contains("hi\u{FFFD}\u{FFFD}there") || out.contains("hi\u{FFFD}there"),
            "expected lossy U+FFFD substitution, got: {:?}",
            out
        );
        assert!(out.contains("tokens: 2"));
    }
}
