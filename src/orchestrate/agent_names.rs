//! Agent name resolution — replacing the prototype's hand-maintained
//! `KNOWN_AGENTS` list, which drifted inside the drift-checker itself
//! (accepted `code-reviewer`, which has no file in this repo; rejected
//! `ship`, which does). Names resolve from the filesystem instead.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use saphyr::{LoadableYamlNode, MarkedYaml};

use crate::orchestrate::target::Target;

/// The result of resolving one harness's agent namespace.
pub enum AgentSource {
    Resolved(BTreeSet<String>),
    /// No directory resolved, or every resolved directory was empty.
    /// Carries the directories that were searched, for the warning message.
    None {
        searched: Vec<PathBuf>,
    },
}

/// One harness's agent namespace. Phase 2 implements Claude; Phase 3 adds
/// OpenCode and Codex, which differ in all three dimensions — directory,
/// filename convention, and where the name actually lives.
pub trait AgentRegistry {
    /// Directories consulted, in order, for the error message.
    fn search_paths(&self) -> Vec<PathBuf>;
    /// Names with no backing file (harness built-ins).
    fn builtins(&self) -> &'static [&'static str];
    fn resolve(&self, dirs: &[PathBuf]) -> AgentSource;
}

pub struct ClaudeRegistry;

impl AgentRegistry for ClaudeRegistry {
    fn search_paths(&self) -> Vec<PathBuf> {
        let mut dirs = vec![PathBuf::from("./.claude/agents")];
        if let Ok(home_dest) = crate::agents::AgentTool::Claude.dest_dir() {
            dirs.push(home_dest.join("agents"));
        }
        dirs
    }

    fn builtins(&self) -> &'static [&'static str] {
        &["general-purpose", "Explore", "Plan", "claude"]
    }

    fn resolve(&self, dirs: &[PathBuf]) -> AgentSource {
        let mut names = BTreeSet::new();
        for dir in dirs {
            names.extend(claude_agent_names_in(dir));
        }
        if names.is_empty() {
            AgentSource::None {
                searched: dirs.to_vec(),
            }
        } else {
            AgentSource::Resolved(names)
        }
    }
}

fn claude_agent_names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("md"))
        .filter_map(|e| agent_name_from_file(&e.path()))
        .collect()
}

/// Frontmatter `name:` is preferred over the file stem when they differ.
/// Verified: all 19 files in `claude/agents/` have `name:` equal to their
/// stem today, so the two agree; preferring frontmatter is the right
/// precedence for the day they diverge.
fn agent_name_from_file(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.to_string();
    let Ok(content) = std::fs::read_to_string(path) else {
        return Some(stem);
    };
    let Some(frontmatter) = extract_frontmatter(&content) else {
        return Some(stem);
    };
    let Ok(docs) = MarkedYaml::load_from_str(&frontmatter) else {
        return Some(stem);
    };
    let name = docs.into_iter().next().and_then(|doc| {
        doc.data
            .as_mapping_get("name")
            .and_then(|n| n.data.as_str().map(str::to_string))
    });
    Some(name.unwrap_or(stem))
}

/// Extracts the text between a leading `---` and the next `---` line.
/// Same saphyr crate as everything else in this feature — no second YAML
/// parser.
fn extract_frontmatter(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(rest[..end].to_string())
}

/// Returns the registry for `target`. The single factory every caller
/// that needs to go from a `Target` to its `AgentRegistry` uses — `check`
/// (per active target) and `Target::is_installed` (its own defaults).
pub fn registry_for(target: Target) -> Box<dyn AgentRegistry> {
    match target {
        Target::Claude => Box::new(ClaudeRegistry),
        Target::OpenCode => Box::new(OpenCodeRegistry),
        Target::Codex => Box::new(CodexRegistry),
    }
}

pub struct OpenCodeRegistry;

impl AgentRegistry for OpenCodeRegistry {
    fn search_paths(&self) -> Vec<PathBuf> {
        let mut dirs = vec![PathBuf::from("./.opencode/agents")];
        if let Ok(home_dest) = crate::agents::AgentTool::OpenCode.dest_dir() {
            dirs.push(home_dest.join("agents"));
        }
        dirs
    }

