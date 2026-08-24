//! JSON shape snapshots and cross-process determinism — the contract the
//! desktop app's `HyprCli::orchestrate_check` / `::orchestrate_compile`
//! will bind to. Asserts on key names and types, not on values, per the
//! pattern `src/commands/storage/info.rs`'s key-pinning tests use: a
//! rename here is a breaking change for the app, and this is where it
//! gets caught.

mod common;

use std::path::{Path, PathBuf};

use common::{isolated_dirs, run};
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    repo_root().join("assets/claude/skills/research_codebase/SKILL.md")
}

fn claude_agents() -> PathBuf {
    repo_root().join("assets/claude/agents")
}

/// `--agents-dir` requires exactly one target, so exercising two targets
/// at once (claude + opencode) means installing both registries for real
/// under the isolated `HOME`, rather than pointing `--agents-dir`
/// anywhere.
fn install_agent_registries(xdg: &Path) {
    let claude_dest = xdg.join(".claude").join("agents");
    std::fs::create_dir_all(&claude_dest).unwrap();
    for entry in std::fs::read_dir(claude_agents()).unwrap().flatten() {
        std::fs::copy(entry.path(), claude_dest.join(entry.file_name())).unwrap();
    }

    let opencode_dest = xdg.join(".config").join("opencode").join("agents");
    std::fs::create_dir_all(&opencode_dest).unwrap();
    for entry in std::fs::read_dir(repo_root().join("assets/opencode/agents"))
        .unwrap()
        .flatten()
    {
        std::fs::copy(entry.path(), opencode_dest.join(entry.file_name())).unwrap();
    }
}

fn check_json(xdg: &Path) -> Value {
    install_agent_registries(xdg);
    let out = run(
        xdg,
        &[
            "orchestrate",
            "check",
            fixture().to_str().unwrap(),
            "--json",
            "--target",
            "claude",
            "--target",
            "opencode",
        ],
    );
    // Deliberately not asserting exit status here: opencode is expected
    // to fail this fixture (the ten-agent gap), which is fine — this
    // function only cares about the JSON shape.
    serde_json::from_slice(&out.stdout).expect("check --json must always emit valid JSON")
}

const PINS: [&str; 4] = [
    "--fact",
    "exit0(git merge-base --is-ancestor HEAD @{u})=true",
    "--fact",
    "backend=git",
];

fn compile_json(xdg: &Path, target: &str) -> Value {
    let fixture_path = fixture();
    let claude_agents_path = claude_agents();
    let mut args = vec![
        "orchestrate",
        "compile",
        fixture_path.to_str().unwrap(),
        "--target",
        target,
        "--agents-dir",
        claude_agents_path.to_str().unwrap(),
        "--areas",
        "4",
        "--request",
        "map the PTY stack",
        "--no-probe",
    ];
    args.extend_from_slice(&PINS);
    let out = run(xdg, &args);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "compile --target {target} must emit valid JSON: {e}\nstdout: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

#[test]
fn check_json_carries_the_documented_top_level_keys() {
    let (_guard, xdg) = isolated_dirs();
    let d = check_json(&xdg);

    assert!(d["version"].is_number());
    assert!(d["ok"].is_boolean());
    let files = d["files"].as_array().expect("files[] must be an array");
    assert!(!files.is_empty());
    let file = &files[0];
    assert!(file["file"].is_string());
    assert!(file["ok"].is_boolean());
    assert!(file["errors"].is_number());
    assert!(file["warnings"].is_number());
    let findings = file["findings"]
        .as_array()
        .expect("findings[] must be an array");
    assert!(
        !findings.is_empty(),
        "checks 1-5 must still produce findings on this fixture"
    );
    for f in findings {
        assert!(
            f["target"].is_null(),
            "checks 1-5 findings must carry target: null"
        );
    }

    let targets = file["targets"]
        .as_array()
        .expect("targets[] must be an array");
    assert_eq!(
        targets.len(),
        2,
        "one entry per active target (claude, opencode)"
    );
    for t in targets {
        assert!(t["target"].is_string());
        assert!(t["ok"].is_boolean());
        assert!(t["errors"].is_number());
        assert!(t["warnings"].is_number());
        assert!(t["findings"].is_array());
    }

    // findings[] shape: severity, check, target, step, line, col, message, hint.
    let sample = findings
        .iter()
        .find(|f| !f["hint"].is_null())
        .expect("at least one finding should carry a hint");
    assert!(sample["severity"].is_string());
    assert!(sample["check"].is_number());
    assert!(sample["step"].is_string() || sample["step"].is_null());
    assert!(sample["line"].is_number() || sample["line"].is_null());
    assert!(sample["col"].is_number() || sample["col"].is_null());
    assert!(sample["message"].is_string());
    assert!(sample["hint"].is_string());
}

#[test]
fn compile_json_carries_the_documented_top_level_keys() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(&xdg, "claude");

    for key in [
        "version",
        "skill",
        "target",
        "source",
        "stepCount",
        "waveCount",
        "totalSpawns",
        "waves",
        "skipped",
        "guards",
        "unresolved",
        "planHash",
    ] {
        assert!(d.get(key).is_some(), "missing top-level key `{key}` in {d}");
    }
    assert!(d["waves"].is_array());
    assert!(d["skipped"].is_array());
    assert!(d["guards"].is_array());
    assert!(d["unresolved"].is_array());
    assert!(d["planHash"].is_string());
}

