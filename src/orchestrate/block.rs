//! Fenced-block extraction and the step model. Locates the first
//! ` ```yaml ` fence in a skill file, parses it with `saphyr`'s marked
//! tree, and hand-walks `orchestration.steps` into a typed `Block` whose
//! every field carries a file-absolute `Pos` for `check`'s findings.

use saphyr::{LoadableYamlNode, MarkedYaml};

/// 1-based file line/column, already offset by the fence's position —
/// every `Pos` in a `Block` is file-absolute, never fence-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentRef {
    One(String),
    OneOf(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct WhenExamples {
    pub pos: Pos,
    pub match_: Vec<String>,
    pub no_match: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GivenEntry {
    pub pos: Pos,
    pub has_src: bool,
    /// A human-legible rendering of the raw entry, for the "has no `src:`"
    /// message — not meant to be a faithful re-serialization.
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct Retry {
    pub pos: Pos,
    pub step: Option<Spanned<String>>,
    pub max_is_integer: bool,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub id: Option<String>,
    /// The step mapping's own position — used when there's no more
    /// specific field to blame (a missing `id`, a bad `fanout:`).
    pub pos: Pos,
    pub requires: Vec<Spanned<String>>,
    pub agent: Option<Spanned<AgentRef>>,
    pub fanout: Option<Spanned<AgentRef>>,
    pub over: Option<Spanned<String>>,
    pub when: Option<Spanned<String>>,
    pub when_examples: Option<WhenExamples>,
    /// A guard that requires taste. Unlike `when:` it is never evaluated —
    /// `compile` records it as an unresolved decision rather than deciding.
    pub judgment: Option<Spanned<String>>,
    pub reject: Option<Spanned<String>>,
    pub given: Vec<GivenEntry>,
    pub retry: Option<Retry>,
    pub inline: bool,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub enum BlockError {
    NoFence,
    Yaml(String),
    NotAMapping,
    NoSteps,
}

impl BlockError {
    pub fn message(&self) -> String {
        match self {
            BlockError::NoFence => "no ```yaml block found".to_string(),
            BlockError::Yaml(msg) => format!("YAML parse error: {msg}"),
            BlockError::NotAMapping => "no `orchestration:` mapping".to_string(),
            BlockError::NoSteps => "`orchestration.steps` missing or empty".to_string(),
        }
    }
}

/// Finds the first ` ```yaml `-fenced block in `src`. Returns the fenced
/// content and the 1-based file line the ` ```yaml ` marker itself sits
/// on. Ports the prototype's first non-greedy match
/// (`lint-orchestration.py:57-65`).
fn extract_fence(src: &str) -> Option<(&str, usize)> {
    const START: &str = "```yaml\n";
    const END: &str = "\n```";
    let start = src.find(START)?;
    let content_start = start + START.len();
    let end = src[content_start..].find(END)?;
    let content = &src[content_start..content_start + end];
    let fence_line = src[..start].matches('\n').count() + 1;
    Some((content, fence_line))
}

/// Saphyr spans are relative to the fenced content (content line 1 = file
/// line `fence_line + 1`), so `file_line = fence_line + span_line`. Getting
/// this wrong puts every reported position off by a constant.
fn pos_of(node: &MarkedYaml, fence_line: usize) -> Pos {
    Pos {
        line: fence_line + node.span.start.line(),
        col: node.span.start.col(),
    }
}

fn spanned_str(node: &MarkedYaml, key: &str, fence_line: usize) -> Option<Spanned<String>> {
    let field = node.data.as_mapping_get(key)?;
    let value = field.data.as_str()?;
    Some(Spanned {
        value: value.to_string(),
        pos: pos_of(field, fence_line),
    })
}

/// `one-of [a, b, c]` is a plain scalar (the value doesn't start with `[`,
/// so YAML never treats it as a flow sequence) — split it out here rather
/// than in the caller.
fn parse_agent_ref(raw: &str) -> AgentRef {
    let trimmed = raw.trim();
    match trimmed.strip_prefix("one-of") {
        Some(rest) => {
            let re = regex_lite::Regex::new(r"[\w\-]+").expect("static pattern");
            let names = re.find_iter(rest).map(|m| m.as_str().to_string()).collect();
            AgentRef::OneOf(names)
        }
        None => AgentRef::One(trimmed.to_string()),
    }
}

fn spanned_agent_ref(node: &MarkedYaml, key: &str, fence_line: usize) -> Option<Spanned<AgentRef>> {
    let raw = spanned_str(node, key, fence_line)?;
    Some(Spanned {
        value: parse_agent_ref(&raw.value),
        pos: raw.pos,
    })
}

fn string_seq(node: &MarkedYaml, key: &str) -> Vec<String> {
    let Some(field) = node.data.as_mapping_get(key) else {
        return Vec::new();
    };
    let Some(seq) = field.data.as_sequence() else {
        return Vec::new();
    };
    seq.iter()
        .filter_map(|n| n.data.as_str().map(str::to_string))
        .collect()
}

fn spanned_str_list(node: &MarkedYaml, key: &str, fence_line: usize) -> Vec<Spanned<String>> {
    let Some(field) = node.data.as_mapping_get(key) else {
        return Vec::new();
    };
    let Some(seq) = field.data.as_sequence() else {
        return Vec::new();
    };
    seq.iter()
        .filter_map(|n| {
            n.data.as_str().map(|s| Spanned {
                value: s.to_string(),
                pos: pos_of(n, fence_line),
            })
        })
        .collect()
}

fn when_examples(node: &MarkedYaml, fence_line: usize) -> Option<WhenExamples> {
    let we = node.data.as_mapping_get("when-examples")?;
    we.data.as_mapping()?;
    Some(WhenExamples {
        pos: pos_of(we, fence_line),
        match_: string_seq(we, "match"),
        no_match: string_seq(we, "no-match"),
    })
}

fn render_scalar_ish(node: &MarkedYaml) -> String {
    if let Some(s) = node.data.as_str() {
        return format!("{s:?}");
    }
    if let Some(map) = node.data.as_mapping() {
        let parts: Vec<String> = map
            .iter()
            .map(|(k, v)| {
                let key = k.data.as_str().unwrap_or("?");
                let val = v.data.as_str().unwrap_or("?");
                format!("{key}: {val}")
            })
            .collect();
        return format!("{{{}}}", parts.join(", "));
    }
    "<value>".to_string()
}

fn given_entries(node: &MarkedYaml, fence_line: usize) -> Vec<GivenEntry> {
    let Some(given) = node.data.as_mapping_get("given") else {
        return Vec::new();
    };
    let Some(seq) = given.data.as_sequence() else {
        return Vec::new();
    };
    seq.iter()
        .map(|entry| {
            let src = entry
                .data
                .as_mapping_get("src")
                .and_then(|n| n.data.as_str());
            GivenEntry {
                pos: pos_of(entry, fence_line),
                has_src: matches!(src, Some(s) if !s.is_empty()),
                raw: render_scalar_ish(entry),
            }
        })
        .collect()
}

fn retry(node: &MarkedYaml, fence_line: usize) -> Option<Retry> {
    let r = node.data.as_mapping_get("retry")?;
    r.data.as_mapping()?;
    let step = r.data.as_mapping_get("step").and_then(|n| {
        n.data.as_str().map(|s| Spanned {
            value: s.to_string(),
            pos: pos_of(n, fence_line),
        })
    });
    let max_is_integer = r
        .data
        .as_mapping_get("max")
        .and_then(|n| n.data.as_integer())
        .is_some();
    Some(Retry {
        pos: pos_of(r, fence_line),
        step,
        max_is_integer,
    })
}

fn parse_step(node: &MarkedYaml, fence_line: usize) -> Step {
    Step {
        id: node
            .data
            .as_mapping_get("id")
            .and_then(|n| n.data.as_str())
            .map(str::to_string),
        pos: pos_of(node, fence_line),
        requires: spanned_str_list(node, "requires", fence_line),
        agent: spanned_agent_ref(node, "agent", fence_line),
        fanout: spanned_agent_ref(node, "fanout", fence_line),
        over: spanned_str(node, "over", fence_line),
        when: spanned_str(node, "when", fence_line),
        when_examples: when_examples(node, fence_line),
        judgment: spanned_str(node, "judgment", fence_line),
        reject: spanned_str(node, "reject", fence_line),
        given: given_entries(node, fence_line),
        retry: retry(node, fence_line),
        inline: node
            .data
            .as_mapping_get("inline")
            .and_then(|n| n.data.as_bool())
            .unwrap_or(false),
    }
}

/// Parses a skill file's `orchestration:` block. Errors here map directly
/// to check 1's findings — see `check::block_error_finding`.
pub fn parse(src: &str) -> Result<Block, BlockError> {
    let (fenced, fence_line) = extract_fence(src).ok_or(BlockError::NoFence)?;

    let docs = MarkedYaml::load_from_str(fenced).map_err(|e| BlockError::Yaml(e.to_string()))?;
    let doc = docs.into_iter().next().ok_or(BlockError::NotAMapping)?;

    let orch = doc
        .data
        .as_mapping_get("orchestration")
        .ok_or(BlockError::NotAMapping)?;
    if orch.data.as_mapping().is_none() {
        return Err(BlockError::NotAMapping);
    }

    let steps_seq = orch
        .data
        .as_mapping_get("steps")
        .and_then(|n| n.data.as_sequence());
    let steps_seq = match steps_seq {
        Some(seq) if !seq.is_empty() => seq,
        _ => return Err(BlockError::NoSteps),
    };

    let steps = steps_seq
        .iter()
        .map(|s| parse_step(s, fence_line))
        .collect();

    Ok(Block { steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(body: &str) -> String {
        format!("---\nname: x\n---\n\n```yaml\n{body}\n```\n")
    }

    #[test]
    fn fence_line_offset_arithmetic_is_file_absolute() {
        // The fence marker sits on line 5; a step declared on the fenced
        // content's own line 3 must report file line 5 + 3 = 8.
        let src =
            "line1\nline2\nline3\nline4\n```yaml\norchestration:\n  steps:\n    - id: a\n```\n";
        let block = parse(src).unwrap();
        assert_eq!(block.steps[0].pos.line, 8);
    }

    #[test]
    fn a_when_mapping_value_is_a_parse_error_with_a_position() {
        let src = skill("orchestration:\n  steps:\n    - id: a\n      when: run: git status\n");
        let err = parse(&src).unwrap_err();
        match err {
            BlockError::Yaml(msg) => assert!(
                msg.to_lowercase()
                    .contains("mapping values are not allowed"),
                "unexpected message: {msg}"
            ),
            other => panic!("expected a Yaml parse error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_step_keys_are_dropped_without_a_finding() {
        let src = skill("orchestration:\n  steps:\n    - id: a\n      totally-made-up-key: 1\n");
        let block = parse(&src).unwrap();
        assert_eq!(block.steps.len(), 1);
        assert_eq!(block.steps[0].id.as_deref(), Some("a"));
    }

    #[test]
    fn one_of_agent_extracts_every_name() {
        let src = skill(
            "orchestration:\n  steps:\n    - id: a\n      agent: one-of [codebase-locator, codebase-analyzer]\n",
        );
        let block = parse(&src).unwrap();
        match &block.steps[0].agent.as_ref().unwrap().value {
            AgentRef::OneOf(names) => {
                assert_eq!(
                    names,
                    &vec![
                        "codebase-locator".to_string(),
                        "codebase-analyzer".to_string()
                    ]
                );
            }
            other => panic!("expected OneOf, got {other:?}"),
        }
    }

    #[test]
    fn given_entry_without_src_is_flagged() {
        let src = skill("orchestration:\n  steps:\n    - id: a\n      given: [just-a-string]\n");
        let block = parse(&src).unwrap();
        assert_eq!(block.steps[0].given.len(), 1);
        assert!(!block.steps[0].given[0].has_src);
    }

    #[test]
    fn no_fence_is_reported_plainly() {
        let src = "# just prose, no orchestration block\n";
        assert!(matches!(parse(src), Err(BlockError::NoFence)));
    }
}
