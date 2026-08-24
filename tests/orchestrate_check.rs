//! Integration tests for `hyprlayer orchestrate check`. Eight deliberately
//! broken fixtures, each inline (three to six lines whose content is the
//! point), plus the positive case against the real, shipped
//! `research_codebase` skill.
//!
//! Every invocation passes `--target claude --agents-dir <repo_root>/claude/agents`:
//! `run()` points `HOME` at a tempdir, so `~/.claude/agents` does not
//! exist — without `--agents-dir`, check 6 silently skips and the
//! `unknown-agent` fixture would pass. `--target claude` is likewise
//! required rather than relied on as a default: under an isolated tempdir
//! `HOME`, `Target::is_installed()` is false for every target (`dirs`
//! honors the `HOME` override in the child process), so the default
//! installed-target set is empty here, not `{claude}` — pin it
//! explicitly instead of depending on that.

mod common;

use std::path::{Path, PathBuf};

use common::{isolated_dirs, run};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn agents_dir_arg() -> String {
    repo_root()
        .join("claude")
        .join("agents")
        .display()
        .to_string()
}

fn write_skill(dir: &Path, name: &str, orchestration_body: &str) -> PathBuf {
    let path = dir.join(name);
    let content = format!(
        "---\nname: {name}\ndescription: fixture\n---\n\n```yaml\n{orchestration_body}\n```\n"
    );
    std::fs::write(&path, content).unwrap();
    path
}

fn check_output(xdg: &Path, file: &Path, extra: &[&str]) -> (String, String, i32) {
    let mut args = vec![
        "orchestrate",
        "check",
        file.to_str().unwrap(),
        "--target",
        "claude",
        "--agents-dir",
    ];
    let agents_dir = agents_dir_arg();
    args.push(&agents_dir);
    args.extend_from_slice(extra);
    let out = run(xdg, &args);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn dangling_requires_fails_naming_the_unknown_step() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "dangling-requires.md",
        "orchestration:\n  steps:\n    - id: a\n      requires: [nope]\n      agent: cartographer\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("requires unknown step `nope`"),
        "stdout: {stdout}"
    );
}

#[test]
fn a_two_step_cycle_fails_naming_the_cycle() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "cycle.md",
        "orchestration:\n  steps:\n    - id: a\n      requires: [b]\n      agent: cartographer\n    - id: b\n      requires: [a]\n      agent: archivist\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(stdout.contains("cycle:"), "stdout: {stdout}");
}

#[test]
fn bad_retry_fails_with_both_the_step_and_max_findings() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "bad-retry.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      retry: { step: ghost }\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("retry.step `ghost` is not a step"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("retry needs an integer `max`"),
        "stdout: {stdout}"
    );
}

#[test]
fn unknown_agent_fails_naming_the_typo() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "unknown-agent.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: cartografer\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("unknown agent `cartografer`"),
        "stdout: {stdout}"
    );
}

#[test]
fn given_entry_without_src_fails() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "given-no-src.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      given: [just-a-string]\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(stdout.contains("has no `src:`"), "stdout: {stdout}");
}

#[test]
fn a_guard_that_fails_its_own_no_match_example_fails() {
    let (_guard, xdg) = isolated_dirs();
    // regex_lite::Regex::new(r"[A-Z]{2,}-\d+").is_match("per ADR-0002") == true,
    // so this no-match example actually matches — the guard is wrong.
    let path = write_skill(
        &xdg,
        "guard-wrong.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      when: matches(request, \"[A-Z]{2,}-\\d+\")\n      when-examples:\n        match: []\n        no-match: [\"per ADR-0002\"]\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("evaluated true, expected false"),
        "stdout: {stdout}"
    );
}

#[test]
fn fanout_without_over_fails() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "fanout-no-over.md",
        "orchestration:\n  steps:\n    - id: a\n      fanout: cartographer\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("`fanout:` without `over:`"),
        "stdout: {stdout}"
    );
}

#[test]
fn an_empty_one_of_fails_rather_than_planning_a_choice_with_no_candidates() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "empty-one-of.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: one-of\n",
    );
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("`agent: one-of` lists no agents"),
        "stdout: {stdout}"
    );
}

#[test]
fn the_research_skill_checks_clean_with_four_warnings() {
    // Four warnings, one per step whose guard needs a live probe:
    // `history` (exit0 test -d thoughts, or a notion/anytype backend),
    // `thoughts-lookup` (a git/obsidian backend),
    // `permalinks` (exit0) and `sync` (backend == git). Each is reported
    // once for the step rather than once per example, because every
    // example it carries is unevaluable for the same structural reason.
    //
    // `history` is the guard that collapsed research_codebase_nt and
    // research_codebase_generic into this one skill: with no thoughts
    // directory it resolves false and the archivist never spawns, which
    // is precisely what the _nt variant used to encode by hand.
    //
    // Diverges from the Python prototype, which emitted six here: it
    // reported per-example, and `follow-up` carried an `exists()` guard
    // that has since been recast as the `judgment:` it always was.
    let (_guard, xdg) = isolated_dirs();
    let fixture = repo_root().join("claude/skills/research_codebase/SKILL.md");
    let (stdout, stderr, code) = check_output(&xdg, &fixture, &[]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let warn_count = stdout.matches("warn").count();
    assert_eq!(
        warn_count, 4,
        "expected exactly four warnings, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("error"),
        "expected zero errors, got:\n{stdout}"
    );

    let (json_out, _stderr, json_code) = check_output(&xdg, &fixture, &["--json"]);
    assert_eq!(json_code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&json_out).unwrap();
    assert_eq!(parsed["ok"], true, "payload: {parsed}");
    assert_eq!(parsed["files"][0]["warnings"], 4, "payload: {parsed}");
    assert_eq!(parsed["files"][0]["errors"], 0, "payload: {parsed}");
}

#[test]
fn check_never_probes() {
    // Fixture guard: `when: exit0(touch <tempdir>/sentinel)`. `check` must
    // report it as an unevaluable warning and the sentinel must not exist
    // afterwards.
    let (_guard, xdg) = isolated_dirs();
    let sentinel = xdg.join("sentinel");
    let body = format!(
        "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n      when: exit0(touch {})\n      when-examples:\n        match: []\n        no-match: []\n",
        sentinel.display()
    );
    let path = write_skill(&xdg, "never-probes.md", &body);
    let (_stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 0);
    assert!(
        !sentinel.exists(),
        "check must never execute an exit0() guard"
    );
}

#[test]
fn a_skill_with_no_yaml_block_reports_plainly_rather_than_panicking() {
    let (_guard, xdg) = isolated_dirs();
    let path = xdg.join("no-block.md");
    std::fs::write(
        &path,
        "---\nname: no-block\n---\n\nJust prose, no orchestration block.\n",
    )
    .unwrap();
    let (stdout, _stderr, code) = check_output(&xdg, &path, &[]);
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(
        stdout.contains("no ```yaml block found"),
        "stdout: {stdout}"
    );
}
