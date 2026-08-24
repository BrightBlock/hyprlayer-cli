//! The `hyprlayer orchestrate` library: grammar, parser, evaluator, block
//! model, checks, agent resolution, scheduler, and plan emission for a
//! skill's declarative `orchestration:` block.
//!
//! This tree is a validator and a planner, never an execution engine — see
//! "What We're NOT Doing" in the plan this was built from. `src/orchestrate/`
//! holds the logic; `src/commands/orchestrate/` holds the three thin CLI
//! handlers that call into it.

pub mod agent_names;
pub mod block;
pub mod check;
pub mod eval;
pub mod expr;
pub mod facts;
pub mod grammar;
pub mod plan;
pub mod schedule;
pub mod target;
