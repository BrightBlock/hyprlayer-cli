//! Lexer, AST, and a recursive-descent parser for the `when:` grammar
//! documented in `grammar.rs`. This is a real parser, not a regex
//! substitution — see the two prototype bugs it deliberately fixes,
//! tested at the bottom of this file.
//!
//! Grammar (highest precedence first):
//!   or_expr  := and_expr ("or" and_expr)*
//!   and_expr := unary ("and" unary)*
//!   unary    := "not" unary | primary
//!   primary  := "(" or_expr ")" | leaf

use std::fmt;

use crate::orchestrate::grammar::LeafKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Leaf(Leaf),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Leaf {
    Comparison {
        path: String,
        negated: bool,
        value: String,
    },
    Exists(String),
    Matches {
        field: String,
        pattern: String,
    },
    Flag(String),
    Available(String),
    Count {
        thing: String,
        op: CmpOp,
        n: i64,
    },
    Exit0(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

impl CmpOp {
    pub fn apply(self, lhs: i64, rhs: i64) -> bool {
        match self {
            CmpOp::Lt => lhs < rhs,
            CmpOp::Gt => lhs > rhs,
            CmpOp::Le => lhs <= rhs,
            CmpOp::Ge => lhs >= rhs,
            CmpOp::Eq => lhs == rhs,
            CmpOp::Ne => lhs != rhs,
        }
    }
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset into the parsed source. The caller (`block.rs`) adds
    /// this to the step's YAML span to report file:line:col.
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.pos)
    }
}

/// Parses a full `when:` expression. Errs unless the whole input is
/// consumed — this is what rejects juxtaposed leaves with no connective
/// (`exists(a) exists(b)`), which the ported-from Python prototype wrongly
/// accepted.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let mut cur = Cursor::new(src);
    let expr = cur.parse_or()?;
    cur.skip_ws();
    if cur.pos != cur.src.len() {
        let rest = cur.rest();
        let preview: String = rest.chars().take(24).collect();
        return Err(cur.err(format!(
            "unexpected trailing input near {preview:?} — leaves must be joined with `and`/`or`"
        )));
    }
    Ok(expr)
}

