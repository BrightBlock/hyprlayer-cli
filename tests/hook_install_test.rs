//! Integration tests for `hyprlayer telemetry hook {install,uninstall,status}`.

mod common;
use common::*;

use std::path::{Path, PathBuf};

const HOOK_SUFFIX: &str = "hyprlayer telemetry record-from-hook";

/// Mirror of the production `is_hyprlayer_hook` predicate, kept here
/// so the integration test stays free of `crate::`-internal imports.
/// The unit tests in `hook.rs` pin the exact recognizer behavior;
/// this just needs to count our entries among possibly-quoted forms.
fn is_hyprlayer_hook(cmd: &str) -> bool {
    let cmd = cmd.trim_start();
    let (argv0, rest) = match cmd.chars().next() {
        Some(q @ ('\'' | '"')) => {
            let Some(close) = cmd[1..].find(q) else {
                return false;
            };
            (&cmd[1..1 + close], cmd[1 + close + 1..].trim_start())
        }
        Some(_) => {
            let Some(sp) = cmd.find(' ') else {
                return false;
            };
            (&cmd[..sp], cmd[sp + 1..].trim_start())
        }
        None => return false,
    };
    if rest != "telemetry record-from-hook" {
        return false;
    }
    let basename = argv0
        .rsplit_once(['/', '\\'])
        .map(|(_, b)| b)
        .unwrap_or(argv0);
    matches!(basename, "hyprlayer" | "hyprlayer.exe")
}

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
    cmds.iter().filter(|c| is_hyprlayer_hook(c)).count()
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
    let cmd = cmds.iter().find(|c| is_hyprlayer_hook(c)).unwrap();
    // Absolute path either bare (legacy) or single-quoted on Unix; both
    // produce a leading `/` or `'`.
    assert!(
        cmd.starts_with('/') || cmd.starts_with('\''),
        "hook command should embed an absolute path, got `{cmd}`"
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
    assert!(is_hyprlayer_hook(cmd), "{cmd}");
    assert!(
        cmd.starts_with('/') || cmd.starts_with('\''),
        "expected absolute path: {cmd}"
    );
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

/// Config whose bundle refresh is backed off, so the startup repair is the
/// only path that can write the hook.
fn write_backed_off_config(home: &Path, mode: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = home.join("hook-config.json");
    let cfg = serde_json::json!({
        "version": 4,
        "lastAgentCheck": now,
        "disableUpdateCheck": true,
        "telemetry": {
            "mode": mode,
            "installationId": "11111111-2222-3333-4444-555555555555",
            "deviceSalt": "deadbeefdeadbeefdeadbeefdeadbeef",
            "apiKeySource": "default",
            "lastFlush": 0,
            "lastConfigRefresh": 0,
            "lastEnrollmentCheck": now,
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
    path
}

/// Drop our Stop entry the way a settings rewrite would, keeping the user's.
fn delete_hyprlayer_entry(settings: &Path) {
    let mut root = read_json(settings);
    let stop = root
        .pointer_mut("/hooks/Stop")
        .and_then(|v| v.as_array_mut())
        .unwrap();
    stop.retain(|g| {
        !g.pointer("/hooks/0/command")
            .and_then(|v| v.as_str())
            .is_some_and(is_hyprlayer_hook)
    });
    std::fs::write(settings, serde_json::to_string_pretty(&root).unwrap()).unwrap();
    assert_eq!(
        count_hyprlayer_hooks(&stop_commands(&read_json(settings))),
        0
    );
}

#[test]
fn startup_restores_a_deleted_hook_only_while_recording() {
    let (_dir, home) = isolated_dirs();
    let settings = settings_path(&home);
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user-thing"}]}],"PreToolUse":[{"matcher":"*","hooks":[{"type":"command","command":"lint"}]}]},"model":"sonnet"}"#,
    )
    .unwrap();
    let recording = write_backed_off_config(&home, "anonymous");
    let rec = recording.to_str().unwrap();
    assert!(
        run(
            &home,
            &["telemetry", "hook", "install", "--config-file", rec]
        )
        .status
        .success()
    );
    delete_hyprlayer_entry(&settings);

    let out = run(&home, &["telemetry", "status", "--config-file", rec]);
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.lines().count(),
        1,
        "expected one stderr line: {stderr}"
    );
    assert!(
        stderr.contains("Restored the hyprlayer Stop hook"),
        "{stderr}"
    );
    let root = read_json(&settings);
    let cmds = stop_commands(&root);
    assert_eq!(count_hyprlayer_hooks(&cmds), 1, "{cmds:?}");
    assert!(cmds.contains(&"user-thing".to_string()));
    assert!(root.pointer("/hooks/PreToolUse").is_some());
    assert_eq!(root.get("model").and_then(|v| v.as_str()), Some("sonnet"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&settings).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "got {mode:o}");
    }

    // Present: nothing written, nothing printed.
    let before = std::fs::read(&settings).unwrap();
    let out = run(&home, &["telemetry", "status", "--config-file", rec]);
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(std::fs::read(&settings).unwrap(), before);

    // Telemetry off: stays deleted.
    delete_hyprlayer_entry(&settings);
    let off = write_backed_off_config(&home, "off");
    let out = run(
        &home,
        &[
            "telemetry",
            "status",
            "--config-file",
            off.to_str().unwrap(),
        ],
    );
    assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
    assert_eq!(
        count_hyprlayer_hooks(&stop_commands(&read_json(&settings))),
        0
    );

    // Deliberate uninstall while recording: stays uninstalled until `hook install`.
    let recording = write_backed_off_config(&home, "anonymous");
    let rec = recording.to_str().unwrap();
    assert!(
        run(
            &home,
            &["telemetry", "hook", "install", "--config-file", rec]
        )
        .status
        .success()
    );
    assert!(
        run(
            &home,
            &["telemetry", "hook", "uninstall", "--config-file", rec]
        )
        .status
        .success()
    );
    assert_eq!(
        read_json(&recording).pointer("/telemetry/hookOptOut"),
        Some(&serde_json::json!(true))
    );
    let out = run(&home, &["telemetry", "status", "--config-file", rec]);
    assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
    assert_eq!(
        count_hyprlayer_hooks(&stop_commands(&read_json(&settings))),
        0
    );
    assert!(
        run(
            &home,
            &["telemetry", "hook", "install", "--config-file", rec]
        )
        .status
        .success()
    );
    assert!(
        read_json(&recording)
            .pointer("/telemetry/hookOptOut")
            .is_none()
    );
    assert_eq!(
        count_hyprlayer_hooks(&stop_commands(&read_json(&settings))),
        1
    );
}

#[test]
fn startup_repair_leaves_a_wrongly_typed_hooks_value_alone() {
    let (_dir, home) = isolated_dirs();
    let settings = settings_path(&home);
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    for body in [
        r#"{"hooks": "i am a string"}"#,
        r#"{"hooks": {"Stop": {"a": 1}}}"#,
    ] {
        std::fs::write(&settings, body).unwrap();
        let cfg = write_backed_off_config(&home, "anonymous");
        let out = run(
            &home,
            &[
                "telemetry",
                "status",
                "--config-file",
                cfg.to_str().unwrap(),
            ],
        );
        assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), body);
    }
}
