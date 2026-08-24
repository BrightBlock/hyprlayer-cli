//! Integration tests for `hyprlayer orchestrate compile`, exercised
//! against the real `research_codebase` skill and against
//! small inline blocks for the individual scheduling rules.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{hyprlayer_bin, isolated_dirs, run};
use wait_timeout::ChildExt;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// The target skill reproduces the observed run ONLY with these pins:
//   permalinks: exit0(git merge-base --is-ancestor HEAD @{u}) → true
//   sync:       backend == git                                → true
//   history:    exit0(test -d thoughts)                       → true
// Without the first two, both steps skip and a wave vanishes. Without the
// third the archivist never spawns and totalSpawns is 6, not 7 — that guard
// is what lets one skill cover both the thoughts and no-thoughts cases, so
// the no-thoughts shape is simply this file with the pin removed.
const PINS: [&str; 6] = [
    "--fact",
    "exit0(git merge-base --is-ancestor HEAD @{u})=true",
    "--fact",
    "backend=git",
    "--fact",
    "exit0(test -d thoughts)=true",
];

fn compile_json(xdg: &Path, extra: &[&str]) -> serde_json::Value {
    let fixture = repo_root().join("claude/skills/research_codebase/SKILL.md");
    let claude_agents = repo_root().join("claude/agents");
    let mut args = vec![
        "orchestrate",
        "compile",
        fixture.to_str().unwrap(),
        "--target",
        "claude",
        "--agents-dir",
        claude_agents.to_str().unwrap(),
    ];
    args.extend_from_slice(&PINS);
    args.extend_from_slice(extra);
    let out = run(xdg, &args);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, got {e}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[test]
fn fifteen_steps_compile_to_nine_waves_and_seven_spawns() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    assert_eq!(d["stepCount"], 15, "payload: {d}");
    assert_eq!(d["waveCount"], 9, "payload: {d}");
    assert_eq!(d["totalSpawns"], 7, "payload: {d}");
}

#[test]
fn skipped_steps_do_not_consume_a_wave_number() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let skipped: Vec<&str> = d["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    // Nine waves, not eight: `present` requires `permalinks` so the summary
    // is not handed over while links are still being written into the doc it
    // points at, which pushes `present` to w8 and `follow-up` to w9.
    //
    // `follow-up` is no longer here: its `exists()` guard was a judgment
    // in disguise and is now declared as one, so it is scheduled (wave 9)
    // with the call recorded in `unresolved[]` instead of silently skipped
    // by a guard that could never evaluate.
    assert_eq!(skipped, vec!["web", "tickets"], "payload: {d}");
    assert_eq!(d["waveCount"], 9, "payload: {d}");
}

#[test]
fn a_skipped_requirement_still_satisfies_its_dependent() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let waves = d["waves"].as_array().unwrap();
    // verify-results requires [map, history, targeted, web, tickets];
    // web/tickets are skipped and must not block it — it lands right
    // after map/history/targeted's wave.
    let verify_wave = waves
        .iter()
        .find(|w| {
            w["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == "verify-results")
        })
        .unwrap();
    assert_eq!(verify_wave["wave"], 4, "payload: {d}");
}

#[test]
fn every_skipped_step_records_the_guard_that_skipped_it() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    for s in d["skipped"].as_array().unwrap() {
        assert!(
            s["when"].as_str().is_some_and(|w| !w.is_empty()),
            "payload: {d}"
        );
        assert!(
            matches!(s["value"].as_str(), Some("false") | Some("unknown")),
            "payload: {d}"
        );
    }
}

