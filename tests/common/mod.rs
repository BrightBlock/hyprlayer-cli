//! Shared helpers for integration tests. Cargo treats `tests/common/`
//! as test-support (not a separate test target) so each `tests/<x>.rs`
//! that does `mod common;` gets its own copy without spawning a binary.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn hyprlayer_bin() -> &'static str {
    env!("CARGO_BIN_EXE_hyprlayer")
}

pub fn isolated_dirs() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let xdg = dir.path().to_path_buf();
    (dir, xdg)
}

pub fn write_opted_in_config(xdg: &Path) {
    write_config(
        xdg,
        serde_json::json!({
            "version": 4,
            "telemetry": {
                "mode": "anonymous",
                "installationId": "11111111-2222-3333-4444-555555555555",
                "deviceSalt": "deadbeefdeadbeefdeadbeefdeadbeef",
                "apiKeySource": "default",
                "lastFlush": 0,
                "lastConfigRefresh": 0,
            }
        }),
    );
}

pub fn write_opted_out_config(xdg: &Path) {
    // `installationId` makes auto-enroll see this as sticky-off
    // (explicit `telemetry off`) rather than a pristine install.
    write_config(
        xdg,
        serde_json::json!({
            "version": 4,
            "telemetry": {
                "mode": "off",
                "installationId": "deadbeef-dead-beef-dead-beefdeadbeef",
                "deviceSalt": "00000000000000000000000000000000",
                "apiKeySource": "default",
                "lastFlush": 0,
                "lastConfigRefresh": 0,
                "lastEnrollmentCheck": 9_999_999_999u64,
            }
        }),
    );
}

fn write_config(xdg: &Path, cfg: serde_json::Value) {
    let dir = xdg.join("hyprlayer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        serde_json::to_string_pretty(&cfg).unwrap(),
    )
    .unwrap();
}

/// macOS resolves `dirs::config_dir` to `~/Library/Application Support`
/// regardless of `XDG_CONFIG_HOME`; write to both layouts so the same
/// test passes on Linux and macOS.
pub fn write_config_in_both_layouts(xdg: &Path, opted_in: bool) {
    let writer = if opted_in {
        write_opted_in_config
    } else {
        write_opted_out_config
    };
    writer(xdg);
    let mac_root = xdg.join("Library").join("Application Support");
    std::fs::create_dir_all(&mac_root).unwrap();
    writer(&mac_root);
}

pub fn locate_spool(xdg: &Path) -> PathBuf {
    let candidates = [
        xdg.join("hyprlayer").join("telemetry").join("spool.jsonl"),
        xdg.join("Library")
            .join("Application Support")
            .join("hyprlayer")
            .join("telemetry")
            .join("spool.jsonl"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

pub fn read_spool_events(path: &Path) -> Vec<serde_json::Value> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("spool line must parse"))
        .collect()
}

pub fn run(xdg: &Path, args: &[&str]) -> std::process::Output {
    run_inner(xdg, None, args)
}

/// `run()` with the child's working directory pinned too.
///
/// Every agent registry's first search path is cwd-relative
/// (`./.claude/agents`, `./.opencode/agents`, `./opencode.json`,
/// `./.codex/agents`), and cargo runs integration tests with cwd set to
/// the manifest directory — so an isolated `HOME` alone does not isolate
/// target discovery. A developer whose checkout contains a `.claude/`
/// directory (which `.gitignore` ignores, so it is invisible to git) makes
/// `claude` resolve as installed and fails the default-target tests.
///
/// This is deliberately not folded into `run()`: `dispatch_instrumentation`
/// runs `thoughts status`, which resolves the current repo from cwd and
/// needs the real one.
pub fn run_in(xdg: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    run_inner(xdg, Some(cwd), args)
}

fn run_inner(xdg: &Path, cwd: Option<&Path>, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(hyprlayer_bin());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.args(args)
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", xdg)
        // Keep the detached background `telemetry flush` from racing the
        // spool these tests assert on (and from POSTing to PostHog).
        .env("HYPRLAYER_DISABLE_BACKGROUND_FLUSH", "1")
        .env_remove("HYPRLAYER_TELEMETRY_KEY")
        .env_remove("HYPRLAYER_ORG_ID");
    cmd.output().expect("hyprlayer binary should be runnable")
}
