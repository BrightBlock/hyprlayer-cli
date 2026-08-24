//! The six mechanical checks a skill's `orchestration:` block must pass,
//! numbered as `orchestration-runtime.md`'s preflight list numbers them.
//! Ports `~/.claude/skills/_thoughts/lint-orchestration.py:101-197`.
//!
//! `check` never executes anything — no `exit0` probing, no PATH lookups,
//! no config reads. The fact environment during `check` contains only
//! each `when-examples:` sample's own text. That is what makes `check`
//! safe to run from a hook or an editor on every keystroke.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::orchestrate::agent_names::{self, AgentSource};
use crate::orchestrate::block::{self, AgentRef, Block, BlockError, GivenEntry, Pos, Spanned};
use crate::orchestrate::eval::{self, FactEnv, FactValue, Tri};
use crate::orchestrate::expr::{self, Expr, Leaf};
use crate::orchestrate::target::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One finding. `check` is `1..=6` for the six numbered checks; `0` marks
/// a finding that isn't one of the six (currently only `reject:`'s
/// grammar warning) — always `Severity::Warning`, so it can never affect
/// the exit code. `target` is `None` for checks 1-5, which are
/// harness-agnostic and must not be reported once per active target; only
/// check 6 findings ever carry `Some(target)` — a structural check-6
/// finding like `fanout:` without `over:` still carries `None`, since it
/// isn't specific to any one harness either.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub check: u8,
    pub target: Option<Target>,
    pub step: Option<String>,
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub file: PathBuf,
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count()
    }
}

pub struct CheckOptions {
    /// Resolve agent names from these directories only, instead of each
    /// active target's own defaults. Repeatable; the caller has already
    /// enforced that this is non-empty only alongside exactly one target.
    pub agents_dir: Vec<PathBuf>,
    /// Harnesses to validate check 6 against. Resolved by the caller —
    /// explicit `--target` flags, or every installed target by default.
    /// Empty means no target is installed anywhere: checks 1-5 still run,
    /// check 6 is skipped with a single target-agnostic warning.
    pub targets: Vec<Target>,
}

/// Reads and checks one file, producing every finding in a single pass.
pub fn check_file(path: &std::path::Path, opts: &CheckOptions) -> Report {
    let mut findings = Vec::new();

    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            findings.push(error(
                1,
                None,
                None,
                format!("could not read {}: {e}", path.display()),
            ));
            return Report {
                file: path.to_path_buf(),
                findings,
            };
        }
    };

    let block = match block::parse(&src) {
        Ok(b) => b,
        Err(e) => {
            findings.push(block_error_finding(&e));
            return Report {
                file: path.to_path_buf(),
                findings,
            };
        }
    };

    check_parses(&block, &mut findings);
    check_requires(&block, &mut findings);
    check_retry(&block, &mut findings);
    check_when(&block, &mut findings);
    check_given(&block, &mut findings);
    check_agents(&block, opts, &mut findings);

    Report {
        file: path.to_path_buf(),
        findings,
    }
}

fn error(check: u8, step: Option<&str>, pos: Option<Pos>, message: String) -> Finding {
    Finding {
        severity: Severity::Error,
        check,
        target: None,
        step: step.map(str::to_string),
        line: pos.map(|p| p.line),
        col: pos.map(|p| p.col),
        message,
        hint: None,
    }
}

fn warn(check: u8, step: Option<&str>, pos: Option<Pos>, message: String) -> Finding {
    Finding {
        severity: Severity::Warning,
        check,
        target: None,
        step: step.map(str::to_string),
        line: pos.map(|p| p.line),
        col: pos.map(|p| p.col),
        message,
        hint: None,
    }
}

fn with_hint(mut f: Finding, hint: impl Into<String>) -> Finding {
    f.hint = Some(hint.into());
    f
}

fn with_target(mut f: Finding, target: Target) -> Finding {
    f.target = Some(target);
    f
}

fn block_error_finding(e: &BlockError) -> Finding {
    error(1, None, None, e.message())
}

