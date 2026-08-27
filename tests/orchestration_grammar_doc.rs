//! Pins `assets/claude/skills/_thoughts/orchestration-runtime.md`'s
//! generated region to the binary's own `orchestrate grammar --markdown`
//! output, so the vendored doc can never silently drift from the grammar
//! the parser actually implements.

mod common;

use std::path::PathBuf;

use common::{isolated_dirs, run};

const START_MARKER: &str = "<!-- generated: hyprlayer orchestrate grammar --markdown -->";
const END_MARKER: &str = "<!-- /generated -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn runtime_doc() -> String {
    let path = repo_root()
        .join("assets")
        .join("claude")
        .join("skills")
        .join("_thoughts")
        .join("orchestration-runtime.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"))
}

#[test]
fn generated_region_matches_the_grammar_generator() {
    let doc_path = repo_root()
        .join("assets")
        .join("claude")
        .join("skills")
        .join("_thoughts")
        .join("orchestration-runtime.md");
    let content = runtime_doc();

    let start = content
        .find(START_MARKER)
        .unwrap_or_else(|| panic!("{doc_path:?} missing start marker {START_MARKER:?}"));
    let after_start = start + START_MARKER.len();
    let end = content[after_start..]
        .find(END_MARKER)
        .map(|i| after_start + i)
        .unwrap_or_else(|| panic!("{doc_path:?} missing end marker {END_MARKER:?}"));

    // The region is the marker line's trailing newline through the byte
    // just before the end marker's own line.
    let region = &content[after_start + 1..end];

    let (_guard, xdg) = isolated_dirs();
    let out = run(&xdg, &["orchestrate", "grammar", "--markdown"]);
    assert!(
        out.status.success(),
        "orchestrate grammar --markdown failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let generated = String::from_utf8(out.stdout).expect("stdout must be UTF-8");

    assert_eq!(
        region, generated,
        "orchestration-runtime.md's generated region is stale.\n\
         Fix: hyprlayer orchestrate grammar --markdown, then paste between\n\
         <!-- generated: ... --> and <!-- /generated --> in\n\
         assets/claude/skills/_thoughts/orchestration-runtime.md"
    );
}

#[test]
fn codex_fanout_scratch_contract_is_no_clobber_and_cleanup_safe() {
    let content = runtime_doc();
    for required in [
        "Create it with\n   no-clobber semantics (`mkdir`, never `mkdir -p`)",
        "Never enter, reuse, or delete an existing candidate",
        "zero-padded counters (`0001`, `0002`, ...)",
        "Never use a raw fan-out value",
        "Record the exact path\n   created successfully",
        "have a canonical parent\n   equal to canonical `<cwd>`",
        "have a basename beginning\n   `.hyprlayer-fanout-`",
        "Never reconstruct its path from a step ID",
        "leave the directory in place and report its path and the failed\n   check",
    ] {
        assert!(
            content.contains(required),
            "orchestration runtime lost fan-out scratch safety contract: {required:?}"
        );
    }
}
