//! Plan JSON and its content hash. Built with `serde_json::json!`, per
//! `src/commands/storage/info.rs`'s `build_json` — not a `Serialize`
//! derive. camelCase keys, kebab-case values (step ids are data).

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::orchestrate::block::{Block, Step};
use crate::orchestrate::eval::{FactValue, Resolved};
use crate::orchestrate::schedule::{GuardRecord, Schedule, SkippedStep, SpawnMode};
use crate::orchestrate::target::Target;

pub fn build(
    block: &Block,
    schedule: &Schedule,
    skill: &str,
    target: Target,
    source: &str,
) -> Value {
    let waves: Vec<Value> = schedule
        .waves
        .iter()
        .enumerate()
        .map(|(i, wave)| {
            json!({
                "wave": i + 1,
                "steps": wave.iter().map(|s| json!({
                    "id": s.step.id,
                    "mode": spawn_mode_str(&s.spawns),
                    "agent": spawn_agent(&s.spawns),
                    "agentCandidates": spawn_candidates(&s.spawns),
                    "over": spawn_over(&s.spawns),
                    "spawns": s.spawns.count(),
                    "retry": retry_json(s.step),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let skipped: Vec<Value> = schedule.skipped.iter().map(skipped_json).collect();
    let guards: Vec<Value> = schedule.guards.iter().map(guard_json).collect();

    // Two kinds of call the compiler declines to make, both recorded so a
    // plan reads as a to-do rather than pretending the decision was made:
    // `agent: one-of [...]` (which of these agents) and `judgment:` (should
    // this step run at all, and how). A step carrying both appears twice —
    // they are genuinely two separate decisions.
    let unresolved: Vec<Value> = schedule
        .waves
        .iter()
        .flatten()
        .flat_map(|s| {
            let mut entries = Vec::new();
            if let SpawnMode::AgentChoice { candidates } = &s.spawns {
                entries.push(json!({
                    "step": s.step.id,
                    "kind": "agent-choice",
                    "candidates": candidates,
                }));
            }
            if let Some(j) = &s.step.judgment {
                entries.push(json!({
                    "step": s.step.id,
                    "kind": "judgment",
                    "question": j.value.trim(),
                }));
            }
            entries
        })
        .collect();

    let total_spawns: usize = schedule
        .waves
        .iter()
        .flatten()
        .map(|s| s.spawns.count())
        .sum();

    let mut value = json!({
        "version": 1,
        "skill": skill,
        "target": target.as_str(),
        "source": source,
        "stepCount": block.steps.len(),
        "waveCount": schedule.waves.len(),
        "totalSpawns": total_spawns,
        "waves": waves,
        "skipped": skipped,
        "guards": guards,
        "unresolved": unresolved,
    });

    // Hash the compact serialization with `planHash` absent (it hasn't
    // been inserted yet). `serde_json::Map` is a `BTreeMap` — no
    // `preserve_order` feature — so `to_string` is already key-sorted and
    // canonical.
    let hash = plan_hash(&value);
    value["planHash"] = json!(format!("sha256:{hash}"));
    value
}

fn plan_hash(value: &Value) -> String {
    let compact = serde_json::to_string(value).expect("plan value must serialize");
    let mut hasher = Sha256::new();
    hasher.update(compact.as_bytes());
    hex::encode(hasher.finalize())
}

fn spawn_mode_str(spawns: &SpawnMode) -> &'static str {
    match spawns {
        SpawnMode::Inline => "inline",
        SpawnMode::Agent { .. } | SpawnMode::AgentChoice { .. } => "agent",
        SpawnMode::Fanout { .. } => "fanout",
    }
}

fn spawn_agent(spawns: &SpawnMode) -> Value {
    match spawns {
        SpawnMode::Agent { name } => json!(name),
        SpawnMode::Fanout { agent, .. } => json!(agent),
        SpawnMode::Inline | SpawnMode::AgentChoice { .. } => Value::Null,
    }
}

fn spawn_candidates(spawns: &SpawnMode) -> Value {
    match spawns {
        SpawnMode::AgentChoice { candidates } => json!(candidates),
        _ => Value::Null,
    }
}

fn spawn_over(spawns: &SpawnMode) -> Value {
    match spawns {
        SpawnMode::Fanout { over, .. } => json!(over),
        _ => Value::Null,
    }
}

/// `retry: {step, max}` is declared data, reported and never followed —
/// so it is emitted on the step that DECLARES it, naming its target, and
/// is deliberately absent from `totalSpawns`. A retry is contingent on a
/// failure the compiler cannot predict; folding a worst case into the
/// scheduled count would make the enumeration ("map x4, history x1, ...")
/// false and turn a plan into a ceiling. A reader budgeting for the worst
/// case reads `max` here and the target step's own `spawns`.
fn retry_json(step: &Step) -> Value {
    match &step.retry {
        Some(r) => json!({
            "step": r.step.as_ref().map(|s| s.value.clone()),
            "max": r.max,
        }),
        None => Value::Null,
    }
}

fn skipped_json(s: &SkippedStep) -> Value {
    json!({
        "id": s.step.id,
        "when": s.when_expr,
        "value": s.value.as_str(),
    })
}

fn guard_json(g: &GuardRecord) -> Value {
    json!({
        "step": g.step.id,
        "expr": g.expr,
        "value": g.value.as_str(),
        "leaves": g.leaves.iter().map(resolved_json).collect::<Vec<_>>(),
    })
}

fn resolved_json(r: &Resolved) -> Value {
    let value = match &r.value {
        Some(FactValue::Str(s)) => json!(s),
        Some(FactValue::Bool(b)) => json!(b),
        Some(FactValue::Int(n)) => json!(n),
        None => Value::Null,
    };
    json!({ "key": r.key, "value": value, "via": r.via })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrate::block;
    use crate::orchestrate::eval::FactEnv;
    use crate::orchestrate::schedule;
    use std::collections::BTreeMap;

    fn parse_block(body: &str) -> block::Block {
        let src = format!("---\nname: x\n---\n\n```yaml\n{body}\n```\n");
        block::parse(&src).unwrap()
    }

    #[test]
    fn the_plan_hash_ignores_its_own_field_and_is_deterministic() {
        let b = parse_block("orchestration:\n  steps:\n    - id: a\n      inline: true\n");
        let env = FactEnv::new();
        let sched = schedule::schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        let v1 = build(&b, &sched, "x", Target::Claude, "x.md");
        let v2 = build(&b, &sched, "x", Target::Claude, "x.md");
        assert_eq!(v1["planHash"], v2["planHash"]);
        assert!(v1["planHash"].as_str().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn two_targets_hash_differently() {
        let b = parse_block("orchestration:\n  steps:\n    - id: a\n      inline: true\n");
        let env = FactEnv::new();
        let sched = schedule::schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        let v1 = build(&b, &sched, "x", Target::Claude, "x.md");
        let v2 = build(&b, &sched, "x", Target::OpenCode, "x.md");
        assert_ne!(v1["planHash"], v2["planHash"]);
    }
}
