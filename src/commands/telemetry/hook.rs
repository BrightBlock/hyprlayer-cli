//! `telemetry hook {install,uninstall,status}` — manages the
//! hyprlayer Stop-hook entry in `~/.claude/settings.json`.

use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::{TelemetryHookInstallArgs, TelemetryHookStatusArgs, TelemetryHookUninstallArgs};
use crate::secure_fs::open_secure;

/// Human-facing suffix shown in `status --json` and the bare-name
/// install historically used pre-quoting. Recognition of the various
/// real-world command spellings goes through `is_hyprlayer_hook`.
const HOOK_COMMAND_SUFFIX: &str = "hyprlayer telemetry record-from-hook";
const HOOK_TIMEOUT: u64 = 30;

/// Recognize a hyprlayer Stop-hook entry in any spelling we've ever
/// shipped: bare name (`hyprlayer telemetry record-from-hook`),
/// unquoted absolute path (v1.5.4 / PR #52), single-quoted Unix path,
/// and double-quoted Windows path. The dedup, removal, and
/// is-installed predicates all key on this so a v1.6.0 → v1.6.x →
/// v1.6.y upgrade chain replaces the entry in place rather than
/// landing duplicates.
///
/// Anchored on argv: argv[0]'s basename must be exactly `hyprlayer`
/// (or `hyprlayer.exe`) and the remaining args must be literally
/// `telemetry record-from-hook`. Unrelated user wrappers like
/// `/opt/hyprlayer-tools/custom telemetry record-from-hook` or
/// `myhyprlayer telemetry record-from-hook` are *not* claimed.
fn is_hyprlayer_hook(cmd: &str) -> bool {
    let Some((argv0, rest)) = split_argv0(cmd) else {
        return false;
    };
    if rest != "telemetry record-from-hook" {
        return false;
    }
    let basename = argv0
        .rsplit_once(['/', '\\'])
        .map(|(_, base)| base)
        .unwrap_or(argv0);
    matches!(basename, "hyprlayer" | "hyprlayer.exe")
}

/// Split a hook command string into `(argv0, rest)`. Strips matching
/// outer quotes (single or double) from argv0; doesn't attempt to
/// honor shell escapes inside the quoted region. Paths containing
/// embedded `'` or `"` are uncommon enough that we accept missed
/// dedup (they'll show up as duplicate entries on next install)
/// rather than implementing a full shell parser here.
fn split_argv0(cmd: &str) -> Option<(&str, &str)> {
    let cmd = cmd.trim_start();
    match cmd.chars().next()? {
        q @ ('\'' | '"') => {
            let close = cmd[1..].find(q)?;
            let argv0 = &cmd[1..1 + close];
            let rest = cmd[1 + close + 1..].trim_start();
            Some((argv0, rest))
        }
        _ => {
            let sp = cmd.find(' ')?;
            Some((&cmd[..sp], cmd[sp + 1..].trim_start()))
        }
    }
}

pub fn install_cmd(_args: TelemetryHookInstallArgs) -> Result<()> {
    install_at(&settings_path()?)
}

pub fn uninstall_cmd(_args: TelemetryHookUninstallArgs) -> Result<()> {
    uninstall_at(&settings_path()?)
}

