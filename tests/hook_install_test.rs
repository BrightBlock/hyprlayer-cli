//! Integration tests for `hyprlayer telemetry hook {install,uninstall,status}`.

mod common;
use common::*;

use std::path::{Path, PathBuf};

const HOOK_SUFFIX: &str = "hyprlayer telemetry record-from-hook";

fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

fn read_json(p: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn stop_commands(root: &serde_json::Value) -> Vec<String> {
    let Some(stop) = root.pointer("/hooks/Stop").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    stop.iter()
        .flat_map(|g| {
            g.get("hooks")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
        })
        .filter_map(|h| h.get("command").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

fn count_hyprlayer_hooks(cmds: &[String]) -> usize {
    cmds.iter().filter(|c| c.ends_with(HOOK_SUFFIX)).count()
}

#[test]
fn install_creates_settings_when_missing() {
    let (_dir, home) = isolated_dirs();
    let path = settings_path(&home);
    assert!(!path.exists());

    let out = run(&home, &["telemetry", "hook", "install"]);
    assert!(out.status.success(), "{:?}", out);
    assert!(path.exists());

    let cmds = stop_commands(&read_json(&path));
    assert_eq!(count_hyprlayer_hooks(&cmds), 1, "{cmds:?}");
    let cmd = cmds.iter().find(|c| c.ends_with(HOOK_SUFFIX)).unwrap();
    assert!(
        cmd.starts_with('/'),
        "hook command should be an absolute path, got `{cmd}`"
    );
}

#[test]
fn install_twice_no_duplicate() {
    let (_dir, home) = isolated_dirs();
    let path = settings_path(&home);
    run(&home, &["telemetry", "hook", "install"]);
    let first = std::fs::read_to_string(&path).unwrap();
    run(&home, &["telemetry", "hook", "install"]);
    let second = std::fs::read_to_string(&path).unwrap();
    assert_eq!(first, second);
    assert_eq!(count_hyprlayer_hooks(&stop_commands(&read_json(&path))), 1);
}

#[test]
fn uninstall_preserves_unrelated_hooks() {
    let (_dir, home) = isolated_dirs();
    let path = settings_path(&home);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{
          "hooks": {
            "Stop": [{"hooks": [{"type": "command", "command": "user-thing"}]}],
            "PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "lint"}]}]
          },
          "model": "sonnet"
        }"#,
    )
    .unwrap();

    run(&home, &["telemetry", "hook", "install"]);
    run(&home, &["telemetry", "hook", "uninstall"]);

    let root = read_json(&path);
    assert_eq!(root.get("model").and_then(|v| v.as_str()), Some("sonnet"));
    assert!(root.pointer("/hooks/PreToolUse").is_some());
    let cmds = stop_commands(&root);
    assert!(cmds.contains(&"user-thing".to_string()));
    assert_eq!(count_hyprlayer_hooks(&cmds), 0);
}

#[test]
fn install_upgrades_bare_command_in_existing_settings() {
    // Migration path: a v1.6.0 user has the bare-name hook installed.
    // Re-running install (e.g. via 24h auto-reinstall) replaces the
    // bare entry with the absolute-path entry — no duplicate.
    let (_dir, home) = isolated_dirs();
    let path = settings_path(&home);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"hyprlayer telemetry record-from-hook"}]}]}}"#,
    )
    .unwrap();

    run(&home, &["telemetry", "hook", "install"]);
    let root = read_json(&path);
    let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
    assert_eq!(stop.len(), 1, "expected in-place upgrade, got {stop:?}");
    let cmd = stop[0]
        .pointer("/hooks/0/command")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(cmd.ends_with(HOOK_SUFFIX));
    assert!(cmd.starts_with('/'), "expected absolute path: {cmd}");
}

#[test]
fn status_reports_install_state() {
    let (_dir, home) = isolated_dirs();
    let before = run(&home, &["telemetry", "hook", "status"]);
    assert!(before.status.success());
    assert!(
        String::from_utf8(before.stdout)
            .unwrap()
            .contains("not installed")
    );

    run(&home, &["telemetry", "hook", "install"]);
    let after = run(&home, &["telemetry", "hook", "status"]);
    assert!(
        String::from_utf8(after.stdout)
            .unwrap()
            .contains("hook installed")
    );
}

#[test]
fn status_json_round_trip() {
    let (_dir, home) = isolated_dirs();
    run(&home, &["telemetry", "hook", "install"]);
    let out = run(&home, &["telemetry", "hook", "status", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.get("installed").and_then(|x| x.as_bool()), Some(true));
    assert_eq!(
        v.get("command_suffix").and_then(|x| x.as_str()),
        Some(HOOK_SUFFIX)
    );
}

#[cfg(unix)]
#[test]
fn installed_settings_file_has_0600_perms() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, home) = isolated_dirs();
    let path = settings_path(&home);
    run(&home, &["telemetry", "hook", "install"]);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "got {mode:o}");
}
