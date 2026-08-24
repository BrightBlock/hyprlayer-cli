//! Tri-state (Kleene) evaluator over a flat fact environment. No probing
//! happens here — `eval` only ever looks facts up in the `FactEnv` it is
//! given. Probing (PATH lookups, `exit0` execution, config reads) is
//! `compile`'s job (`facts.rs`, Phase 4); `check` builds a `FactEnv`
//! containing only example text (Phase 2) and never executes anything.

use std::collections::BTreeMap;
use std::fmt;

use crate::orchestrate::expr::{Expr, Leaf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    pub fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }

    pub fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }

    pub fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::True, _) | (_, Tri::True) => Tri::True,
            (Tri::False, Tri::False) => Tri::False,
            _ => Tri::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tri::True => "true",
            Tri::False => "false",
            Tri::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Tri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn bool_tri(b: bool) -> Tri {
    if b { Tri::True } else { Tri::False }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FactValue {
    Str(String),
    Bool(bool),
    Int(i64),
}

/// A fact plus where it came from, so the compile plan can show its
/// `via` provenance next to the value.
#[derive(Debug, Clone)]
pub struct FactEntry {
    pub value: FactValue,
    pub via: &'static str,
}

/// A flat `--fact key=value`-shaped environment. Deliberately a
/// `BTreeMap` (not a `HashMap`) so iteration order — and therefore any
/// downstream serialization — is stable across runs.
#[derive(Debug, Clone, Default)]
pub struct FactEnv {
    facts: BTreeMap<String, FactEntry>,
}

impl FactEnv {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key`, but never overwrites an existing entry — callers that
    /// build the environment in decreasing-precedence order (Phase 4's
    /// `facts::build`: `--fact` first, then `--request`, then probes) get
    /// "first write wins" for free.
    pub fn set_if_absent(&mut self, key: impl Into<String>, value: FactValue, via: &'static str) {
        self.facts
            .entry(key.into())
            .or_insert(FactEntry { value, via });
    }

    /// Unconditional set, for callers (like `check`'s example binding)
    /// that build a fresh environment per evaluation and don't need
    /// precedence semantics.
    pub fn set(&mut self, key: impl Into<String>, value: FactValue, via: &'static str) {
        self.facts.insert(key.into(), FactEntry { value, via });
    }

    pub fn get(&self, key: &str) -> Option<&FactEntry> {
        self.facts.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.facts.contains_key(key)
    }
}

/// One resolved leaf, appended to the trace by `eval` regardless of
/// whether the leaf's value ends up mattering to the boolean result —
/// short-circuiting would silently drop leaves from the compile plan's
/// audit trail.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub key: String,
    pub value: Option<FactValue>,
    pub via: &'static str,
}

/// The canonical `--fact` key for a leaf. Shared by `eval` (lookup) and
/// `facts::build` (Phase 4, so `--fact key=value` addresses exactly what
/// `eval` will read).
pub(crate) fn leaf_key(leaf: &Leaf) -> String {
    match leaf {
        Leaf::Comparison { path, .. } => path.clone(),
        Leaf::Matches { field, .. } => field.clone(),
        Leaf::Exists(thing) => format!("exists({thing})"),
        Leaf::Flag(name) => format!("flag({name})"),
        Leaf::Available(bin) => format!("available({bin})"),
        Leaf::Count { thing, .. } => format!("count({thing})"),
        Leaf::Exit0(cmd) => format!("exit0({cmd})"),
    }
}

/// Evaluates `expr` against `env`, appending one `Resolved` per leaf
/// consulted (in left-to-right order, no short-circuiting) so the trace
/// is complete regardless of which leaves ended up dominating the result.
pub fn eval(expr: &Expr, env: &FactEnv, trace: &mut Vec<Resolved>) -> Tri {
    match expr {
        Expr::Not(inner) => eval(inner, env, trace).not(),
        Expr::And(a, b) => {
            let l = eval(a, env, trace);
            let r = eval(b, env, trace);
            l.and(r)
        }
        Expr::Or(a, b) => {
            let l = eval(a, env, trace);
            let r = eval(b, env, trace);
            l.or(r)
        }
        Expr::Leaf(leaf) => eval_leaf(leaf, env, trace),
    }
}

fn eval_leaf(leaf: &Leaf, env: &FactEnv, trace: &mut Vec<Resolved>) -> Tri {
    let key = leaf_key(leaf);
    let entry = env.get(&key);
    let (value, via) = match entry {
        Some(e) => (Some(e.value.clone()), e.via),
        None => (None, "unresolved-default-false"),
    };
    trace.push(Resolved {
        key,
        value: value.clone(),
        via,
    });

    match (leaf, value) {
        (
            Leaf::Comparison {
                negated,
                value: expected,
                ..
            },
            Some(FactValue::Str(actual)),
        ) => {
            let eq = actual == *expected;
            bool_tri(if *negated { !eq } else { eq })
        }
        (Leaf::Matches { pattern, .. }, Some(FactValue::Str(text))) => {
            match regex_lite::Regex::new(pattern) {
                Ok(re) => bool_tri(re.is_match(&text)),
                Err(_) => Tri::Unknown,
            }
        }
        (
            Leaf::Exists(_) | Leaf::Flag(_) | Leaf::Available(_) | Leaf::Exit0(_),
            Some(FactValue::Bool(b)),
        ) => bool_tri(b),
        (Leaf::Count { op, n, .. }, Some(FactValue::Int(actual))) => bool_tri(op.apply(actual, *n)),
        _ => Tri::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrate::expr::parse;

    #[test]
    fn and_truth_table() {
        assert_eq!(Tri::False.and(Tri::Unknown), Tri::False);
        assert_eq!(Tri::Unknown.and(Tri::False), Tri::False);
        assert_eq!(Tri::True.and(Tri::Unknown), Tri::Unknown);
        assert_eq!(Tri::Unknown.and(Tri::True), Tri::Unknown);
        assert_eq!(Tri::True.and(Tri::True), Tri::True);
        assert_eq!(Tri::Unknown.and(Tri::Unknown), Tri::Unknown);
    }

    #[test]
    fn or_truth_table() {
        assert_eq!(Tri::True.or(Tri::Unknown), Tri::True);
        assert_eq!(Tri::Unknown.or(Tri::True), Tri::True);
        assert_eq!(Tri::False.or(Tri::Unknown), Tri::Unknown);
        assert_eq!(Tri::Unknown.or(Tri::False), Tri::Unknown);
        assert_eq!(Tri::False.or(Tri::False), Tri::False);
        assert_eq!(Tri::Unknown.or(Tri::Unknown), Tri::Unknown);
    }

    #[test]
    fn not_unknown_is_unknown() {
        assert_eq!(Tri::Unknown.not(), Tri::Unknown);
        assert_eq!(Tri::True.not(), Tri::False);
        assert_eq!(Tri::False.not(), Tri::True);
    }

    #[test]
    fn an_unresolved_leaf_is_unknown_with_the_default_via() {
        let expr = parse("exists(thing)").unwrap();
        let env = FactEnv::new();
        let mut trace = Vec::new();
        assert_eq!(eval(&expr, &env, &mut trace), Tri::Unknown);
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].key, "exists(thing)");
        assert_eq!(trace[0].via, "unresolved-default-false");
        assert!(trace[0].value.is_none());
    }

    #[test]
    fn a_resolved_comparison_leaf_evaluates_and_records_provenance() {
        let expr = parse("backend == git").unwrap();
        let mut env = FactEnv::new();
        env.set("backend", FactValue::Str("git".to_string()), "fact-flag");
        let mut trace = Vec::new();
        assert_eq!(eval(&expr, &env, &mut trace), Tri::True);
        assert_eq!(trace[0].via, "fact-flag");
    }

    #[test]
    fn every_leaf_is_traced_even_when_and_short_circuits_logically() {
        let expr = parse("exists(a) and exists(b)").unwrap();
        let env = FactEnv::new();
        let mut trace = Vec::new();
        eval(&expr, &env, &mut trace);
        assert_eq!(trace.len(), 2, "both leaves must be recorded: {trace:?}");
    }
}