pub fn status_cmd(args: TelemetryHookStatusArgs) -> Result<()> {
    let path = settings_path()?;
    let installed = is_installed_at(&path);
    if args.json {
        let payload = json!({
            "installed": installed,
            "settings_path": path,
            "command_suffix": HOOK_COMMAND_SUFFIX,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if installed {
        println!("hook installed at {}", path.display());
    } else {
        println!("hook not installed (settings: {})", path.display());
    }
    Ok(())
}

pub fn settings_path() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve $HOME for ~/.claude/settings.json"))?;
    Ok(home.join(".claude").join("settings.json"))
}

/// Absolute path to the currently-running hyprlayer binary, used as the
/// hook command. Falls back to the bare name if `current_exe` fails
/// (e.g. on a platform without /proc).
///
/// The path is shell-quoted so installations under directories with
/// spaces (e.g. `C:\Program Files\…` on Windows or any spaced $HOME on
/// Unix) produce a command the Claude Code shell parses as one argv[0].
fn hook_command() -> String {
    match std::env::current_exe() {
        Ok(p) => format!(
            "{} telemetry record-from-hook",
            shell_quote(&p.display().to_string())
        ),
        Err(_) => HOOK_COMMAND_SUFFIX.to_string(),
    }
}

/// Quote a single argv token for the shell that runs Claude Code hooks.
///
/// On Unix, hooks run via `/bin/sh -c`; we wrap in single quotes and
/// escape any embedded `'` as the standard `'\''` dance.
///
/// On Windows, hooks run via `cmd.exe /c`; double quotes are sufficient
/// because filesystem rules forbid `"` in paths, so a path can't break
/// out of the quoted region. cmd.exe's metacharacters (`& | < > ^ %`)
/// are inert inside double quotes.
#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(windows)]
fn shell_quote(s: &str) -> String {
    format!("\"{s}\"")
}

pub fn install_at(path: &Path) -> Result<()> {
    let mut root = read_or_default(path)?;
    if insert_hook(&mut root, &hook_command())? {
        write_atomic(path, &root)?;
    }
    Ok(())
}

pub fn uninstall_at(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_or_default(path)?;
    if remove_hook(&mut root) {
        write_atomic(path, &root)?;
    }
    Ok(())
}

pub fn is_installed_at(path: &Path) -> bool {
    let Ok(root) = read_or_default(path) else {
        return false;
    };
    has_hook(&root)
}

fn read_or_default(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("{} is not valid JSON: {e}", path.display()))
}

/// Refuse to write a settings.json owned by a different uid.
#[cfg(unix)]
fn ensure_owner_safe(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if !path.exists() {
        return Ok(());
    }
    let meta = std::fs::metadata(path)?;
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        anyhow::bail!(
            "{} is owned by uid {}, refusing to write as uid {}",
            path.display(),
            meta.uid(),
            euid
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_safe(_path: &Path) -> Result<()> {
    Ok(())
}

/// Atomic write via `secure_fs::open_secure` with `create_new` so the
/// temp file is created with `O_NOFOLLOW | O_EXCL` (mode 0600 on Unix).
/// PID + nanosecond suffix avoids collision with parallel installs.
fn write_atomic(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    ensure_owner_safe(path)?;
    let suffix = format!("tmp.{}.{}", std::process::id(), nanos_since_epoch());
    let tmp = path.with_extension(suffix);
    let pretty = serde_json::to_string_pretty(root)?;
    let result = (|| -> std::io::Result<()> {
        let mut f = open_secure(&tmp, |o| {
            o.write(true).create_new(true);
        })?;
        f.write_all(pretty.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

fn nanos_since_epoch() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Insert our hook entry into the settings tree. Returns `Ok(true)` if
/// the tree changed and needs writing back, `Ok(false)` if our entry was
/// already present, and `Err` if pre-existing user data has the wrong
/// type (refusing to clobber). `null` is treated as "no value yet" and
/// promoted to the default empty container.
fn insert_hook(root: &mut Value, command: &str) -> Result<bool> {
    if !root.is_object() {
        *root = json!({});
    }
    let obj = root.as_object_mut().expect("just promoted to object");
    if matches!(obj.get("hooks"), Some(Value::Null)) {
        obj.insert("hooks".into(), json!({}));
    }
    let hooks_entry = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks_entry.as_object_mut().ok_or_else(|| {
        anyhow::anyhow!("settings.json `hooks` field is not an object; refusing to overwrite")
    })?;
    if matches!(hooks.get("Stop"), Some(Value::Null)) {
        hooks.insert("Stop".into(), json!([]));
    }
    let stop_entry = hooks.entry("Stop").or_insert_with(|| json!([]));
    let stop = stop_entry.as_array_mut().ok_or_else(|| {
        anyhow::anyhow!("settings.json `hooks.Stop` field is not an array; refusing to overwrite")
    })?;

    // Replace any existing entry whose command matches our suffix
    // (covers v1.6.0 bare-name installs being upgraded to absolute-path).
    let mut replaced_in_place = false;
    for group in stop.iter_mut() {
        let Some(arr) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for handler in arr.iter_mut() {
            let Some(existing) = handler.get("command").and_then(Value::as_str) else {
                continue;
            };
            if existing == command {
                return Ok(false);
            }
            if is_hyprlayer_hook(existing) {
                handler["command"] = json!(command);
                replaced_in_place = true;
            }
        }
    }
    if replaced_in_place {
        return Ok(true);
    }

    stop.push(json!({
        "hooks": [{"type": "command", "command": command, "timeout": HOOK_TIMEOUT}]
    }));
    Ok(true)
}

fn remove_hook(root: &mut Value) -> bool {
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(stop) = hooks.get_mut("Stop").and_then(Value::as_array_mut) else {
        return false;
    };

    let mut changed = false;
    for group in stop.iter_mut() {
        let Some(arr) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = arr.len();
        arr.retain(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .is_none_or(|c| !is_hyprlayer_hook(c))
        });
        if arr.len() != before {
            changed = true;
        }
    }
    if changed {
        stop.retain(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        });
        if stop.is_empty() {
            hooks.remove("Stop");
        }
        if hooks.is_empty()
            && let Some(obj) = root.as_object_mut()
        {
            obj.remove("hooks");
        }
    }
    changed
}

fn has_hook(root: &Value) -> bool {
    let Some(stop) = root.pointer("/hooks/Stop").and_then(Value::as_array) else {
        return false;
    };
    stop.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|arr| {
            arr.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_hyprlayer_hook)
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const TEST_CMD: &str = "/test/abs/hyprlayer telemetry record-from-hook";

    fn write_settings(dir: &Path, contents: &str) -> PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn install_at_test(path: &Path) -> Result<()> {
        let mut root = read_or_default(path)?;
        if insert_hook(&mut root, TEST_CMD)? {
            write_atomic(path, &root)?;
        }
        Ok(())
    }

    #[test]
    fn install_creates_settings_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.json");
        install_at_test(&path).unwrap();
        assert!(is_installed_at(&path));
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(
            stop[0].pointer("/hooks/0/command").and_then(Value::as_str),
            Some(TEST_CMD)
        );
    }

    #[test]
    fn install_twice_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        install_at_test(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        install_at_test(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn install_writes_settings_with_0600_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        install_at_test(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "settings.json must be 0600, got {mode:o}"
        );
    }

    #[test]
    fn install_upgrades_bare_command_to_absolute_path() {
        // A v1.6.0 user has the bare-name hook installed. Re-running
        // install with an absolute path should rewrite that entry,
        // not append a duplicate.
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"hyprlayer telemetry record-from-hook","timeout":30}]}]}}"#,
        );
        install_at_test(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 1, "expected in-place upgrade, got {stop:?}");
        assert_eq!(
            stop[0].pointer("/hooks/0/command").and_then(Value::as_str),
            Some(TEST_CMD)
        );
    }

    #[test]
    fn install_preserves_unrelated_hooks() {
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{
              "hooks": {
                "Stop": [
                  {"hooks": [{"type": "command", "command": "user-thing", "timeout": 5}]}
                ],
                "PreToolUse": [
                  {"matcher": "*", "hooks": [{"type": "command", "command": "lint"}]}
                ]
              },
              "model": "sonnet"
            }"#,
        );
        install_at_test(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root.get("model").and_then(Value::as_str), Some("sonnet"));
        assert!(root.pointer("/hooks/PreToolUse").is_some());
        let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 2);
        let commands: Vec<&str> = stop
            .iter()
            .flat_map(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|h| h.get("command").and_then(Value::as_str))
            .collect();
        assert!(commands.contains(&"user-thing"));
        assert!(commands.contains(&TEST_CMD));
    }

    #[test]
    fn install_appends_new_group_rather_than_merging() {
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"a"},{"type":"command","command":"b"}]}]}}"#,
        );
        install_at_test(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 2);
    }

    #[test]
    fn install_errors_on_wrong_typed_hooks_value() {
        let dir = tempdir().unwrap();
        let path = write_settings(dir.path(), r#"{"hooks": "i am a string"}"#);
        let err = install_at_test(&path).unwrap_err().to_string();
        assert!(err.contains("`hooks` field is not an object"), "{err}");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            root.get("hooks").and_then(Value::as_str),
            Some("i am a string")
        );
        assert!(!is_installed_at(&path));
    }

    #[test]
    fn install_errors_on_wrong_typed_stop_value() {
        let dir = tempdir().unwrap();
        let path = write_settings(dir.path(), r#"{"hooks": {"Stop": "wrong"}}"#);
        let err = install_at_test(&path).unwrap_err().to_string();
        assert!(err.contains("`hooks.Stop` field is not an array"), "{err}");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            root.pointer("/hooks/Stop").and_then(Value::as_str),
            Some("wrong")
        );
        assert!(!is_installed_at(&path));
    }

    #[test]
    fn install_errors_on_array_typed_hooks_value() {
        let dir = tempdir().unwrap();
        let path = write_settings(dir.path(), r#"{"hooks": [1, 2, 3]}"#);
        assert!(install_at_test(&path).is_err());
    }

    #[test]
    fn install_errors_on_object_typed_stop_value() {
        let dir = tempdir().unwrap();
        let path = write_settings(dir.path(), r#"{"hooks": {"Stop": {"a": 1}}}"#);
        assert!(install_at_test(&path).is_err());
    }

    #[test]
    fn install_promotes_null_hooks_to_object() {
        // `null` is the only non-object the user might reasonably write
        // intending "no hooks." Treat it as `{}` and continue.
        let dir = tempdir().unwrap();
        let path = write_settings(dir.path(), r#"{"hooks": null}"#);
        install_at_test(&path).unwrap();
        assert!(is_installed_at(&path));
    }

    #[test]
    fn uninstall_removes_bare_and_absolute_path_entries() {
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{"hooks":{"Stop":[
                {"hooks":[{"type":"command","command":"hyprlayer telemetry record-from-hook"}]},
                {"hooks":[{"type":"command","command":"/abs/path/hyprlayer telemetry record-from-hook","timeout":30}]},
                {"hooks":[{"type":"command","command":"unrelated"}]}
            ]}}"#,
        );
        uninstall_at(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(
            stop[0].pointer("/hooks/0/command").and_then(Value::as_str),
            Some("unrelated")
        );
    }

    #[test]
    fn uninstall_drops_empty_stop_array() {
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"hyprlayer telemetry record-from-hook"}]}]}}"#,
        );
        uninstall_at(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root.get("hooks").is_none(), "{root}");
    }

    #[test]
    fn uninstall_on_missing_file_is_noop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.json");
        uninstall_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn is_installed_returns_false_for_missing_file() {
        let dir = tempdir().unwrap();
        assert!(!is_installed_at(&dir.path().join("nope.json")));
    }

    #[test]
    fn is_hyprlayer_hook_recognizes_every_install_format() {
        assert!(is_hyprlayer_hook("hyprlayer telemetry record-from-hook"));
        assert!(is_hyprlayer_hook(
            "/usr/local/bin/hyprlayer telemetry record-from-hook"
        ));
        assert!(is_hyprlayer_hook(
            "'/Users/jt/Some User/hyprlayer' telemetry record-from-hook"
        ));
        assert!(is_hyprlayer_hook(
            r#""C:\Program Files\hyprlayer\hyprlayer.exe" telemetry record-from-hook"#
        ));
    }

    #[test]
    fn is_hyprlayer_hook_rejects_unrelated_commands() {
        assert!(!is_hyprlayer_hook("/usr/bin/lint"));
        assert!(!is_hyprlayer_hook("hyprlayer telemetry flush"));
        assert!(!is_hyprlayer_hook(
            "/path/to/other-tool telemetry record-from-hook"
        ));
    }

    #[test]
    fn is_hyprlayer_hook_rejects_user_wrappers_with_hyprlayer_in_path() {
        // Regression: previous loose `contains("hyprlayer")` check
        // would clobber any user hook whose path mentioned hyprlayer.
        assert!(!is_hyprlayer_hook(
            "/opt/hyprlayer-tools/custom telemetry record-from-hook"
        ));
        assert!(!is_hyprlayer_hook(
            "/usr/local/bin/myhyprlayer telemetry record-from-hook"
        ));
        assert!(!is_hyprlayer_hook(
            "/usr/local/bin/hyprlayer-extra telemetry record-from-hook"
        ));
    }

    #[test]
    fn is_hyprlayer_hook_rejects_extra_args_or_wrong_subcommand() {
        assert!(!is_hyprlayer_hook(
            "/usr/local/bin/hyprlayer telemetry record-from-hook --debug"
        ));
        assert!(!is_hyprlayer_hook("/usr/local/bin/hyprlayer telemetry on"));
        assert!(!is_hyprlayer_hook("/usr/local/bin/hyprlayer"));
    }

    #[test]
    fn split_argv0_handles_every_quoting_style() {
        assert_eq!(
            split_argv0("hyprlayer telemetry record-from-hook"),
            Some(("hyprlayer", "telemetry record-from-hook"))
        );
        assert_eq!(
            split_argv0("/abs/hyprlayer telemetry record-from-hook"),
            Some(("/abs/hyprlayer", "telemetry record-from-hook"))
        );
        assert_eq!(
            split_argv0("'/with space/hyprlayer' telemetry record-from-hook"),
            Some(("/with space/hyprlayer", "telemetry record-from-hook"))
        );
        assert_eq!(
            split_argv0(r#""C:\Program Files\hyprlayer.exe" telemetry record-from-hook"#),
            Some((
                r"C:\Program Files\hyprlayer.exe",
                "telemetry record-from-hook"
            ))
        );
        assert_eq!(split_argv0(""), None);
        assert_eq!(split_argv0("'unterminated telemetry"), None);
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_wraps_path_with_spaces_in_single_quotes() {
        assert_eq!(
            shell_quote("/Users/Some User/hyprlayer"),
            "'/Users/Some User/hyprlayer'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        assert_eq!(
            shell_quote("/path/o'malley/bin"),
            "'/path/o'\\''malley/bin'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hook_command_quotes_path_with_spaces() {
        // Smoke test — the helper must produce something the shell can
        // parse into our suffix as a single argv[0].
        let cmd = hook_command();
        assert!(cmd.ends_with(" telemetry record-from-hook"), "{cmd}");
        if let Ok(exe) = std::env::current_exe()
            && exe.display().to_string().contains(' ')
        {
            assert!(cmd.starts_with('\''), "spaced path must be quoted: {cmd}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_wraps_windows_path_in_double_quotes() {
        assert_eq!(
            shell_quote(r"C:\Program Files\hyprlayer\hyprlayer.exe"),
            r#""C:\Program Files\hyprlayer\hyprlayer.exe""#
        );
    }

    #[test]
    fn install_round_trip_preserves_other_top_level_keys() {
        let dir = tempdir().unwrap();
        let path = write_settings(
            dir.path(),
            r#"{"theme":"dark","statusLine":{"command":"x"}}"#,
        );
        install_at_test(&path).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root.get("theme").and_then(Value::as_str), Some("dark"));
        assert!(root.pointer("/statusLine/command").is_some());
        assert!(is_installed_at(&path));
    }
}
