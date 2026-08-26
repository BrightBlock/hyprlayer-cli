//! Integration test for `hyprlayer ai reinstall` with no network available.
//!
//! The release metadata and asset downloads are genuinely network-bound, so
//! rather than mock them, this asserts the property that matters for offline
//! CI and for a user with a dead connection: a fully-offline `ai reinstall`
//! fails cleanly, prints an
//! actionable message rather than panicking, and leaves the destination
//! directory untouched (Phase 4's staging guarantee — see
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