#[test]
fn a_one_of_agent_is_one_spawn_with_an_unresolved_choice() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let unresolved = d["unresolved"].as_array().unwrap();
    let choice: Vec<_> = unresolved
        .iter()
        .filter(|u| u["kind"] == "agent-choice")
        .collect();
    // Two `one-of` steps: `targeted` picks among the three narrow codebase
    // agents, `thoughts-lookup` between locator and analyzer. Each is one
    // spawn with the choice deferred, never two spawns.
    let steps: Vec<&str> = choice.iter().map(|c| c["step"].as_str().unwrap()).collect();
    assert_eq!(steps, vec!["thoughts-lookup", "targeted"], "payload: {d}");
    let targeted = choice.iter().find(|c| c["step"] == "targeted").unwrap();
    assert_eq!(targeted["candidates"].as_array().unwrap().len(), 3);
}

#[test]
fn a_fanout_with_no_size_binding_is_an_error_naming_the_flag() {
    let (_guard, xdg) = isolated_dirs();
    let fixture = repo_root().join("claude/skills/research_codebase/SKILL.md");
    let claude_agents = repo_root().join("claude/agents");
    let mut args = vec![
        "orchestrate",
        "compile",
        fixture.to_str().unwrap(),
        "--target",
        "claude",
        "--agents-dir",
        claude_agents.to_str().unwrap(),
        "--request",
        "map the PTY stack",
        "--no-probe",
    ];
    args.extend_from_slice(&PINS);
    let out = run(&xdg, &args);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--fanout areas=N required"),
        "stderr: {stderr}"
    );
}

#[test]
fn two_identical_invocations_produce_byte_identical_stdout() {
    let (_guard, xdg) = isolated_dirs();
    let fixture = repo_root().join("claude/skills/research_codebase/SKILL.md");
    let claude_agents = repo_root().join("claude/agents");
    let mut args = vec![
        "orchestrate",
        "compile",
        fixture.to_str().unwrap(),
        "--target",
        "claude",
        "--agents-dir",
        claude_agents.to_str().unwrap(),
        "--areas",
        "4",
        "--request",
        "map the PTY stack",
        "--no-probe",
    ];
    args.extend_from_slice(&PINS);
    let out1 = run(&xdg, &args);
    let out2 = run(&xdg, &args);
    assert_eq!(out1.stdout, out2.stdout);
}