/// **Check 1**: the fence exists, the YAML parses, `orchestration:` is a
/// mapping, `steps:` is a non-empty list (all guaranteed by a successful
/// `block::parse`), every step has an `id`, and ids are unique.
fn check_parses(block: &Block, findings: &mut Vec<Finding>) {
    let mut seen = BTreeSet::new();
    let mut dupes = BTreeSet::new();
    for step in &block.steps {
        match &step.id {
            None => findings.push(with_hint(
                error(1, None, Some(step.pos), "step without an `id`".to_string()),
                "every step needs a unique `id:`",
            )),
            Some(id) => {
                if !seen.insert(id.clone()) {
                    dupes.insert(id.clone());
                }
            }
        }
    }
    for d in dupes {
        findings.push(error(1, Some(&d), None, format!("duplicate step id: {d}")));
    }
}

/// **Check 2**: every `requires` entry names a declared step; the
/// dependency graph is acyclic.
fn check_requires(block: &Block, findings: &mut Vec<Finding>) {
    let ids: BTreeSet<&str> = block.steps.iter().filter_map(|s| s.id.as_deref()).collect();

    for step in &block.steps {
        let Some(sid) = step.id.as_deref() else {
            continue;
        };
        for dep in &step.requires {
            if !ids.contains(dep.value.as_str()) {
                findings.push(with_hint(
                    error(
                        2,
                        Some(sid),
                        Some(dep.pos),
                        format!("step `{sid}`: requires unknown step `{}`", dep.value),
                    ),
                    "no step declares that id — add it, or correct the reference",
                ));
            }
        }
    }

    let graph: std::collections::BTreeMap<&str, Vec<&str>> = block
        .steps
        .iter()
        .filter_map(|s| {
            s.id.as_deref().map(|id| {
                (
                    id,
                    s.requires
                        .iter()
                        .map(|r| r.value.as_str())
                        .collect::<Vec<_>>(),
                )
            })
        })
        .collect();

    let mut seen = BTreeSet::new();
    for &start in graph.keys() {
        if seen.contains(start) {
            continue;
        }
        let mut stack = Vec::new();
        let mut on_stack = BTreeSet::new();
        if let Some(cycle) = find_cycle(start, &graph, &mut seen, &mut stack, &mut on_stack) {
            findings.push(error(
                2,
                None,
                None,
                format!("cycle: {}", cycle.join(" -> ")),
            ));
        }
    }
}

/// DFS with an on-stack set, mirroring `lint-orchestration.py:133-149`.
fn find_cycle<'a>(
    node: &'a str,
    graph: &std::collections::BTreeMap<&'a str, Vec<&'a str>>,
    seen: &mut BTreeSet<&'a str>,
    stack: &mut Vec<&'a str>,
    on_stack: &mut BTreeSet<&'a str>,
) -> Option<Vec<String>> {
    if on_stack.contains(node) {
        let mut path: Vec<String> = stack.iter().map(|s| s.to_string()).collect();
        path.push(node.to_string());
        return Some(path);
    }
    if seen.contains(node) {
        return None;
    }
    stack.push(node);
    on_stack.insert(node);
    if let Some(deps) = graph.get(node) {
        for &dep in deps {
            if graph.contains_key(dep)
                && let Some(cycle) = find_cycle(dep, graph, seen, stack, on_stack)
            {
                return Some(cycle);
            }
        }
    }
    on_stack.remove(node);
    stack.pop();
    seen.insert(node);
    None
}

/// **Check 3**: `retry.step` names a declared step, and `retry.max` is an
/// integer. Both are independent findings on the same step.
fn check_retry(block: &Block, findings: &mut Vec<Finding>) {
    let ids: BTreeSet<&str> = block.steps.iter().filter_map(|s| s.id.as_deref()).collect();

    for step in &block.steps {
        let Some(sid) = step.id.as_deref() else {
            continue;
        };
        let Some(retry) = &step.retry else { continue };

        let step_ok = retry
            .step
            .as_ref()
            .is_some_and(|s| ids.contains(s.value.as_str()));
        if !step_ok {
            let (pos, named) = match &retry.step {
                Some(s) => (s.pos, s.value.clone()),
                None => (retry.pos, "<missing>".to_string()),
            };
            findings.push(error(
                3,
                Some(sid),
                Some(pos),
                format!("step `{sid}`: retry.step `{named}` is not a step"),
            ));
        }

        if !retry.max_is_integer {
            findings.push(error(
                3,
                Some(sid),
                Some(retry.pos),
                format!("step `{sid}`: retry needs an integer `max`"),
            ));
        }
    }
}