    fn builtins(&self) -> &'static [&'static str] {
        &["general", "explore", "scout"]
    }

    fn resolve(&self, dirs: &[PathBuf]) -> AgentSource {
        let mut names = BTreeSet::new();
        for dir in dirs {
            names.extend(opencode_agent_names_in(dir));
        }
        for json_path in opencode_json_candidates() {
            names.extend(opencode_json_agent_names(&json_path));
        }
        if names.is_empty() {
            AgentSource::None {
                searched: dirs.to_vec(),
            }
        } else {
            AgentSource::Resolved(names)
        }
    }
}

fn opencode_agent_names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("md"))
        .filter_map(|e| opencode_agent_name_from_file(&e.path()))
        .collect()
}

/// OpenCode agent names come from the **filename**, never frontmatter —
/// verified: 0 of 9 files in `opencode/agents/` carry a `name:` key.
/// Only `mode: subagent` entries count: a `mode: primary` agent cannot be
/// spawned as a subagent, so admitting it would make check 6 accept a
/// name that fails at runtime. A file with no discoverable `mode:` at all
/// is excluded rather than guessed at.
fn opencode_agent_name_from_file(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let frontmatter = extract_frontmatter(&content)?;
    let docs = MarkedYaml::load_from_str(&frontmatter).ok()?;
    let doc = docs.into_iter().next()?;
    let mode = doc
        .data
        .as_mapping_get("mode")
        .and_then(|n| n.data.as_str());
    if mode != Some("subagent") {
        return None;
    }
    path.file_stem()?.to_str().map(str::to_string)
}

/// OpenCode agents may also be declared inline in `opencode.json` under an
/// `agent` key (`{"agent": {"my-agent": {...}}}`) — checked at both the
/// project root and the global OpenCode config directory.
fn opencode_json_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from("./opencode.json")];
    if let Ok(home_dest) = crate::agents::AgentTool::OpenCode.dest_dir() {
        candidates.push(home_dest.join("opencode.json"));
    }
    candidates
}

fn opencode_json_agent_names(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    value
        .get("agent")
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

pub struct CodexRegistry;

impl AgentRegistry for CodexRegistry {
    /// Codex has no `AgentTool` variant and no hyprlayer-managed install
    /// (`src/agents.rs`'s `AgentTool` is `Claude | Copilot | OpenCode`) —
    /// its paths are constructed directly from `dirs::home_dir()`.
    /// `orchestrate` validates against the user's own registry without
    /// owning it.
    fn search_paths(&self) -> Vec<PathBuf> {
        let mut dirs = vec![PathBuf::from("./.codex/agents")];
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".codex").join("agents"));
        }
        dirs
    }

    fn builtins(&self) -> &'static [&'static str] {
        &["default", "worker", "explorer"]
    }

    fn resolve(&self, dirs: &[PathBuf]) -> AgentSource {
        let mut names = BTreeSet::new();
        for dir in dirs {
            names.extend(codex_agent_names_in(dir));
        }
        if names.is_empty() {
            AgentSource::None {
                searched: dirs.to_vec(),
            }
        } else {
            AgentSource::Resolved(names)
        }
    }
}

fn codex_agent_names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("toml"))
        .filter_map(|e| codex_agent_name_from_file(&e.path()))
        .collect()
}

