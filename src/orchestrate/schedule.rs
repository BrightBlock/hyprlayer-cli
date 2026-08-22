//! The wave scheduler. Two semantics here were verified empirically
//! against `research_codebase_declared` and are the only way 14 steps
//! reproduce as 7 waves — do not "simplify" either one:
//!
//!   1. Skipped steps are EXCLUDED from wave numbering. Including the
//!      skipped `follow-up` yields 8 waves, not 7.
//!   2. A skipped step counts as SATISFIED for its dependents.
//!      `verify-results` requires `[map, history, targeted, web, tickets]`;
//!      `web` and `tickets` are skipped, and `verify-results` must still
//!      become ready in the wave after map/history/targeted.

use std::collections::{BTreeMap, BTreeSet};

use crate::orchestrate::block::{AgentRef, Step};
use crate::orchestrate::eval::{self, FactEnv, Resolved, Tri};
use crate::orchestrate::expr;

#[derive(Debug, Clone)]
pub enum SpawnMode {
    Inline,
    Agent {
        name: String,
    },
    /// `agent: one-of [...]` — compile cannot pick; it is a judgment
    /// call, recorded as unresolved rather than guessed at.
    AgentChoice {
        candidates: Vec<String>,
    },
    Fanout {
        agent: String,
        over: String,
        n: usize,
    },
}

impl SpawnMode {
    pub fn count(&self) -> usize {
        match self {
            SpawnMode::Inline => 0,
            SpawnMode::Agent { .. } | SpawnMode::AgentChoice { .. } => 1,
            SpawnMode::Fanout { n, .. } => *n,
        }
    }
}

pub struct ScheduledStep<'a> {
    pub step: &'a Step,
    pub spawns: SpawnMode,
}

pub struct SkippedStep<'a> {
    pub step: &'a Step,
    pub when_expr: String,
    pub value: Tri,
}

pub struct GuardRecord<'a> {
    pub step: &'a Step,
    pub expr: String,
    pub value: Tri,
    pub leaves: Vec<Resolved>,
}

pub struct Schedule<'a> {
    /// `waves[0]` is wave 1, in declaration order within the wave.
    pub waves: Vec<Vec<ScheduledStep<'a>>>,
    pub skipped: Vec<SkippedStep<'a>>,
    pub guards: Vec<GuardRecord<'a>>,
}

#[derive(Debug)]
pub enum ScheduleError {
    BadGuard {
        step: String,
        message: String,
    },
    /// A cycle or a dangling `requires` — `check` should already have
    /// caught this; a defensive stop, not a panic, if it wasn't.
    Unresolvable {
        steps: Vec<String>,
    },
    MissingFanoutSize {
        step: String,
        over: String,
    },
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::BadGuard { step, message } => {
                write!(f, "step `{step}`: when: {message}")
            }
            ScheduleError::Unresolvable { steps } => {
                write!(
                    f,
                    "cannot schedule step(s) {} — unresolved `requires` or a cycle",
                    steps.join(", ")
                )
            }
            ScheduleError::MissingFanoutSize { step, over } => {
                write!(
                    f,
                    "step `{step}`: --fanout {over}=N required (over: {over})"
                )
            }
        }
    }
}

pub fn schedule<'a>(
    steps: &'a [Step],
    env: &FactEnv,
    fanout_sizes: &BTreeMap<String, usize>,
) -> Result<Schedule<'a>, ScheduleError> {
    let mut guards = Vec::new();
    let mut skipped: Vec<SkippedStep<'a>> = Vec::new();
    let mut included: Vec<&'a Step> = Vec::new();

    for step in steps {
        let Some(when) = &step.when else {
            included.push(step);
            continue;
        };
        let parsed = expr::parse(&when.value).map_err(|e| ScheduleError::BadGuard {
            step: step.id.clone().unwrap_or_default(),
            message: e.message,
        })?;
        let mut trace = Vec::new();
        let value = eval::eval(&parsed, env, &mut trace);
        let expr_str = parsed.to_string();
        guards.push(GuardRecord {
            step,
            expr: expr_str.clone(),
            value,
            leaves: trace,
        });
        match value {
            // `True` includes; `False` or `Unknown` skips — a guard that
            // cannot be resolved is not run. This is the conservative
            // reading and matches the observed run.
            Tri::True => included.push(step),
            Tri::False | Tri::Unknown => skipped.push(SkippedStep {
                step,
                when_expr: expr_str,
                value,
            }),
        }
    }

    // A skipped step counts as SATISFIED for its dependents (semantic 2).
    let mut done: BTreeSet<&str> = skipped
        .iter()
        .filter_map(|s| s.step.id.as_deref())
        .collect();

    let mut waves: Vec<Vec<ScheduledStep<'a>>> = Vec::new();
    let mut pending: Vec<&'a Step> = included;

    while !pending.is_empty() {
        let (ready, not_ready): (Vec<&Step>, Vec<&Step>) = pending
            .into_iter()
            .partition(|s| s.requires.iter().all(|r| done.contains(r.value.as_str())));

        if ready.is_empty() {
            return Err(ScheduleError::Unresolvable {
                steps: not_ready.iter().filter_map(|s| s.id.clone()).collect(),
            });
        }

        for s in &ready {
            if let Some(id) = s.id.as_deref() {
                done.insert(id);
            }
        }

        let mut wave_steps = Vec::with_capacity(ready.len());
        for s in ready {
            wave_steps.push(ScheduledStep {
                step: s,
                spawns: spawn_mode(s, fanout_sizes)?,
            });
        }
        // Skipped steps are EXCLUDED from wave numbering (semantic 1) —
        // `waves.len()` after this push is the count of INCLUDED waves
        // only, never incremented for a round with nothing ready.
        waves.push(wave_steps);
        pending = not_ready;
    }

    Ok(Schedule {
        waves,
        skipped,
        guards,
    })
}