#[test]
fn the_plan_hash_ignores_the_plan_hash_field() {
    let (_guard, xdg) = isolated_dirs();
    let d4 = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let d6 = compile_json(
        &xdg,
        &[
            "--areas",
            "6",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    assert_ne!(
        d4["planHash"], d6["planHash"],
        "different areas count must hash differently"
    );
    // Same invocation twice must hash identically (byte-identical run,
    // asserted separately) — here we assert the hash field itself is a
    // stable sha256 string shape.
    let hash = d4["planHash"].as_str().unwrap();
    assert!(hash.starts_with("sha256:"), "payload: {d4}");
    assert_eq!(hash.len(), "sha256:".len() + 64, "payload: {d4}");
}

#[test]
fn compile_with_no_probe_runs_no_commands() {
    // Guard is `exit0(touch <tempdir>/sentinel)`; with --no-probe the
    // step skips as `unknown` and the sentinel file is never created.
    let (_guard, xdg) = isolated_dirs();
    let sentinel = xdg.join("sentinel");
    let content = format!(
        "---\nname: x\n---\n\n```yaml\norchestration:\n  steps:\n    - id: a\n      inline: true\n      when: exit0(touch {})\n```\n",
        sentinel.display()
    );
    let path = xdg.join("x.md");
    std::fs::write(&path, content).unwrap();
    let out = run(
        &xdg,
        &[
            "orchestrate",
            "compile",
            path.to_str().unwrap(),
            "--target",
            "claude",
            "--no-probe",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !sentinel.exists(),
        "compile --no-probe must never execute an exit0() guard"
    );
}

// Live-probe resolution (via: probe:exec / probe:config against a real
// `git` invocation and the real `HyprlayerConfig`) is exercised manually
// per the plan's Manual Testing Steps, not here — it depends on this
// repo's own upstream-branch state, which an automated test should not
// assume.

#[test]
fn every_judgment_step_is_recorded_as_an_unresolved_decision() {
    // A `judgment:` is a call the compiler declines to make, exactly like
    // `agent: one-of [...]`. Both land in `unresolved[]` so a plan reads as
    // a to-do rather than pretending the decision was made. A step
    // carrying both appears twice, because they are two separate calls.
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let judgments: Vec<&str> = d["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["kind"] == "judgment")
        .map(|u| u["step"].as_str().unwrap())
        .collect();
    assert_eq!(
        judgments,
        vec![
            "decompose",
            // `history` is the consolidation of the old no-thoughts variant.
            // Its `when:` only proves a paper trail EXISTS; whether this
            // research wants one is the user's intent, which no probe can
            // read — so the decision is a judgment and lands here.
            "history",
            "thoughts-lookup",
            "targeted",
            "verify-results",
            "write",
            "follow-up"
        ],
        "payload: {d}"
    );
    for u in d["unresolved"].as_array().unwrap() {
        if u["kind"] == "judgment" {
            let q = u["question"].as_str().unwrap_or("");
            assert!(!q.is_empty(), "judgment entry needs its question: {u}");
        }
    }
}

#[test]
fn a_probe_never_writes_its_own_output_into_the_plan() {
    // `compile`'s stdout IS the plan artifact, so an `exit0()` probe must be
    // run for its status alone. Inheriting stdout splices the command's
    // output in front of the JSON and breaks the byte-identical guarantee —
    // found in the wild on a `exit0(git log -1 --format=%ai)` guard, which
    // printed a timestamp above the plan.
    let (_guard, xdg) = isolated_dirs();
    let content = "---\nname: x\n---\n\n```yaml\norchestration:\n  steps:\n    - id: a\n      inline: true\n      when: exit0(echo probe-chatter)\n```\n";
    let path = xdg.join("probe-output.md");
    std::fs::write(&path, content).unwrap();
    let out = run(
        &xdg,
        &[
            "orchestrate",
            "compile",
            path.to_str().unwrap(),
            "--target",
            "claude",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The invariant is positional, not textual: the command's own text is
    // legitimately in the plan (it IS the guard expression and the leaf key),
    // so grepping for it can never pass. Leaked output lands *above* the
    // object, so requiring stdout to begin with `{` and parse whole is what
    // actually catches it.
    assert!(
        stdout.trim_start().starts_with('{'),
        "probe output leaked above the plan artifact:\n{stdout}"
    );
    let d: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("plan is not valid JSON ({e}):\n{stdout}"));
    // The probe still ran and still decided the guard: `echo` exits 0.
    assert_eq!(d["waves"][0]["steps"][0]["id"], "a", "payload: {d}");
}

/// Spawns `compile` with a stdin the caller owns and its plan written to a
/// file rather than a pipe — waiting on a child whose stdout is a pipe can
/// deadlock on a full buffer, and this test's whole point is the wait.
#[cfg(unix)]
fn spawn_compile_with_piped_stdin(
    xdg: &Path,
    block: &Path,
    plan_path: &Path,
) -> std::process::Child {
    Command::new(hyprlayer_bin())
        .args([
            "orchestrate",
            "compile",
            block.to_str().unwrap(),
            "--target",
            "claude",
        ])
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", xdg)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(std::fs::File::create(plan_path).unwrap()))
        .stderr(Stdio::null())
        .spawn()
        .expect("hyprlayer binary should be runnable")
}

/// The stdin half of the same invariant `a_probe_never_writes_its_own_output_
/// into_the_plan` pins for stdout. A probe that inherited stdin makes the
/// guard a reader of the caller's input: `exit0(read line)` under an open
/// pipe blocked forever and `compile` emitted no plan at all, and under a
/// pipe carrying data it consumed bytes meant for the parent and resolved
/// the guard from them — so `planHash` moved with whatever the caller
/// happened to leave on stdin, which is not one of the inputs the artifact
/// is a function of. Unix-only: `read` is a POSIX shell builtin.
#[cfg(unix)]
#[test]
fn a_guard_that_reads_stdin_neither_stalls_compile_nor_eats_the_input() {
    let (_guard, xdg) = isolated_dirs();
    let content = "---\nname: x\n---\n\n```yaml\norchestration:\n  steps:\n    - id: a\n      inline: true\n      when: exit0(read line)\n```\n";
    let block = xdg.join("stdin-guard.md");
    std::fs::write(&block, content).unwrap();

    // A pipe that is never written and never closed: an inherited stdin
    // gives `read` a descriptor that will deliver neither a line nor an EOF.
    let waited_plan = xdg.join("waited.json");
    let mut child = spawn_compile_with_piped_stdin(&xdg, &block, &waited_plan);
    let write_end = child.stdin.take().expect("stdin was piped");
    let finished = child
        .wait_timeout(Duration::from_secs(30))
        .expect("waiting on compile must not fail");
    let Some(status) = finished else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("compile hung on an `exit0()` guard that reads stdin");
    };
    assert!(status.success(), "compile failed: {status:?}");
    drop(write_end);

    // The same guard with a line actually available. Reading it would flip
    // the guard to true and swallow the parent's input on the way.
    let fed_plan = xdg.join("fed.json");
    let mut child = spawn_compile_with_piped_stdin(&xdg, &block, &fed_plan);
    let mut write_end = child.stdin.take().expect("stdin was piped");
    write_end.write_all(b"STOLEN\n").unwrap();
    drop(write_end);
    let status = child.wait().expect("compile must terminate");
    assert!(status.success(), "compile failed: {status:?}");

    let waited: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&waited_plan).unwrap()).unwrap();
    let fed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fed_plan).unwrap()).unwrap();
    assert_eq!(
        waited["planHash"], fed["planHash"],
        "the caller's stdin changed the plan: {waited} vs {fed}"
    );
    assert_eq!(fed["guards"][0]["value"], "false", "payload: {fed}");
}