/// **Check 4**: `when:` parses under the Phase 1 grammar; a `when:`
/// without `when-examples:` is an error; every `match:` example must
/// evaluate `true` and every `no-match:` example `false`. `Unknown` is a
/// warning, not an error. `reject:` is parsed and grammar-checked at
/// warning level only — it is not one of the six.
fn check_when(block: &Block, findings: &mut Vec<Finding>) {
    for step in &block.steps {
        let sid = step.id.as_deref().unwrap_or("?");

        if let Some(when) = &step.when {
            match expr::parse(&when.value) {
                Err(e) => {
                    findings.push(with_hint(
                        error(
                            4,
                            Some(sid),
                            Some(offset_pos(when.pos, e.pos)),
                            format!("step `{sid}`: `when:` off-grammar — {}", e.message),
                        ),
                        "see `hyprlayer orchestrate grammar` for the closed grammar",
                    ));
                }
                Ok(parsed) => match &step.when_examples {
                    None => findings.push(with_hint(
                        error(
                            4,
                            Some(sid),
                            Some(when.pos),
                            format!("step `{sid}`: `when:` without `when-examples:` is unverified"),
                        ),
                        "add `when-examples: {match: [...], no-match: [...]}`",
                    )),
                    Some(examples) => {
                        let unevaluable = check_examples(
                            sid,
                            &parsed,
                            &examples.match_,
                            true,
                            examples.pos,
                            findings,
                        ) + check_examples(
                            sid,
                            &parsed,
                            &examples.no_match,
                            false,
                            examples.pos,
                            findings,
                        );
                        if unevaluable > 0 {
                            let plural = if unevaluable == 1 { "" } else { "s" };
                            findings.push(with_hint(
                                warn(
                                    4,
                                    Some(sid),
                                    Some(examples.pos),
                                    format!(
                                        "step `{sid}`: {unevaluable} example{plural} cannot be \
                                         evaluated statically (the guard needs a live probe)"
                                    ),
                                ),
                                "only `matches()` evaluates during check; exit0/available/flag/exists/count \
                                 resolve at `orchestrate compile` time",
                            ));
                        }
                    }
                },
            }
        }

        if let Some(reject) = &step.reject
            && let Err(e) = expr::parse(&reject.value)
        {
            findings.push(warn(
                0,
                Some(sid),
                Some(offset_pos(reject.pos, e.pos)),
                format!("step `{sid}`: `reject:` off-grammar — {}", e.message),
            ));
        }
    }
}

/// `when:`/`reject:` are single-line scalars in every observed block, so
/// a parse error's byte offset (relative to the scalar's own text) maps
/// onto the same line, offset from the scalar's starting column.
fn offset_pos(base: Pos, byte_offset: usize) -> Pos {
    Pos {
        line: base.line,
        col: base.col + byte_offset,
    }
}

/// Returns how many of `samples` could not be evaluated statically. The
/// caller aggregates that into ONE warning per step rather than one per
/// example: a guard built on `exit0`/`exists`/`backend` can never be
/// evaluated here (check refuses to execute), so every example it carries
/// is unevaluable for the same structural reason. Reporting each one
/// separately doubled the output without adding a fact.
///
/// Genuine disagreements between a guard and its own examples are still
/// reported individually, because each one is a distinct defect.
fn check_examples(
    sid: &str,
    expr: &Expr,
    samples: &[String],
    want: bool,
    pos: Pos,
    findings: &mut Vec<Finding>,
) -> usize {
    let label = if want { "match" } else { "no-match" };
    let mut unevaluable = 0;
    for sample in samples {
        let env = env_for_example(expr, sample);
        let mut trace = Vec::new();
        match eval::eval(expr, &env, &mut trace) {
            Tri::Unknown => unevaluable += 1,
            got => {
                let got_bool = got == Tri::True;
                if got_bool != want {
                    findings.push(error(
                        4,
                        Some(sid),
                        Some(pos),
                        format!(
                            "step `{sid}`: {label} example {sample:?} evaluated {got_bool}, expected {want} — \
                             the guard is wrong, not the example"
                        ),
                    ));
                }
            }
        }
    }
    unevaluable
}

