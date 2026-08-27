//! Integration tests for `hyprlayer ai reinstall` with no network available.
//!
//! A complete verified local generation must repair all live link farms
//! without touching GitHub. With no local generation, the release metadata
//! and asset downloads are genuinely network-bound; that path must fail
//! cleanly, print an actionable message rather than panicking, and leave the
//! destination directory untouched (Phase 4's staging guarantee — see
//! `2026-08-19-agent-bundle-archive-download.md`).
//!
//! Linux-only: it runs the binary inside an unprivileged network namespace
//! (`unshare --user --net`) with no interfaces up, so every connection
//! attempt fails immediately (`ENETUNREACH`) instead of hanging on a DNS
//! timeout. Skips itself (rather than failing) when `unshare` isn't usable
//! — some sandboxed/hardened CI environments restrict unprivileged user
//! namespaces.

#![cfg(target_os = "linux")]

mod common;
use common::*;

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[test]
fn reinstall_with_no_network_fails_cleanly_and_leaves_dest_untouched() {
    if !unshare_net_available() {
        eprintln!(
            "skipping: `unshare --user --net` is not usable in this environment \
             (restricted user namespaces) — cannot isolate network for this test"
        );
        return;
    }

    let (_dir, xdg) = isolated_dirs();
    write_reinstall_config(&xdg);

    let dest = xdg.join(".claude");
    assert!(!dest.exists(), "destination must not pre-exist");

    let out = run_without_network(&xdg, &["ai", "reinstall"]);
    assert!(
        !out.status.success(),
        "reinstall should fail with no network: {out:?}"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.to_lowercase().contains("panicked"),
        "should fail cleanly, not panic: {stderr}"
    );
    assert!(
        stderr.contains("Error"),
        "should print an actionable error message: {stderr}"
    );

    assert!(
        !dest.exists(),
        "a fully offline install must never create the destination directory \
         (staging guarantee) — found one at {}",
        dest.display()
    );
}

#[test]
fn reinstall_with_a_complete_local_generation_succeeds_without_network() {
    if !unshare_net_available() {
        eprintln!(
            "skipping: `unshare --user --net` is not usable in this environment \
             (restricted user namespaces) — cannot isolate network for this test"
        );
        return;
    }

    let (_dir, xdg) = isolated_dirs();
    write_reinstall_config(&xdg);
    write_local_generation(&xdg);

    let out = run_without_network(&xdg, &["ai", "reinstall"]);
    assert!(
        out.status.success(),
        "reinstall should repair from the local store: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("Downloading"),
        "a local repair must not claim to download: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(xdg.join(".claude/skills/code_review").is_symlink());
    assert!(xdg.join(".agents/skills/code_review").is_symlink());
    assert!(xdg.join(".codex/skills/code_review").is_symlink());
    assert!(xdg.join(".codex/agents/codebase-locator.toml").is_symlink());
}

#[test]
fn forced_reinstall_bypasses_a_complete_local_generation() {
    if !unshare_net_available() {
        eprintln!(
            "skipping: `unshare --user --net` is not usable in this environment \
             (restricted user namespaces) — cannot isolate network for this test"
        );
        return;
    }

    let (_dir, xdg) = isolated_dirs();
    write_reinstall_config(&xdg);
    write_local_generation(&xdg);

    let out = run_without_network(&xdg, &["ai", "reinstall", "-f"]);
    assert!(
        !out.status.success(),
        "forced reinstall should attempt the unavailable network"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Downloading"),
        "forced reinstall should bypass the local store: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!xdg.join(".claude").exists());
}

fn unshare_net_available() -> bool {
    Command::new("unshare")
        .args(["--user", "--net", "--map-root-user", "--", "true"])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn write_reinstall_config(xdg: &Path) {
    let dir = xdg.join("hyprlayer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 4,
            "ai": { "agentTool": "claude" },
            "thoughts": {
                "user": "test",
                "backend": {
                    "kind": "git",
                    "thoughtsRepo": "/tmp/hyprlayer-offline-test-nonexistent",
                    "reposDir": "repos",
                    "globalDir": "global",
                },
            },
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_local_generation(xdg: &Path) {
    let version = env!("CARGO_PKG_VERSION");
    let generation = xdg.join("hyprlayer/agents").join(version);
    write_bundle(
        &generation.join("claude"),
        "claude",
        version,
        &[
            ("agents/codebase-locator.md", b"locator\n"),
            ("skills/code_review/SKILL.md", b"review\n"),
        ],
    );
    write_bundle(
        &generation.join("codex"),
        "codex",
        version,
        &[(
            "agents/codebase-locator.toml",
            b"name = \"codebase-locator\"\n",
        )],
    );
}

fn write_bundle(root: &Path, harness: &str, version: &str, files: &[(&str, &[u8])]) {
    let manifest_files: Vec<_> = files
        .iter()
        .map(|(relative, bytes)| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
            serde_json::json!({
                "path": relative,
                "sha256": hex::encode(Sha256::digest(bytes)),
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "version": version,
        "harness": harness,
        "min_cli_version": "1.6.0",
        "files": manifest_files,
    });
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

/// Like `common::run`, but inside a network namespace with no interfaces
/// up (loopback only, and it's left down).
fn run_without_network(xdg: &Path, args: &[&str]) -> std::process::Output {
    Command::new("unshare")
        .args(["--user", "--net", "--map-root-user", "--", hyprlayer_bin()])
        .args(args)
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", xdg)
        .env("HYPRLAYER_DISABLE_BACKGROUND_FLUSH", "1")
        .env_remove("HYPRLAYER_TELEMETRY_KEY")
        .env_remove("HYPRLAYER_ORG_ID")
        .output()
        .expect("unshare should be runnable — checked by unshare_net_available")
}
