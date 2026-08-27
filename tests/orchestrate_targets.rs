//! Per-platform agent resolution, checks 1-5 reported once (not once per
//! target), exit 1 when any single target errors, and the `--agents-dir` +
//! multi-target guard rail.
//!
//! Agent directories are built inside `isolated_dirs()` tempdirs so no
//! test depends on what the developer actually has installed.

mod common;

use std::path::{Path, PathBuf};

use common::{isolated_dirs, run_in};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn write_skill(dir: &Path, name: &str, orchestration_body: &str) -> PathBuf {
    let path = dir.join(name);
    let content = format!(
        "---\nname: {name}\ndescription: fixture\n---\n\n```yaml\n{orchestration_body}\n```\n"
    );
    std::fs::write(&path, content).unwrap();
    path
}

fn json_of(xdg: &Path, args: &[&str]) -> serde_json::Value {
    let out = run_in(xdg, xdg, args);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, got error {e}\nstdout: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

const CARTOGRAPHER_STEP: &str =
    "orchestration:\n  steps:\n    - id: a\n      agent: cartographer\n";

#[test]
fn bundled_agents_resolve_for_both_native_platforms() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);

    let claude_agents = repo_root().join("assets/claude/agents");
    let codex_agents = repo_root().join("assets/codex/agents");
    for (target, agents) in [("claude", claude_agents), ("codex", codex_agents)] {
        let out = run_in(
            &xdg,
            &xdg,
            &[
                "orchestrate",
                "check",
                path.to_str().unwrap(),
                "--target",
                target,
                "--agents-dir",
                agents.to_str().unwrap(),
            ],
        );
        assert!(
            out.status.success(),
            "{target} should resolve {}: {out:?}",
            agents.display()
        );
    }
}

#[test]
fn an_unverified_codex_builtin_name_requires_a_custom_agent_file() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "x.md",
        "orchestration:\n  steps:\n    - id: a\n      agent: explorer\n",
    );
    let agents = xdg.join("codex-agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("fixture.toml"),
        "name = \"fixture\"\ndescription = \"fixture\"\n",
    )
    .unwrap();

    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "codex",
            "--agents-dir",
            agents.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "out: {out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("unknown agent `explorer`"),
        "undocumented Codex agent names must not resolve implicitly"
    );
}

#[test]
fn checks_one_through_five_are_reported_once_not_once_per_target() {
    // A dangling `requires` must produce exactly ONE finding with
    // target: null, however many targets are active.
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(
        &xdg,
        "x.md",
        "orchestration:\n  steps:\n    - id: a\n      requires: [nope]\n      agent: cartographer\n",
    );
    // `--agents-dir` requires exactly one target, so this multi-target
    // run falls back to each target's own (absent, under an isolated
    // HOME/CWD) default search paths — check 6 will warn "no agent
    // source found" per target, but check 2's dangling-requires finding
    // must still appear exactly once, target-agnostic.
    let payload = json_of(
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--json",
            "--target",
            "claude",
            "--target",
            "codex",
        ],
    );
    let findings = payload["files"][0]["findings"].as_array().unwrap();
    let dangling: Vec<_> = findings
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .unwrap_or("")
                .contains("requires unknown step")
        })
        .collect();
    assert_eq!(dangling.len(), 1, "payload: {payload}");
    assert!(
        dangling[0]["target"].is_null(),
        "checks 1-5 findings must carry target: null, got {payload}"
    );
}

#[test]
fn exit_is_one_when_any_single_target_has_an_error() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let claude_agents = repo_root().join("assets/claude/agents");
    let incomplete_codex_agents = xdg.join("incomplete-codex-agents");
    std::fs::create_dir_all(&incomplete_codex_agents).unwrap();
    std::fs::write(
        incomplete_codex_agents.join("fixture.toml"),
        "name = \"fixture\"\ndescription = \"fixture\"\n",
    )
    .unwrap();

    // claude alone: clean.
    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "claude",
            "--agents-dir",
            claude_agents.to_str().unwrap(),
        ],
    );
    assert!(out.status.success());

    // codex alone: fails against an intentionally incomplete registry.
    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "codex",
            "--agents-dir",
            incomplete_codex_agents.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1));
}

/// `exit_is_one_when_any_single_target_has_an_error` runs the two targets
/// in separate invocations, so it never exercises the aggregation at
/// `src/commands/orchestrate/check.rs`: a regression rewriting `ok` from
/// "no target has errors" to "any target passed" leaves it green. This
/// drives both targets through one invocation instead.
///
/// `--agents-dir` requires exactly one target, so both registries are
/// installed for real under the isolated `HOME`.
#[test]
fn exit_is_one_when_one_of_two_targets_in_one_invocation_has_an_error() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);

    let claude_dest = xdg.join(".claude").join("agents");
    std::fs::create_dir_all(&claude_dest).unwrap();
    for entry in std::fs::read_dir(repo_root().join("assets/claude/agents"))
        .unwrap()
        .flatten()
    {
        std::fs::copy(entry.path(), claude_dest.join(entry.file_name())).unwrap();
    }
    let codex_dest = xdg.join(".codex").join("agents");
    std::fs::create_dir_all(&codex_dest).unwrap();
    std::fs::write(
        codex_dest.join("fixture.toml"),
        "name = \"fixture\"\ndescription = \"fixture\"\n",
    )
    .unwrap();

    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--json",
            "--target",
            "claude",
            "--target",
            "codex",
        ],
    );
    // One invocation, one exit code: Claude resolves `cartographer` and the
    // intentionally incomplete Codex registry does not.
    assert_eq!(out.status.code(), Some(1), "out: {out:?}");

    let d: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(d["ok"], false, "payload: {d}");
    let file = &d["files"][0];
    assert_eq!(file["ok"], false, "payload: {d}");
    let targets = file["targets"].as_array().expect("targets[]");
    assert_eq!(targets.len(), 2, "payload: {d}");
    let ok_by_target: Vec<(&str, bool)> = targets
        .iter()
        .map(|t| {
            (
                t["target"].as_str().unwrap(),
                t["ok"].as_bool().expect("ok flag"),
            )
        })
        .collect();
    assert!(ok_by_target.contains(&("claude", true)), "payload: {d}");
    assert!(ok_by_target.contains(&("codex", false)), "payload: {d}");
}