/// Ports `eval_when`'s substitution exactly (`lint-orchestration.py:78-98`):
/// **every** `matches(<field>, ...)` leaf binds to the example text,
/// regardless of what `<field>` names. Any other reading turns evaluable
/// examples into `Unknown` and changes the target skill's warning count.
fn env_for_example(expr: &Expr, text: &str) -> FactEnv {
    let mut env = FactEnv::new();
    let mut fields = Vec::new();
    collect_matches_fields(expr, &mut fields);
    for field in fields {
        env.set(
            field,
            FactValue::Str(text.to_string()),
            "check-example-text",
        );
    }
    env
}

fn collect_matches_fields(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Leaf(Leaf::Matches { field, .. }) => out.push(field.clone()),
        Expr::Leaf(_) => {}
        Expr::Not(inner) => collect_matches_fields(inner, out),
        Expr::And(a, b) | Expr::Or(a, b) => {
            collect_matches_fields(a, out);
            collect_matches_fields(b, out);
        }
    }
}

/// **Check 5**: every `given:` entry is a mapping with a non-empty `src:`.
fn check_given(block: &Block, findings: &mut Vec<Finding>) {
    for step in &block.steps {
        let sid = step.id.as_deref().unwrap_or("?");
        for entry in &step.given {
            let GivenEntry { pos, has_src, raw } = entry;
            if !has_src {
                findings.push(with_hint(
                    error(
                        5,
                        Some(sid),
                        Some(*pos),
                        format!("step `{sid}`: given entry {raw} has no `src:`"),
                    ),
                    "make it an `ask:` instead",
                ));
            }
        }
    }
}

/// **Check 6**: every `agent:`/`fanout:` name (including each name inside
/// `one-of [...]`) resolves in every *active* target's namespace, unless
/// that target's agent source is absent. `fanout:` without `over:` is
/// folded in here too, as the wiring check the prototype leaves dangling
/// outside the six — but checked once, target-agnostically, since a
/// missing `over:` isn't specific to any one harness.
fn check_agents(block: &Block, opts: &CheckOptions, findings: &mut Vec<Finding>) {
    for step in &block.steps {
        let sid = step.id.as_deref().unwrap_or("?");
        if let Some(fanout) = &step.fanout
            && step.over.is_none()
        {
            findings.push(with_hint(
                error(
                    6,
                    Some(sid),
                    Some(fanout.pos),
                    format!("step `{sid}`: `fanout:` without `over:`"),
                ),
                "add `over: <list-name>`",
            ));
        }
    }

    if opts.targets.is_empty() {
        let searched: Vec<String> = Target::ALL
            .iter()
            .flat_map(|t| agent_names::registry_for(*t).search_paths())
            .map(|p| p.display().to_string())
            .collect();
        findings.push(warn(
            6,
            None,
            None,
            format!(
                "no agent source found (looked in {}); skipping the agent-name check",
                searched.join(", ")
            ),
        ));
        return;
    }

    // `--agents-dir` is only ever legal alongside exactly one target (the
    // caller enforces this), so applying it to every active target here
    // is safe — there is only ever one.
    for &target in &opts.targets {
        check_agents_for_target(block, target, &opts.agents_dir, findings);
    }
}

fn check_agents_for_target(
    block: &Block,
    target: Target,
    agents_dir: &[PathBuf],
    findings: &mut Vec<Finding>,
) {
    let registry = agent_names::registry_for(target);
    let dirs = if agents_dir.is_empty() {
        registry.search_paths()
    } else {
        agents_dir.to_vec()
    };

    let names = match registry.resolve(&dirs) {
        AgentSource::Resolved(names) => names,
        AgentSource::None { searched } => {
            let list: Vec<String> = searched.iter().map(|p| p.display().to_string()).collect();
            findings.push(with_target(
                warn(
                    6,
                    None,
                    None,
                    format!(
                        "no agent source found for {target} (looked in {}); skipping the agent-name check",
                        list.join(", ")
                    ),
                ),
                target,
            ));
            return;
        }
    };
    let builtins: BTreeSet<&str> = registry.builtins().iter().copied().collect();

    for step in &block.steps {
        let sid = step.id.as_deref().unwrap_or("?");
        if let Some(fanout) = &step.fanout {
            check_agent_ref(sid, fanout, target, &names, &builtins, findings);
        }
        if let Some(agent) = &step.agent {
            check_agent_ref(sid, agent, target, &names, &builtins, findings);
        }
    }
}

