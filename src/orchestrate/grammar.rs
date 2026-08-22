//! The single source of grammar truth for `when:` guards.
//!
//! `LEAF_FORMS` is read by exactly two things: the recursive-descent parser
//! in `expr.rs` (which matches on `LeafKind` exhaustively — adding a row
//! here without teaching the parser about it is a compile error) and the
//! doc generator below, which regenerates the table vendored into
//! `claude/skills/_thoughts/orchestration-runtime.md`. Removing a row from
//! the parser without removing it here breaks `tests/orchestration_grammar_doc.rs`.

use colored::Colorize;
use serde_json::{Value, json};

/// Discriminant for a `when:` leaf. The parser matches on this
/// exhaustively, so adding a row to `LEAF_FORMS` without teaching the
/// parser about it is a compile error. That is the anti-drift mechanism —
/// do not replace the match with a `_ =>` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafKind {
    Comparison,  // <path> == <value> / !=
    Exists,      // exists(<thing>)
    Matches,     // matches(<field>, <pattern>)
    Flag,        // flag(<name>)
    Available,   // available(<binary>)
    Count,       // count(<thing>) <op> <n>
    Exit0,       // exit0(<command>)
    Composition, // <expr> and/or/not <expr> — documented, not a leaf
}

impl LeafKind {
    fn as_str(self) -> &'static str {
        match self {
            LeafKind::Comparison => "comparison",
            LeafKind::Exists => "exists",
            LeafKind::Matches => "matches",
            LeafKind::Flag => "flag",
            LeafKind::Available => "available",
            LeafKind::Count => "count",
            LeafKind::Exit0 => "exit0",
            LeafKind::Composition => "composition",
        }
    }
}

/// One row of the grammar table. `syntax` and `example` are pre-rendered
/// markdown (backticks included where the doc table wants them — the
/// comparison row needs two separate code spans in one cell); JSON output
/// strips the backticks back out rather than a second copy of the text.
pub struct LeafForm {
    pub kind: LeafKind,
    pub syntax: &'static str,
    pub true_when: &'static str,
    pub example: &'static str,
}

pub const LEAF_FORMS: [LeafForm; 8] = [
    LeafForm {
        kind: LeafKind::Comparison,
        syntax: "`<path> == <value>` / `!=`",
        true_when: "Literal comparison against known state",
        example: "`backend == git`",
    },
    LeafForm {
        kind: LeafKind::Exists,
        syntax: "`exists(<thing>)`",
        true_when: "The named thing is present",
        example: "`exists(plan.phases)`",
    },
    LeafForm {
        kind: LeafKind::Matches,
        syntax: "`matches(<field>, <pattern>)`",
        true_when: "Regex match against an input",
        example: "`matches(request, \"ENG-\\d+\")`",
    },
    LeafForm {
        kind: LeafKind::Flag,
        syntax: "`flag(<name>)`",
        true_when: "The invocation carried that flag",
        example: "`flag(--claude)`",
    },
    LeafForm {
        kind: LeafKind::Available,
        syntax: "`available(<binary>)`",
        true_when: "Resolvable on PATH",
        example: "`available(codex)`",
    },
    LeafForm {
        kind: LeafKind::Count,
        syntax: "`count(<thing>) <op> <n>`",
        true_when: "Numeric comparison",
        example: "`count(findings) > 0`",
    },
    LeafForm {
        kind: LeafKind::Exit0,
        syntax: "`exit0(<command>)`",
        true_when: "The command exits 0",
        example: "`exit0(git diff --quiet)`",
    },
    LeafForm {
        kind: LeafKind::Composition,
        syntax: "`<expr> and/or/not <expr>`",
        true_when: "Boolean composition",
        example: "`flag(--codex) and not available(codex)`",
    },
];

/// Documented once, next to the grammar it governs.
pub const PRECEDENCE_NOTE: &str = "Precedence: `not` binds tightest, then `and`, then `or`; \
     `and`/`or` are left-associative, so `a and b or c` parses as `(a and b) or c`. \
     Parentheses are permitted and override this.";

/// The markdown table plus the precedence note, byte-for-byte what
/// `claude/skills/_thoughts/orchestration-runtime.md`'s generated region
/// holds. Exactly 10 table lines (header, separator, 8 rows), a blank
/// line, then the precedence note — each ending in `\n`.
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("| Form | True when | Example |\n");
    out.push_str("|---|---|---|\n");
    for form in &LEAF_FORMS {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            form.syntax, form.true_when, form.example
        ));
    }
    out.push('\n');
    out.push_str(PRECEDENCE_NOTE);
    out.push('\n');
    out
}

/// The machine-readable grammar description the desktop app binds to.
pub fn render_json() -> Value {
    let forms: Vec<Value> = LEAF_FORMS
        .iter()
        .map(|f| {
            json!({
                "kind": f.kind.as_str(),
                "syntax": f.syntax.replace('`', ""),
                "trueWhen": f.true_when,
                "example": f.example.replace('`', ""),
            })
        })
        .collect();
    json!({
        "version": 1,
        "precedence": ["not", "and", "or"],
        "associativity": "left",
        "forms": forms,
    })
}

/// Colored terminal listing for `hyprlayer orchestrate grammar` with no
/// flags — a human reading the grammar at a glance, not a machine consumer.
pub fn render_human() {
    println!("{}", "The `when:` guard grammar".yellow().bold());
    println!();
    for form in &LEAF_FORMS {
        let plain_syntax = form.syntax.replace('`', "");
        let plain_example = form.example.replace('`', "");
        println!("  {}", plain_syntax.cyan());
        println!("    {}", form.true_when);
        println!("    {} {}", "e.g.".bright_black(), plain_example.green());
        println!();
    }
    println!("{}", PRECEDENCE_NOTE.replace('`', "").bright_black());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markdown_emits_exactly_ten_table_lines_plus_the_precedence_note() {
        let md = render_markdown();
        let lines: Vec<&str> = md.lines().collect();
        // 10 table lines (header + separator + 8 rows), 1 blank line, 1
        // precedence line = 12 logical lines.
        assert_eq!(lines.len(), 12, "unexpected line count in:\n{md}");
        assert_eq!(lines[0], "| Form | True when | Example |");
        assert_eq!(lines[1], "|---|---|---|");
        assert_eq!(lines[10], "");
        assert!(lines[11].starts_with("Precedence:"));
    }

    #[test]
    fn render_json_has_eight_forms_and_the_precedence_metadata() {
        let json = render_json();
        assert_eq!(json["version"], 1);
        assert_eq!(json["forms"].as_array().unwrap().len(), 8);
        assert_eq!(json["precedence"], json!(["not", "and", "or"]));
        assert_eq!(json["associativity"], "left");
        // JSON values strip markdown backticks — machine consumers don't want them.
        assert_eq!(json["forms"][0]["syntax"], "<path> == <value> / !=");
    }
}