#[test]
fn no_installed_target_warns_and_skips_check_six() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    // No --target given, and under an isolated HOME/CWD neither native
    // platform is installed, so the default target set is empty.
    let payload = json_of(
        &xdg,
        &["orchestrate", "check", path.to_str().unwrap(), "--json"],
    );
    assert_eq!(payload["ok"], true, "payload: {payload}");
    let findings = payload["files"][0]["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["message"]
            .as_str()
            .unwrap_or("")
            .contains("no agent source found")),
        "payload: {payload}"
    );
    assert_eq!(
        payload["files"][0]["targets"].as_array().unwrap().len(),
        0,
        "payload: {payload}"
    );
}

#[test]
fn agents_dir_with_multiple_targets_is_an_error_naming_the_fix() {
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let claude_agents = repo_root().join("assets/claude/agents");
    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "claude",
            "--target",
            "codex",
            "--agents-dir",
            claude_agents.to_str().unwrap(),
        ],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--agents-dir requires exactly one --target"),
        "stderr: {stderr}"
    );
}

#[test]
fn the_default_target_set_is_claude_only_when_no_other_platform_is_installed() {
    let (_guard, xdg) = isolated_dirs();
    // Install only Claude's agent directory under the isolated HOME.
    let claude_agents_dir = xdg.join(".claude").join("agents");
    std::fs::create_dir_all(&claude_agents_dir).unwrap();
    std::fs::write(
        claude_agents_dir.join("cartographer.md"),
        "---\nname: cartographer\n---\nbody\n",
    )
    .unwrap();

    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let payload = json_of(
        &xdg,
        &["orchestrate", "check", path.to_str().unwrap(), "--json"],
    );
    let targets: Vec<&str> = payload["files"][0]["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"].as_str().unwrap())
        .collect();
    assert_eq!(targets, vec!["claude"], "payload: {payload}");
}

#[test]
fn agents_dir_does_not_make_a_target_count_as_installed() {
    // `--agents-dir` requires exactly one `--target`, so pass `claude`
    // explicitly with an `--agents-dir` pointing at a directory that is
    // NOT any registry's real default search path, then check that a
    // *second*, unrelated invocation with no `--target` at all still
    // resolves the default set as if `--agents-dir` had never been used —
    // proving the flag plays no role in `is_installed()`.
    let (_guard, xdg) = isolated_dirs();
    let unrelated_agents = xdg.join("not-a-registry-path");
    std::fs::create_dir_all(&unrelated_agents).unwrap();
    std::fs::write(
        unrelated_agents.join("cartographer.md"),
        "---\nname: cartographer\n---\nbody\n",
    )
    .unwrap();

    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "claude",
            "--agents-dir",
            unrelated_agents.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "explicit --target claude with the override should resolve cartographer"
    );

    // Now the default-resolution run: no --target, no --agents-dir. If
    // the prior invocation's --agents-dir had somehow made claude count
    // as "installed" going forward, this would silently pick claude up
    // as the default and mask a real regression.
    let payload = json_of(
        &xdg,
        &["orchestrate", "check", path.to_str().unwrap(), "--json"],
    );
    assert_eq!(
        payload["files"][0]["targets"].as_array().unwrap().len(),
        0,
        "payload: {payload}"
    );
}

#[test]
fn a_repeated_target_is_deduplicated_into_one_block() {
    // clap's `Vec<Target>` accepts repeats. Without dedup this emitted
    // two identical `targets[]` entries and ran check 6 twice, breaking
    // the "one entry per active target" contract and double-counting for
    // any consumer summing `targets[].errors`.
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let payload = json_of(
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--json",
            "--target",
            "claude",
            "--target",
            "claude",
        ],
    );
    let targets = payload["files"][0]["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1, "payload: {payload}");
    assert_eq!(targets[0]["target"], "claude", "payload: {payload}");
}

#[test]
fn a_repeated_target_still_allows_agents_dir() {
    // `--agents-dir` requires exactly one target; a target repeated twice
    // is still one target, so it must not trip that guard rail.
    let (_guard, xdg) = isolated_dirs();
    let path = write_skill(&xdg, "x.md", CARTOGRAPHER_STEP);
    let claude_agents = repo_root().join("assets/claude/agents");
    let out = run_in(
        &xdg,
        &xdg,
        &[
            "orchestrate",
            "check",
            path.to_str().unwrap(),
            "--target",
            "claude",
            "--target",
            "claude",
            "--agents-dir",
            claude_agents.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
