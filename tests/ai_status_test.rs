//! Human-facing `hyprlayer ai status` output stays compact in the common case.

mod common;
use common::*;

use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn status_collapses_matching_version_fields_and_hides_a_stale_sha() {
    let (_dir, xdg) = isolated_dirs();
    let config_path = xdg.join("status-config.json");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let config = serde_json::json!({
        "version": 4,
        "lastAgentCheck": now,
        "agentsInstalledSha": "d705a48094606e267f817226f553a5c5a9764072",
        "agentsInstalledVersion": env!("CARGO_PKG_VERSION"),
        "disableUpdateCheck": true,
        "telemetry": {
            "mode": "off",
            "installationId": "deadbeef-dead-beef-dead-beefdeadbeef",
            "deviceSalt": "00000000000000000000000000000000",
            "apiKeySource": "default",
            "lastFlush": 0,
            "lastConfigRefresh": 0,
            "lastEnrollmentCheck": now,
        }
    });
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let out = run(
        &xdg,
        &[
            "ai",
            "status",
            "--config-file",
            config_path.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();

    assert!(
        stdout.contains(&format!("  Version: {}", env!("CARGO_PKG_VERSION"))),
        "{stdout}"
    );
    assert!(stdout.contains("  Claude Code:"), "{stdout}");
    assert!(stdout.contains("  Codex:"), "{stdout}");
    for redundant in [
        "AI Platforms:",
        "Desired assets:",
        "Assets version:",
        "Binary version:",
        "Bundle SHA:",
        "d705a48",
    ] {
        assert!(
            !stdout.contains(redundant),
            "found `{redundant}` in:\n{stdout}"
        );
    }
}