#[test]
fn every_guard_value_is_one_of_three_strings() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(&xdg, "claude");
    for g in d["guards"].as_array().unwrap() {
        let v = g["value"]
            .as_str()
            .unwrap_or_else(|| panic!("guard value must be a string: {g}"));
        assert!(
            matches!(v, "true" | "false" | "unknown"),
            "unexpected guard value {v:?} in {g}"
        );
    }
    for s in d["skipped"].as_array().unwrap() {
        let v = s["value"]
            .as_str()
            .unwrap_or_else(|| panic!("skipped value must be a string: {s}"));
        assert!(
            matches!(v, "true" | "false" | "unknown"),
            "unexpected skipped value {v:?} in {s}"
        );
    }
}

#[test]
fn every_guard_leaf_carries_a_via_provenance_string() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(&xdg, "claude");
    let mut saw_any_leaf = false;
    for g in d["guards"].as_array().unwrap() {
        for leaf in g["leaves"]
            .as_array()
            .expect("guard.leaves must be an array")
        {
            saw_any_leaf = true;
            assert!(leaf["key"].is_string(), "leaf: {leaf}");
            assert!(
                leaf["via"].is_string() && !leaf["via"].as_str().unwrap().is_empty(),
                "leaf: {leaf}"
            );
            // value is Some(...) or null (unresolved) — either is valid,
            // but the key must always be present.
            assert!(leaf.get("value").is_some(), "leaf: {leaf}");
        }
    }
    assert!(
        saw_any_leaf,
        "expected at least one guard leaf on this fixture"
    );
}

#[test]
fn plan_hash_is_a_sha256_hex_string_with_the_sha256_prefix() {
    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(&xdg, "claude");
    let hash = d["planHash"].as_str().unwrap();
    assert!(hash.starts_with("sha256:"), "got {hash:?}");
    let hex_part = &hash["sha256:".len()..];
    assert_eq!(hex_part.len(), 64, "got {hex_part:?}");
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "got {hex_part:?}"
    );
}

/// `plan_hash_is_a_sha256_hex_string_with_the_sha256_prefix` pins the
/// shape and `the_same_argv_produces_the_same_bytes...` pins determinism,
/// but neither pins what the digest is *of* — a regression hashing the
/// source path, or a constant, satisfies both. This recomputes it from
/// the emitted plan the way an outside consumer would: drop `planHash`,
/// re-serialize, sha256. That is what makes the artifact independently
/// verifiable rather than merely stable, which is the claim README makes
/// for it.
///
/// `serde_json::Map` is a `BTreeMap` here (no `preserve_order` feature),
/// so `to_string` is key-sorted and canonical — the recompute needs no
/// separate canonicalization step.
#[test]
fn the_plan_hash_is_the_digest_of_the_plan_with_plan_hash_removed() {
    use sha2::{Digest, Sha256};

    let (_guard, xdg) = isolated_dirs();
    let d = compile_json(&xdg, "claude");
    let emitted = d["planHash"].as_str().expect("planHash").to_string();

    let mut without = d.clone();
    without
        .as_object_mut()
        .expect("plan is an object")
        .remove("planHash")
        .expect("planHash must be present before removal");

    let compact = serde_json::to_string(&without).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(compact.as_bytes());
    let recomputed = format!("sha256:{}", hex::encode(hasher.finalize()));

    assert_eq!(
        emitted, recomputed,
        "planHash is not the digest of the rest of the plan"
    );
}

#[test]
fn the_same_block_compiled_for_two_targets_has_two_different_hashes() {
    let (_guard, xdg) = isolated_dirs();
    let claude_plan = compile_json(&xdg, "claude");
    let opencode_plan = compile_json(&xdg, "opencode");
    assert_ne!(claude_plan["planHash"], opencode_plan["planHash"]);
    assert_eq!(claude_plan["target"], "claude");
    assert_eq!(opencode_plan["target"], "opencode");
}

#[test]
fn the_same_argv_produces_the_same_bytes_from_two_separate_processes() {
    let (_guard, xdg) = isolated_dirs();
    let fixture_path = fixture();
    let claude_agents_path = claude_agents();
    let mut args = vec![
        "orchestrate",
        "compile",
        fixture_path.to_str().unwrap(),
        "--target",
        "claude",
        "--agents-dir",
        claude_agents_path.to_str().unwrap(),
        "--areas",
        "4",
        "--request",
        "map the PTY stack",
        "--no-probe",
    ];
    args.extend_from_slice(&PINS);
    let out1 = run(&xdg, &args);
    let out2 = run(&xdg, &args);
    assert!(out1.status.success());
    assert!(out2.status.success());
    assert_eq!(
        out1.stdout, out2.stdout,
        "two separate process runs diverged"
    );
}

#[test]
fn the_plan_hash_is_stable_across_processes() {
    let (_guard, xdg) = isolated_dirs();
    let d1 = compile_json(&xdg, "claude");
    let d2 = compile_json(&xdg, "claude");
    assert_eq!(d1["planHash"], d2["planHash"]);
}