fn spawn_mode(
    step: &Step,
    fanout_sizes: &BTreeMap<String, usize>,
) -> Result<SpawnMode, ScheduleError> {
    if step.inline {
        return Ok(SpawnMode::Inline);
    }
    if let Some(fanout) = &step.fanout {
        let over_name = step
            .over
            .as_ref()
            .map(|o| o.value.clone())
            .unwrap_or_default();
        let agent_name = match &fanout.value {
            AgentRef::One(n) => n.clone(),
            AgentRef::OneOf(ns) => ns.join(","),
        };
        let n = fanout_sizes.get(&over_name).copied().ok_or_else(|| {
            ScheduleError::MissingFanoutSize {
                step: step.id.clone().unwrap_or_default(),
                over: over_name.clone(),
            }
        })?;
        return Ok(SpawnMode::Fanout {
            agent: agent_name,
            over: over_name,
            n,
        });
    }
    if let Some(agent) = &step.agent {
        return Ok(match &agent.value {
            AgentRef::One(n) => SpawnMode::Agent { name: n.clone() },
            AgentRef::OneOf(ns) => SpawnMode::AgentChoice {
                candidates: ns.clone(),
            },
        });
    }
    // Neither `inline:`, `fanout:`, nor `agent:` — no delegation is named,
    // so nothing spawns.
    Ok(SpawnMode::Inline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrate::block;

    fn parse_block(body: &str) -> block::Block {
        let src = format!("---\nname: x\n---\n\n```yaml\n{body}\n```\n");
        block::parse(&src).unwrap()
    }

    #[test]
    fn a_three_step_chain_schedules_in_three_waves() {
        let b = parse_block(
            "orchestration:\n  steps:\n    - id: a\n      inline: true\n    - id: b\n      requires: [a]\n      inline: true\n    - id: c\n      requires: [b]\n      inline: true\n",
        );
        let env = FactEnv::new();
        let sched = schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        assert_eq!(sched.waves.len(), 3);
    }

    #[test]
    fn a_skipped_step_does_not_consume_a_wave_number() {
        let b = parse_block(
            "orchestration:\n  steps:\n    - id: a\n      inline: true\n    - id: b\n      requires: [a]\n      when: flag(--never)\n      inline: true\n    - id: c\n      requires: [a]\n      inline: true\n",
        );
        let env = FactEnv::new();
        let sched = schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        // b is Unknown (flag(--never) unresolved) -> skipped. Only a and c
        // are scheduled, both depending solely on nothing/a -> 2 waves.
        assert_eq!(sched.waves.len(), 2);
        assert_eq!(sched.skipped.len(), 1);
        assert_eq!(sched.skipped[0].step.id.as_deref(), Some("b"));
    }

    #[test]
    fn a_skipped_requirement_still_satisfies_its_dependent() {
        let b = parse_block(
            "orchestration:\n  steps:\n    - id: a\n      inline: true\n    - id: skip-me\n      requires: [a]\n      when: flag(--never)\n      inline: true\n    - id: c\n      requires: [a, skip-me]\n      inline: true\n",
        );
        let env = FactEnv::new();
        let sched = schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        // a -> wave 1; c requires a and skip-me, skip-me is satisfied by
        // skipping, so c lands in wave 2, not stuck forever.
        assert_eq!(sched.waves.len(), 2);
        assert_eq!(sched.waves[1][0].step.id.as_deref(), Some("c"));
    }

    #[test]
    fn a_one_of_agent_is_one_spawn_with_an_unresolved_choice() {
        let b =
            parse_block("orchestration:\n  steps:\n    - id: a\n      agent: one-of [x, y, z]\n");
        let env = FactEnv::new();
        let sched = schedule(&b.steps, &env, &BTreeMap::new()).unwrap();
        let s = &sched.waves[0][0];
        assert_eq!(s.spawns.count(), 1);
        assert!(
            matches!(&s.spawns, SpawnMode::AgentChoice { candidates } if candidates.len() == 3)
        );
    }

    #[test]
    fn a_fanout_with_no_size_binding_is_an_error_naming_the_flag() {
        let b = parse_block(
            "orchestration:\n  steps:\n    - id: a\n      fanout: cartographer\n      over: areas\n",
        );
        let env = FactEnv::new();
        match schedule(&b.steps, &env, &BTreeMap::new()) {
            Err(ScheduleError::MissingFanoutSize { step, over }) => {
                assert_eq!(step, "a");
                assert_eq!(over, "areas");
            }
            Err(other) => panic!("expected MissingFanoutSize, got {other}"),
            Ok(_) => panic!("expected an error, got Ok"),
        }
    }

    #[test]
    fn a_fanout_spawns_n_times() {
        let b = parse_block(
            "orchestration:\n  steps:\n    - id: a\n      fanout: cartographer\n      over: areas\n",
        );
        let env = FactEnv::new();
        let mut sizes = BTreeMap::new();
        sizes.insert("areas".to_string(), 4);
        let sched = schedule(&b.steps, &env, &sizes).unwrap();
        assert_eq!(sched.waves[0][0].spawns.count(), 4);
    }
}
