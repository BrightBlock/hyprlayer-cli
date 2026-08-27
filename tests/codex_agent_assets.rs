//! Dev-only generator and drift guard for Codex custom-agent assets.
//!
//! `assets/claude/agents/*.md` is the source of truth. Running this test
//! rewrites `assets/codex/agents/*.toml` to the deterministic projection the
//! installer ships, and removes generated TOMLs whose Claude source vanished.
//! A source edit therefore leaves the generated companion file dirty in git,
//! just like the repository's other checked-in generated artifacts.

use saphyr::{LoadableYamlNode, MarkedYaml};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
struct ClaudeAgent {
    name: String,
    description: String,
    tools: Vec<String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CodexAgent {
    name: String,
    description: String,
    developer_instructions: String,
    sandbox_mode: Option<String>,
}

#[derive(Debug, Default)]
struct OrchestrationAgentRequirements {
    /// Every persona named by either `agent:` or `fanout:` in every skill.
    required: BTreeSet<String>,
    /// The subset that must write a file when Codex transport is active.
    fanout: BTreeSet<String>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn parse_claude_agent(path: &Path) -> ClaudeAgent {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let rest = source
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{} has no leading YAML frontmatter", path.display()));
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{} has no closing frontmatter fence", path.display()));
    // The blank separator after the closing fence is layout, not persona
    // content. Preserve every byte after it, including the final newline.
    let body = body.strip_prefix('\n').unwrap_or(body).to_string();

    // Claude accepts these historical frontmatters even though several
    // descriptions contain an unquoted `: ` and are therefore not strict
    // YAML. The fields we derive are deliberately single-line scalars, so
    // parse that established shape directly and fail on duplicates.
    let field = |key: &str| {
        let prefix = format!("{key}: ");
        let values: Vec<&str> = frontmatter
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .collect();
        assert!(
            values.len() <= 1,
            "{} repeats frontmatter field `{key}`",
            path.display()
        );
        values.first().copied()
    };
    let required_string = |key: &str| {
        field(key)
            .unwrap_or_else(|| panic!("{} is missing `{key}`", path.display()))
            .to_string()
    };

    let tools = field("tools")
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    ClaudeAgent {
        name: required_string("name"),
        description: required_string("description"),
        tools,
        body,
    }
}

fn agent_names_in_ref(raw: &str, skill: &Path, field: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let names = match trimmed.strip_prefix("one-of") {
        Some(rest) => {
            let re = regex_lite::Regex::new(r"[\w\-]+").expect("static agent-name pattern");
            re.find_iter(rest)
                .map(|found| found.as_str().to_string())
                .collect()
        }
        None => vec![trimmed.to_string()],
    };
    assert!(
        !names.is_empty() && names.iter().all(|name| !name.is_empty()),
        "{} has an empty `{field}:` agent reference",
        skill.display()
    );
    names
}

/// Walk every checked-in skill block so a new or changed `fanout:` cannot
/// silently leave its Codex persona read-only. Direct `agent:` references are
/// collected too: the policy test below proves every required persona exists
/// and that non-fan-out, non-mutating workers remain explicitly read-only.
fn orchestration_agent_requirements(skills_dir: &Path) -> OrchestrationAgentRequirements {
    let mut skill_files: Vec<PathBuf> = fs::read_dir(skills_dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", skills_dir.display()))
        .flatten()
        .map(|entry| entry.path().join("SKILL.md"))
        .filter(|path| path.is_file())
        .collect();
    skill_files.sort();
    assert!(
        !skill_files.is_empty(),
        "{} contains no skills",
        skills_dir.display()
    );

    let mut requirements = OrchestrationAgentRequirements::default();
    for skill in skill_files {
        let source = fs::read_to_string(&skill)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", skill.display()));
        let fenced = source
            .split_once("```yaml\n")
            .and_then(|(_, rest)| rest.split_once("\n```").map(|(yaml, _)| yaml))
            .unwrap_or_else(|| panic!("{} has no YAML orchestration fence", skill.display()));
        let documents = MarkedYaml::load_from_str(fenced)
            .unwrap_or_else(|e| panic!("{} has invalid YAML: {e}", skill.display()));
        let document = documents
            .first()
            .unwrap_or_else(|| panic!("{} has an empty YAML fence", skill.display()));
        let steps = document
            .data
            .as_mapping_get("orchestration")
            .and_then(|node| node.data.as_mapping_get("steps"))
            .and_then(|node| node.data.as_sequence())
            .unwrap_or_else(|| panic!("{} has no orchestration.steps", skill.display()));

        for step in steps {
            for field in ["agent", "fanout"] {
                let Some(raw) = step
                    .data
                    .as_mapping_get(field)
                    .and_then(|node| node.data.as_str())
                else {
                    continue;
                };
                let names = agent_names_in_ref(raw, &skill, field);
                requirements.required.extend(names.iter().cloned());
                if field == "fanout" {
                    requirements.fanout.extend(names);
                }
            }
        }
    }

    requirements
}

fn has_mutating_tools(agent: &ClaudeAgent) -> bool {
    agent
        .tools
        .iter()
        .any(|tool| tool == "Write" || tool == "Edit")
}

/// Codex fan-out returns results through scratch files, so every persona
/// discovered under `fanout:` must inherit the parent sandbox. Ordinary
/// personas without Claude's Write/Edit tools remain explicitly read-only.
fn uses_read_only_sandbox(
    agent: &ClaudeAgent,
    requirements: &OrchestrationAgentRequirements,
) -> bool {
    !requirements.fanout.contains(&agent.name) && !has_mutating_tools(agent)
}

fn render_codex_agent(
    agent: &ClaudeAgent,
    requirements: &OrchestrationAgentRequirements,
) -> String {
    assert!(
        !agent.body.contains("'''"),
        "agent {} contains TOML's multiline literal delimiter",
        agent.name
    );
    let mut rendered = format!(
        "# Generated from assets/claude/agents/{}.md. Do not edit by hand.\n\
         name = {}\n\
         description = {}\n",
        agent.name,
        serde_json::to_string(&agent.name).unwrap(),
        serde_json::to_string(&agent.description).unwrap(),
    );
    if uses_read_only_sandbox(agent, requirements) {
        rendered.push_str("sandbox_mode = \"read-only\"\n");
    }
    rendered.push_str("developer_instructions = '''");
    rendered.push_str(&agent.body);
    rendered.push_str("'''\n");
    rendered
}

fn write_if_changed(path: &Path, contents: &str) {
    if fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

fn generate_codex_agents(
    claude_dir: &Path,
    codex_dir: &Path,
    requirements: &OrchestrationAgentRequirements,
) -> Vec<ClaudeAgent> {
    let mut sources: Vec<PathBuf> = fs::read_dir(claude_dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", claude_dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    sources.sort();

    let mut expected_files = BTreeSet::new();
    let mut agents = Vec::with_capacity(sources.len());
    for source in sources {
        let agent = parse_claude_agent(&source);
        let stem = source.file_stem().and_then(|stem| stem.to_str()).unwrap();
        assert_eq!(
            agent.name,
            stem,
            "{}: the verified 1:1 Codex name policy requires name == file stem",
            source.display()
        );
        let filename = format!("{}.toml", agent.name);
        expected_files.insert(filename.clone());
        write_if_changed(
            &codex_dir.join(filename),
            &render_codex_agent(&agent, requirements),
        );
        agents.push(agent);
    }

    if codex_dir.is_dir() {
        for entry in fs::read_dir(codex_dir).unwrap().flatten() {
            let path = entry.path();
            let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.extension().is_some_and(|ext| ext == "toml")
                && !expected_files.contains(filename)
            {
                fs::remove_file(&path).unwrap_or_else(|e| {
                    panic!(
                        "failed to remove stale generated file {}: {e}",
                        path.display()
                    )
                });
            }
        }
    }

    agents
}

#[test]
fn generated_codex_agent_tree_matches_claude_sources() {
    let root = repo_root();
    let claude_dir = root.join("assets/claude/agents");
    let codex_dir = root.join("assets/codex/agents");
    let requirements = orchestration_agent_requirements(&root.join("assets/claude/skills"));
    let agents = generate_codex_agents(&claude_dir, &codex_dir, &requirements);

    let source_names: BTreeSet<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
    let missing: Vec<&String> = requirements
        .required
        .iter()
        .filter(|name| !source_names.contains(name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "orchestration references personas with no Claude source: {missing:?}"
    );

    assert_eq!(agents.len(), 19, "the source agent inventory changed");
    let generated: Vec<PathBuf> = fs::read_dir(&codex_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    assert_eq!(generated.len(), agents.len());

    for agent in &agents {
        let path = codex_dir.join(format!("{}.toml", agent.name));
        let parsed: CodexAgent = toml::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{} is not valid Codex TOML: {e}", path.display()));
        assert_eq!(parsed.name, agent.name, "{}: name drift", path.display());
        assert_eq!(
            parsed.description,
            agent.description,
            "{}: description drift",
            path.display()
        );
        assert_eq!(
            parsed.developer_instructions,
            agent.body,
            "{}: persona body lost bytes",
            path.display()
        );
        assert_eq!(
            parsed.sandbox_mode.as_deref(),
            uses_read_only_sandbox(agent, &requirements).then_some("read-only"),
            "{}: read-only mapping drift",
            path.display()
        );

        if requirements.required.contains(&agent.name) {
            assert_eq!(
                parsed.sandbox_mode.is_none(),
                requirements.fanout.contains(&agent.name) || has_mutating_tools(agent),
                "{}: required persona has an unsafe sandbox policy",
                path.display()
            );
        }
    }
}

#[test]
fn write_and_edit_agents_inherit_while_ordinary_agents_stay_read_only() {
    let requirements = OrchestrationAgentRequirements::default();
    let base = ClaudeAgent {
        name: "fixture-agent".to_string(),
        description: "fixture".to_string(),
        tools: vec!["Read".to_string(), "TodoWrite".to_string()],
        body: "body\n".to_string(),
    };
    assert!(
        uses_read_only_sandbox(&base, &requirements),
        "TodoWrite is not the Write tool"
    );

    let mut with_write = base;
    with_write.tools.push("Write".to_string());
    assert!(!uses_read_only_sandbox(&with_write, &requirements));

    with_write.tools.pop();
    with_write.tools.push("Edit".to_string());
    assert!(!uses_read_only_sandbox(&with_write, &requirements));
}

#[test]
fn every_declared_fanout_persona_inherits_the_parent_sandbox() {
    let root = repo_root();
    let requirements = orchestration_agent_requirements(&root.join("assets/claude/skills"));
    assert!(
        !requirements.fanout.is_empty(),
        "the checked-in skills should exercise fan-out policy"
    );

    for name in &requirements.fanout {
        let agent =
            parse_claude_agent(&root.join("assets/claude/agents").join(format!("{name}.md")));
        assert!(
            !uses_read_only_sandbox(&agent, &requirements),
            "{name} must inherit a writable sandbox for file-mediated results"
        );
        assert!(
            !render_codex_agent(&agent, &requirements).contains("sandbox_mode"),
            "{name} TOML must omit sandbox_mode"
        );
    }
}

#[test]
fn every_read_only_fanout_persona_defines_the_transport_exception() {
    const CONTRACT: &str = "When the caller supplies a designated transient Codex fanout result path, you MUST write your final response only to that exact path.";

    let root = repo_root();
    let requirements = orchestration_agent_requirements(&root.join("assets/claude/skills"));
    let mut read_only_fanout_count = 0;
    for name in &requirements.fanout {
        let agent =
            parse_claude_agent(&root.join("assets/claude/agents").join(format!("{name}.md")));
        if has_mutating_tools(&agent) {
            continue;
        }
        read_only_fanout_count += 1;
        assert!(
            agent.body.contains(CONTRACT),
            "{name} must state the exact designated-path transport contract"
        );
        assert!(
            agent.body.contains("read-only") && agent.body.contains("transport-only"),
            "{name} must describe the exception as transport-only"
        );
    }
    assert!(
        read_only_fanout_count > 0,
        "the test inventory must exercise a read-only persona's transport exception"
    );
}

#[test]
fn hyphenated_names_are_preserved_one_to_one() {
    let requirements = OrchestrationAgentRequirements::default();
    let agent = ClaudeAgent {
        name: "codebase-locator".to_string(),
        description: "fixture".to_string(),
        tools: vec!["Read".to_string()],
        body: "body\n".to_string(),
    };
    let rendered = render_codex_agent(&agent, &requirements);
    let parsed: CodexAgent = toml::from_str(&rendered).unwrap();
    assert_eq!(parsed.name, "codebase-locator");
}