fn check_agent_ref(
    sid: &str,
    field: &Spanned<AgentRef>,
    target: Target,
    names: &BTreeSet<String>,
    builtins: &BTreeSet<&str>,
    findings: &mut Vec<Finding>,
) {
    let candidates: Vec<&str> = match &field.value {
        AgentRef::One(n) => vec![n.as_str()],
        AgentRef::OneOf(ns) => ns.iter().map(String::as_str).collect(),
    };
    for name in candidates {
        if !names.contains(name) && !builtins.contains(name) {
            let finding = with_target(
                error(
                    6,
                    Some(sid),
                    Some(field.pos),
                    format!("step `{sid}`: unknown agent `{name}`"),
                ),
                target,
            );
            // The point of validating against more than one target: a
            // name valid for Claude can be missing from a harness that
            // loads the very same file without the author doing anything.
            let finding = if target == Target::OpenCode {
                with_hint(
                    finding,
                    "this file IS loaded by opencode — it reads ~/.claude/skills/ directly",
                )
            } else {
                finding
            };
            findings.push(finding);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "---\nname: {name}\n---\n\n```yaml\n{body}\n```\n").unwrap();
        path
    }

    fn repo_agents_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/claude/agents")
    }

    #[test]
    fn dangling_requires_is_an_error_naming_the_step_and_the_bad_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      requires: [nope]\n      agent: cartographer\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(report.has_errors());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.check == 2 && f.message.contains("requires unknown step `nope`"))
        );
    }

    #[test]
    fn a_two_cycle_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      requires: [b]\n      agent: cartographer\n    - id: b\n      requires: [a]\n      agent: archivist\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.starts_with("cycle:"))
        );
    }

    #[test]
    fn bad_retry_reports_both_findings() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      retry: { step: ghost }\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("is not a step"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("integer `max`"))
        );
    }

    #[test]
    fn unknown_agent_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      agent: cartografer\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.check == 6 && f.message.contains("unknown agent `cartografer`"))
        );
    }

    #[test]
    fn given_entry_without_src_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      given: [just-a-string]\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.check == 5 && f.message.contains("no `src:`"))
        );
    }

    #[test]
    fn a_guard_whose_no_match_example_actually_matches_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      when: matches(request, \"[A-Z]{2,}-\\d+\")\n      when-examples:\n        match: []\n        no-match: [\"per ADR-0002\"]\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.check == 4 && f.message.contains("evaluated true, expected false"))
        );
    }

    #[test]
    fn fanout_without_over_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      fanout: cartographer\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.check == 6 && f.message.contains("`fanout:` without `over:`"))
        );
    }

    #[test]
    fn empty_agent_source_warns_and_skips_check_six_instead_of_failing_every_name() {
        let dir = tempfile::tempdir().unwrap();
        let empty_agents = tempfile::tempdir().unwrap();
        let path = write_fixture(
            dir.path(),
            "x.md",
            "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n",
        );
        let opts = CheckOptions {
            agents_dir: vec![empty_agents.path().to_path_buf()],
            targets: vec![Target::Claude],
        };
        let report = check_file(&path, &opts);
        assert!(!report.has_errors());
        assert!(report.findings.iter().any(
            |f| f.severity == Severity::Warning && f.message.contains("no agent source found")
        ));
    }

    #[test]
    fn check_never_probes_exit0() {
        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("sentinel");
        let body = format!(
            "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      when: exit0(touch {})\n      when-examples:\n        match: []\n        no-match: []\n",
            sentinel.display()
        );
        let path = write_fixture(dir.path(), "x.md", &body);
        let opts = CheckOptions {
            agents_dir: vec![repo_agents_dir()],
            targets: vec![Target::Claude],
        };
        let _report = check_file(&path, &opts);
        assert!(
            !sentinel.exists(),
            "check must never execute an exit0() guard"
        );
    }
}