/// Codex subagents are TOML, not markdown — the `name` key, falling back
/// to the file stem exactly like the Claude/frontmatter case.
fn codex_agent_name_from_file(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.to_string();
    let Ok(content) = std::fs::read_to_string(path) else {
        return Some(stem);
    };
    // `toml::Value::from_str` parses a single value, not a document — a
    // whole file needs `toml::Table`.
    let Ok(table) = content.parse::<toml::Table>() else {
        return Some(stem);
    };
    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(name.unwrap_or(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_name_is_preferred_over_the_file_stem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weird-stem.md");
        std::fs::write(&path, "---\nname: real-name\ndescription: x\n---\n\nbody\n").unwrap();
        assert_eq!(agent_name_from_file(&path), Some("real-name".to_string()));
    }

    #[test]
    fn missing_frontmatter_falls_back_to_the_stem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cartographer.md");
        std::fs::write(&path, "no frontmatter here\n").unwrap();
        assert_eq!(
            agent_name_from_file(&path),
            Some("cartographer".to_string())
        );
    }

    #[test]
    fn frontmatter_with_no_name_key_falls_back_to_the_stem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archivist.md");
        std::fs::write(&path, "---\ndescription: x\n---\n\nbody\n").unwrap();
        assert_eq!(agent_name_from_file(&path), Some("archivist".to_string()));
    }

    #[test]
    fn empty_source_is_none_not_nineteen_errors() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClaudeRegistry;
        let dirs = vec![dir.path().to_path_buf()];
        assert!(matches!(registry.resolve(&dirs), AgentSource::None { .. }));
    }

    #[test]
    fn builtins_resolve_without_a_file() {
        let registry = ClaudeRegistry;
        assert!(registry.builtins().contains(&"general-purpose"));
        assert!(registry.builtins().contains(&"Explore"));
        assert!(registry.builtins().contains(&"Plan"));
        assert!(registry.builtins().contains(&"claude"));
    }

    #[test]
    fn all_repo_agent_names_resolve_from_the_repo_directory() {
        let repo_agents = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("claude/agents");
        let registry = ClaudeRegistry;
        let AgentSource::Resolved(names) = registry.resolve(&[repo_agents]) else {
            panic!("expected the repo's claude/agents/ to resolve names");
        };
        assert!(names.contains("cartographer"));
        assert!(names.contains("ship"));
        assert!(!names.contains("code-reviewer"));
    }

    #[test]
    fn opencode_agent_names_come_from_the_filename_not_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        // File stem differs from any frontmatter field on purpose — there
        // is no `name:` key in real opencode agent files (verified: 0 of
        // 9), so the stem must be what resolves.
        let path = dir.path().join("codebase-locator.md");
        std::fs::write(&path, "---\ndescription: x\nmode: subagent\n---\n\nbody\n").unwrap();
        assert_eq!(
            opencode_agent_name_from_file(&path),
            Some("codebase-locator".to_string())
        );
    }

    #[test]
    fn an_opencode_primary_mode_agent_is_not_spawnable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build.md");
        std::fs::write(&path, "---\ndescription: x\nmode: primary\n---\n\nbody\n").unwrap();
        assert_eq!(opencode_agent_name_from_file(&path), None);
    }

    #[test]
    fn all_repo_opencode_agent_names_resolve_and_have_the_ten_agent_gap() {
        let repo_agents = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("opencode/agents");
        let registry = OpenCodeRegistry;
        let AgentSource::Resolved(names) = registry.resolve(&[repo_agents]) else {
            panic!("expected the repo's opencode/agents/ to resolve names");
        };
        assert!(names.contains("codebase-locator"));
        // The ten-agent Claude/OpenCode gap.
        assert!(!names.contains("cartographer"));
        assert!(!names.contains("archivist"));
        assert!(!names.contains("ship"));
    }

    #[test]
    fn codex_agent_names_are_read_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weird-stem.toml");
        std::fs::write(&path, "name = \"real-name\"\ndescription = \"x\"\n").unwrap();
        assert_eq!(
            codex_agent_name_from_file(&path),
            Some("real-name".to_string())
        );
    }

    #[test]
    fn codex_agent_without_a_name_key_falls_back_to_the_stem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.toml");
        std::fs::write(&path, "description = \"x\"\n").unwrap();
        assert_eq!(
            codex_agent_name_from_file(&path),
            Some("worker".to_string())
        );
    }

    #[test]
    fn codex_builtins_resolve_without_a_file() {
        let registry = CodexRegistry;
        assert!(registry.builtins().contains(&"default"));
        assert!(registry.builtins().contains(&"worker"));
        assert!(registry.builtins().contains(&"explorer"));
    }

    /// The registries' identity was previously asserted here through a
    /// `target()` accessor that nothing else called. Those three assertions
    /// were tautological — each impl returned a literal — and the invariant
    /// worth holding, that `registry_for` dispatches each `Target` to the
    /// right registry, is covered behaviourally by
    /// `orchestrate_targets.rs::a_claude_only_agent_is_an_error_for_opencode`,
    /// which drives the binary and fails on any mis-dispatch.
    #[test]
    fn each_registrys_search_paths_are_distinct() {
        assert!(
            ClaudeRegistry
                .search_paths()
                .iter()
                .any(|p| p.ends_with("agents") && p.to_string_lossy().contains(".claude"))
        );
        assert!(
            CodexRegistry
                .search_paths()
                .iter()
                .any(|p| p.ends_with("agents") && p.to_string_lossy().contains(".codex"))
        );
    }
}
