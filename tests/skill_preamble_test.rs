//! Skill preamble contract: Claude skills carry no inline beacon because
//! Claude Code's Stop hook records skill events for them.

use std::path::PathBuf;

const START_MARKER: &str = "<!-- hyprlayer:telemetry-beacon -->";
const END_MARKER: &str = "<!-- /hyprlayer:telemetry-beacon -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn every_claude_skill_has_no_preamble() {
    let dir = repo_root().join("assets").join("claude").join("skills");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("assets/claude/skills/ should exist")
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
