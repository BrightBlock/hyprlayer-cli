//! Guards the freeze described in `assets/FROZEN.md`: the root `claude/`
//! tree serves pre-1.6.0 Claude clients whose download
//! paths are hardcoded (`repo_dir()`, `src/agents.rs:113-119`) and must
//! never drift from the recorded state again.
//!
//! Pinned against the recorded freeze SHA rather than `origin/master`, so
//! this keeps holding as `master` advances past the freeze point. A
//! deliberate future change to a frozen tree updates `FROZEN_SHA` below and
//! the SHA recorded in `assets/FROZEN.md` together, in the same commit.
//!
//! Skips cleanly (rather than failing) when git metadata or the freeze
//! commit itself is unavailable — e.g. a shallow CI checkout — matching the
//! skip-not-fail convention in `tests/reinstall_offline_test.rs`.

use std::path::PathBuf;
use std::process::Command;

/// `origin/master` HEAD at freeze time. See `assets/FROZEN.md`.
const FROZEN_SHA: &str = "3253eacedc83659790f00f40dec74f84e49beb56";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn freeze_commit_available() -> bool {
    Command::new("git")
        .current_dir(repo_root())
        .args(["cat-file", "-e", &format!("{FROZEN_SHA}^{{commit}}")])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn root_trees_are_byte_identical_to_the_freeze_sha() {
    if !git_available() {
        eprintln!("skipping: `git` is not usable in this environment");
        return;
    }
    if !repo_root().join(".git").exists() {
        eprintln!("skipping: {} is not a git checkout", repo_root().display());
        return;
    }
    if !freeze_commit_available() {
        eprintln!(
            "skipping: freeze commit {FROZEN_SHA} is not present locally \
             (likely a shallow clone) — cannot diff against it"
        );
        return;
    }

    let out = Command::new("git")
        .current_dir(repo_root())
        .args(["diff", "--quiet", FROZEN_SHA, "--", "claude/"])
        .output()
        .expect("git should be runnable — checked by git_available");

    assert!(
        out.status.success(),
        "root claude/ has drifted from the freeze commit {FROZEN_SHA}; \
         this tree is frozen (see \
         assets/FROZEN.md) — either revert the drift or, if the change was \
         deliberate, update FROZEN_SHA here and the SHA recorded in \
         assets/FROZEN.md together.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
