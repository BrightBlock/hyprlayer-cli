//! Pins `claude/skills/_thoughts/orchestration-runtime.md`'s generated
//! region to the binary's own `orchestrate grammar --markdown` output, so
//! the vendored doc can never silently drift from the grammar the parser
//! actually implements.

mod common;

use std::path::PathBuf;

use common::{isolated_dirs, run};

const START_MARKER: &str = "<!-- generated: hyprlayer orchestrate grammar --markdown -->";
const END_MARKER: &str = "<!-- /generated -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn generated_region_matches_the_grammar_generator() {
    let doc_path = repo_root()
        .join("claude")
        .join("skills")
        .join("_thoughts")
        .join("orchestration-runtime.md");
    let content = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("failed to read {doc_path:?}: {e}"));

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
         claude/skills/_thoughts/orchestration-runtime.md"
    );
}