#[test]
fn a_declared_retry_is_reported_on_its_step_without_moving_the_spawn_count() {
    // `retry: { step: map, max: 1 }` sits on `verify-results` and targets
    // `map`, so it is reported on the step that DECLARES it, naming the
    // target — the plan reports retries as data and never follows them.
    //
    // It must not move `totalSpawns`, which enumerates what is SCHEDULED
    // (map x4, history x1, thoughts-lookup x1, targeted x1 = 7). A retry
    // is contingent on a failure compile cannot predict; folding a worst
    // case in would falsify that enumeration and turn a count into a
    // ceiling. `validate_plan` is the case that makes this concrete: its
    // `verify-report` spawns nothing and retries a 3-spawn fanout, so a
    // worst-case total would report 6 for a run that makes 3. A reader
    // budgeting for the worst case reads `max` here against the target
    // step's own `spawns`.
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(
        &xdg,
        &[
            "--areas",
            "4",
            "--request",
            "map the PTY stack",
            "--no-probe",
        ],
    );
    let retries: Vec<(&str, &serde_json::Value)> = d["waves"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| w["steps"].as_array().unwrap())
        .filter(|s| !s["retry"].is_null())
        .map(|s| (s["id"].as_str().unwrap(), &s["retry"]))
        .collect();
    assert_eq!(retries.len(), 1, "payload: {d}");
    let (id, retry) = retries[0];
    assert_eq!(id, "verify-results", "payload: {d}");
    assert_eq!(retry["step"], "map", "payload: {d}");
    assert_eq!(retry["max"], 1, "payload: {d}");
    assert_eq!(d["totalSpawns"], 7, "payload: {d}");

    // Every other scheduled step carries the key explicitly as null, the
    // same way `agent`/`over` do — a reader never has to guess whether a
    // missing key means "no retry" or "not emitted".
    for w in d["waves"].as_array().unwrap() {
        for s in w["steps"].as_array().unwrap() {
            assert!(s.get("retry").is_some(), "step missing `retry` key: {s}");
        }
    }
}