/// Walks `expr`, appending every leaf found (in left-to-right order).
/// Used by `facts::build` to find every `matches()` field to bind to
/// `--request` and every `available()`/`exit0()` leaf worth probing,
/// without probing guards the block doesn't actually use.
pub fn collect_leaves<'a>(expr: &'a Expr, out: &mut Vec<&'a Leaf>) {
    match expr {
        Expr::Leaf(l) => out.push(l),
        Expr::Not(inner) => collect_leaves(inner, out),
        Expr::And(a, b) | Expr::Or(a, b) => {
            collect_leaves(a, out);
            collect_leaves(b, out);
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

fn is_value_char(c: char) -> bool {
    is_ident_char(c) || c == '/'
}

#[derive(Clone, Copy)]
struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            pos: self.pos,
            message: message.into(),
        }
    }

    /// Consumes a reserved connective keyword (`and`/`or`/`not`) at the
    /// current position, requiring a word boundary after it. Rolls back
    /// and returns `false` on any mismatch, including a bare prefix match
    /// (`android` must not consume `and`).
    fn try_keyword(&mut self, kw: &str) -> bool {
        let save = *self;
        self.skip_ws();
        if let Some(after) = self.rest().strip_prefix(kw) {
            let boundary = after.chars().next().is_none_or(|c| !is_ident_char(c));
            if boundary {
                self.pos += kw.len();
                return true;
            }
        }
        *self = save;
        false
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        let mut probe = *self;
        probe.try_keyword(kw)
    }

    fn read_run(&mut self, pred: impl Fn(char) -> bool) -> Option<&'a str> {
        self.skip_ws();
        let start = self.pos;
        let rest = self.rest();
        let len = rest
            .char_indices()
            .take_while(|&(_, c)| pred(c))
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        if len == 0 {
            return None;
        }
        self.pos += len;
        Some(&self.src[start..start + len])
    }

    fn read_ident(&mut self) -> Option<&'a str> {
        self.read_run(is_ident_char)
    }

    fn read_value(&mut self) -> Option<&'a str> {
        self.read_run(is_value_char)
    }

    /// Reads the opaque argument text between an already-consumed `(` and
    /// its balanced matching `)`, tracking paren depth so an argument
    /// containing nested parens (or, as with `exit0`, unrelated braces)
    /// still stops at the right place.
    fn read_balanced_arg(&mut self) -> Result<&'a str, ParseError> {
        let start = self.pos;
        let mut depth: usize = 1;
        let bytes = self.src.as_bytes();
        let mut i = self.pos;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let arg = self.src[start..i].trim();
                        self.pos = i + 1;
                        return Ok(arg);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Err(self.err("unterminated '(' — missing a matching ')'"))
    }

    /// Reads a double-quoted string, honoring `\"` and `\\` as escapes.
    /// Any other `\X` sequence is kept literal (`\d` stays `\d`) so regex
    /// patterns like `"ENG-\d+"` don't need doubled backslashes.
    fn read_quoted(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        if self.peek_char() != Some('"') {
            return Err(self.err("expected a double-quoted pattern"));
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek_char() {
                None => return Err(self.err("unterminated string literal")),
                Some('"') => {
                    self.pos += 1;
                    break;
                }
                Some('\\') => {
                    self.pos += 1;
                    match self.peek_char() {
                        Some('"') => {
                            out.push('"');
                            self.pos += 1;
                        }
                        Some('\\') => {
                            out.push('\\');
                            self.pos += 1;
                        }
                        _ => out.push('\\'),
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        Ok(out)
    }

    fn read_cmp_op(&mut self) -> Result<CmpOp, ParseError> {
        self.skip_ws();
        let rest = self.rest();
        for (text, op) in [
            ("<=", CmpOp::Le),
            (">=", CmpOp::Ge),
            ("==", CmpOp::Eq),
            ("!=", CmpOp::Ne),
            ("<", CmpOp::Lt),
            (">", CmpOp::Gt),
        ] {
            if rest.starts_with(text) {
                self.pos += text.len();
                return Ok(op);
            }
        }
        Err(self
            .err("count() needs a comparison operator (<, >, <=, >=, ==, !=) after its argument"))
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.try_keyword("or") {
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        while self.try_keyword("and") {
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.try_keyword("not") {
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        for kw in ["and", "or", "not"] {
            if self.peek_keyword(kw) {
                return Err(self.err(format!(
                    "expected an expression, found the connective `{kw}` — \
                     check for a doubled connective or a missing leaf"
                )));
            }
        }
        if self.peek_char() == Some('(') {
            self.pos += 1;
            let inner = self.parse_or()?;
            self.skip_ws();
            if self.peek_char() != Some(')') {
                return Err(self.err("missing closing ')'"));
            }
            self.pos += 1;
            return Ok(inner);
        }
        if self.peek_char().is_none() {
            return Err(self.err("expected an expression, found end of input"));
        }
        Ok(Expr::Leaf(self.parse_leaf()?))
    }

    fn parse_leaf(&mut self) -> Result<Leaf, ParseError> {
        let head_start = {
            self.skip_ws();
            self.pos
        };
        let head = self
            .read_ident()
            .ok_or_else(|| self.err("expected an expression (a leaf form or a path)"))?;
        let head = head.to_string();

        if self.peek_char() == Some('(') {
            self.pos += 1;
            return self.parse_leaf_call(&head, head_start);
        }

        // Comparison: <path> == <value>  /  <path> != <value>
        self.skip_ws();
        let negated = if self.rest().starts_with("==") {
            self.pos += 2;
            false
        } else if self.rest().starts_with("!=") {
            self.pos += 2;
            true
        } else {
            return Err(self.err(format!(
                "expected '==' or '!=' after `{head}`, or '(' to start a leaf call"
            )));
        };
        let value = self
            .read_value()
            .ok_or_else(|| self.err("expected a value after the comparison operator"))?;
        Ok(Leaf::Comparison {
            path: head,
            negated,
            value: value.to_string(),
        })
    }

    /// Dispatches on `LeafKind`, not on `head` directly, through an
    /// exhaustive match. That is the anti-drift mechanism described in
    /// `grammar.rs`: add a variant to `LeafKind` (and a `LEAF_FORMS` row)
    /// without adding an arm here, and this fails to compile.
    fn parse_leaf_call(&mut self, head: &str, head_start: usize) -> Result<Leaf, ParseError> {
        let Some(kind) = leaf_kind_for_head(head) else {
            return Err(ParseError {
                pos: head_start,
                message: format!(
                    "unknown leaf form `{head}(...)` — not one of exists/matches/flag/available/count/exit0"
                ),
            });
        };
        match kind {
            LeafKind::Exists => {
                let arg = self.read_balanced_arg()?;
                if arg.is_empty() {
                    return Err(self.err("exists() needs a non-empty argument"));
                }
                Ok(Leaf::Exists(arg.to_string()))
            }
            LeafKind::Flag => {
                let arg = self.read_balanced_arg()?;
                if arg.is_empty() {
                    return Err(self.err("flag() needs a non-empty argument"));
                }
                Ok(Leaf::Flag(arg.to_string()))
            }
            LeafKind::Available => {
                let arg = self.read_balanced_arg()?;
                if arg.is_empty() {
                    return Err(self.err("available() needs a non-empty argument"));
                }
                Ok(Leaf::Available(arg.to_string()))
            }
            LeafKind::Exit0 => {
                let arg = self.read_balanced_arg()?;
                if arg.is_empty() {
                    return Err(self.err("exit0() needs a non-empty command"));
                }
                Ok(Leaf::Exit0(arg.to_string()))
            }
            LeafKind::Matches => {
                self.skip_ws();
                let field = self
                    .read_ident()
                    .ok_or_else(|| self.err("matches() needs a field name"))?
                    .to_string();
                self.skip_ws();
                if self.peek_char() != Some(',') {
                    return Err(self.err("matches() needs a ',' between the field and the pattern"));
                }
                self.pos += 1;
                let pattern = self.read_quoted()?;
                self.skip_ws();
                if self.peek_char() != Some(')') {
                    return Err(self.err("matches() is missing its closing ')'"));
                }
                self.pos += 1;
                regex_lite::Regex::new(&pattern).map_err(|e| {
                    self.err(format!("matches() pattern is not a valid regex: {e}"))
                })?;
                Ok(Leaf::Matches { field, pattern })
            }
            LeafKind::Count => {
                let thing = self.read_balanced_arg()?;
                if thing.is_empty() {
                    return Err(self.err("count() needs a non-empty argument"));
                }
                let thing = thing.to_string();
                let op = self.read_cmp_op()?;
                self.skip_ws();
                let num_start = self.pos;
                let num = self
                    .read_ident()
                    .ok_or_else(|| self.err("count() comparison needs an integer"))?;
                let n: i64 = num.parse().map_err(|_| ParseError {
                    pos: num_start,
                    message: format!("count() comparison operand {num:?} is not an integer"),
                })?;
                Ok(Leaf::Count { thing, op, n })
            }
            LeafKind::Comparison => unreachable!(
                "LeafKind::Comparison never reaches parse_leaf_call: parse_leaf only calls \
                 it after seeing '(' immediately following the head, and the comparison form \
                 (`<path> == <value>`) never has one there"
            ),
            LeafKind::Composition => unreachable!(
                "LeafKind::Composition (and/or/not) is parsed by parse_or/parse_and/parse_unary, \
                 never inside a single leaf call"
            ),
        }
    }
}

/// Maps a leaf call's head identifier to its `LeafKind`, for the six
/// function-call forms. `None` for anything else (including a plain path
/// about to be read as a comparison) — that part is inherently a runtime
/// lookup, not something a type system can close; the compile-time
/// guarantee lives in `parse_leaf_call`'s exhaustive match above.
fn leaf_kind_for_head(head: &str) -> Option<LeafKind> {
    match head {
        "exists" => Some(LeafKind::Exists),
        "matches" => Some(LeafKind::Matches),
        "flag" => Some(LeafKind::Flag),
        "available" => Some(LeafKind::Available),
        "count" => Some(LeafKind::Count),
        "exit0" => Some(LeafKind::Exit0),
        _ => None,
    }
}

fn escape_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

impl fmt::Display for Leaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Leaf::Comparison {
                path,
                negated,
                value,
            } => {
                let op = if *negated { "!=" } else { "==" };
                write!(f, "{path} {op} {value}")
            }
            Leaf::Exists(thing) => write!(f, "exists({thing})"),
            Leaf::Matches { field, pattern } => {
                write!(f, "matches({field}, \"{}\")", escape_pattern(pattern))
            }
            Leaf::Flag(name) => write!(f, "flag({name})"),
            Leaf::Available(bin) => write!(f, "available({bin})"),
            Leaf::Count { thing, op, n } => write!(f, "count({thing}) {op} {n}"),
            Leaf::Exit0(cmd) => write!(f, "exit0({cmd})"),
        }
    }
}

/// Precedence used only to decide whether `Display` needs to wrap an
/// operand in parens to round-trip correctly: leaf(3) > not(2) > and(1) >
/// or(0).
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Leaf(_) => 3,
        Expr::Not(_) => 2,
        Expr::And(_, _) => 1,
        Expr::Or(_, _) => 0,
    }
}

fn fmt_operand(e: &Expr, min_prec: u8) -> String {
    if prec(e) < min_prec {
        format!("({e})")
    } else {
        format!("{e}")
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Leaf(l) => write!(f, "{l}"),
            Expr::Not(inner) => write!(f, "not {}", fmt_operand(inner, 2)),
            Expr::And(a, b) => write!(f, "{} and {}", fmt_operand(a, 1), fmt_operand(b, 2)),
            Expr::Or(a, b) => write!(f, "{} or {}", fmt_operand(a, 0), fmt_operand(b, 1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn juxtaposed_leaves_without_a_connective_are_rejected() {
        // The Python prototype PASSED this: LEAF.sub("|") leaves an empty
        // token that `if tok.strip()` skips. lint-orchestration.py:68-75.
        assert!(parse("exists(a) exists(b)").is_err());
    }

    #[test]
    fn a_doubled_connective_is_rejected() {
        // The Python prototype PASSED this too: the `+` in its CONNECTIVE
        // regex (lint-orchestration.py:43) admits repeats.
        assert!(parse("exists(a) and and exists(b)").is_err());
    }

    #[test]
    fn and_not_is_legal_because_not_is_a_unary_prefix() {
        parse(r#"matches(request, "[A-Z]{2,}-\d+") and not matches(request, "(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+")"#)
            .unwrap();
        parse("exists(new-user-message) and not exists(new-research-question)").unwrap();
    }

    #[test]
    fn and_binds_tighter_than_or_and_both_are_left_associative() {
        let a = parse("exists(a) and exists(b) or exists(c)").unwrap();
        let b = parse("(exists(a) and exists(b)) or exists(c)").unwrap();
        assert_eq!(a.to_string(), b.to_string());
        assert!(matches!(a, Expr::Or(l, _) if matches!(*l, Expr::And(_, _))));
    }

    #[test]
    fn a_matches_pattern_containing_a_close_paren_lexes_whole() {
        let expr = parse(r#"matches(request, "(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+")"#).unwrap();
        match expr {
            Expr::Leaf(Leaf::Matches { pattern, .. }) => {
                assert_eq!(pattern, r"(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+");
            }
            other => panic!("expected a Matches leaf, got {other:?}"),
        }
    }

    #[test]
    fn exit0_scans_to_the_balanced_close_paren() {
        let expr = parse("exit0(git merge-base --is-ancestor HEAD @{u})").unwrap();
        match expr {
            Expr::Leaf(Leaf::Exit0(cmd)) => {
                assert_eq!(cmd, "git merge-base --is-ancestor HEAD @{u}");
            }
            other => panic!("expected an Exit0 leaf, got {other:?}"),
        }
    }

    #[test]
    fn off_grammar_text_is_rejected() {
        assert!(parse("totally bogus text").is_err());
    }

    #[test]
    fn the_prototypes_correctly_caught_cases_stay_rejected() {
        assert!(parse("exists(a) BANANA exists(b)").is_err());
    }

    #[test]
    fn canonical_display_round_trips_through_the_parser() {
        let src = "flag(--codex)   and   not available(codex)";
        let parsed = parse(src).unwrap();
        let canonical = parsed.to_string();
        assert_eq!(canonical, "flag(--codex) and not available(codex)");
        let reparsed = parse(&canonical).unwrap();
        assert_eq!(reparsed.to_string(), canonical);
    }

    #[test]
    fn count_leaf_parses_operator_and_integer() {
        let expr = parse("count(findings) > 0").unwrap();
        match expr {
            Expr::Leaf(Leaf::Count { thing, op, n }) => {
                assert_eq!(thing, "findings");
                assert_eq!(op, CmpOp::Gt);
                assert_eq!(n, 0);
            }
            other => panic!("expected a Count leaf, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_regex_pattern_is_a_parse_error() {
        assert!(parse(r#"matches(request, "[unterminated")"#).is_err());
    }

    #[test]
    fn parse_error_position_points_at_the_offending_byte() {
        let err = parse("exists(a) exists(b)").unwrap_err();
        assert_eq!(err.pos, 10);
    }
}
