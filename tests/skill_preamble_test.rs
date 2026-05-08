//! Skill preamble contract: Claude skills carry no beacon (the Stop
//! hook records skill events for them); Copilot + OpenCode skills must.

use std::path::{Path, PathBuf};

const START_MARKER: &str = "<!-- hyprlayer:telemetry-beacon -->";
const END_MARKER: &str = "<!-- /hyprlayer:telemetry-beacon -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extract_preamble<'a>(content: &'a str, file: &Path) -> &'a str {
    let start = content
        .find(START_MARKER)
        .unwrap_or_else(|| panic!("{file:?} missing start marker"));
    let end = content
        .find(END_MARKER)
        .unwrap_or_else(|| panic!("{file:?} missing end marker"));
    assert!(end > start, "{file:?} markers in wrong order");
    &content[start..end + END_MARKER.len()]
}

fn assert_preamble(file: &Path, expected_skill: &str) {
    let content =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("failed to read {file:?}: {e}"));
    assert!(
        content.contains(START_MARKER) && content.contains(END_MARKER),
        "{file:?} missing beacon markers; run `bash tools/inject-telemetry-preamble.sh`"
    );
    let preamble = extract_preamble(&content, file);

    let start_needle = format!("hyprlayer telemetry skill-start --skill {expected_skill}");
    let end_needle =
        format!("hyprlayer telemetry skill-end --skill {expected_skill} --session <token>");
    assert!(
        preamble.contains(&start_needle),
        "{file:?} missing `{start_needle}`"
    );
    assert!(
        preamble.contains(&end_needle),
        "{file:?} missing `{end_needle}`"
    );

    // Bare commands work in bash + cmd + PowerShell; shell-specific
    // redirection idioms can't appear in a file shared across all three.
    let forbidden = [
        ">/dev/null",
        "2>&1",
        "*>$null",
        "2>$null",
        ">NUL",
        "|| true",
        "command -v",
    ];
    for needle in forbidden {
        assert!(
            !preamble.contains(needle),
            "{file:?} preamble has shell-specific syntax `{needle}`"
        );
    }
}

#[test]
fn every_claude_skill_has_no_preamble() {
    let dir = repo_root().join("claude").join("skills");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("claude/skills/ should exist")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && !e.file_name().to_string_lossy().starts_with('_')
        })
        .collect();
    assert!(!entries.is_empty(), "expected at least one claude skill");
    for entry in entries {
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&skill_md).unwrap();
        assert!(
            !content.contains(START_MARKER) && !content.contains(END_MARKER),
            "{skill_md:?} still carries the telemetry beacon; run \
             `python3 tools/inject-telemetry-preamble.py --uninject claude`"
        );
    }
}

#[test]
fn every_copilot_prompt_has_telemetry_preamble() {
    let dir = repo_root().join("copilot").join("prompts");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("copilot/prompts/ should exist")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".prompt.md"))
        .collect();
    assert!(!entries.is_empty(), "expected at least one copilot prompt");
    for entry in entries {
        let name_str = entry.file_name().to_string_lossy().into_owned();
        let name = name_str.trim_end_matches(".prompt.md");
        assert_preamble(&entry.path(), name);
    }
}

#[test]
fn every_opencode_command_has_telemetry_preamble() {
    let dir = repo_root().join("opencode").join("commands");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("opencode/commands/ should exist")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name().to_string_lossy().ends_with(".md")
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .collect();
    assert!(
        !entries.is_empty(),
        "expected at least one opencode command"
    );
    for entry in entries {
        let name_str = entry.file_name().to_string_lossy().into_owned();
        let name = name_str.trim_end_matches(".md");
        assert_preamble(&entry.path(), name);
    }
}

#[cfg(unix)]
#[test]
fn injection_script_is_idempotent() {
    let script = repo_root()
        .join("tools")
        .join("inject-telemetry-preamble.sh");
    if !script.exists() {
        eprintln!("inject-telemetry-preamble.sh missing; skipping idempotency check");
        return;
    }
    let out = std::process::Command::new("bash")
        .arg(&script)
        .current_dir(repo_root())
        .output()
        .expect("inject script should run");
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Updated: 0"),
        "re-run was not a no-op:\n{stdout}"
    );
}
