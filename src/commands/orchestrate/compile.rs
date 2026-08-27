use anyhow::{Context, Result, bail};
use colored::Colorize;

use crate::cli::OrchestrateCompileArgs;
use crate::orchestrate::block;
use crate::orchestrate::expr;
use crate::orchestrate::facts::{self, FactInputs};
use crate::orchestrate::plan;
use crate::orchestrate::schedule::{self, Schedule};
use crate::orchestrate::target::Target;

pub fn compile(args: OrchestrateCompileArgs) -> Result<()> {
    let OrchestrateCompileArgs {
        file,
        request,
        request_file,
        mut fanout,
        areas,
        fact,
        no_probe,
        agents_dir: _agents_dir,
        target,
        human,
    } = args;

    // --areas N desugars to --fanout areas=N. Both given is an error —
    // defaulting to one would silently produce a wrong plan.
    if let Some(n) = areas {
        if fanout
            .iter()
            .any(|f| f.trim_start().starts_with("areas=") || f.trim() == "areas")
        {
            bail!("--areas and --fanout areas=N both given; use one or the other");
        }
        fanout.push(format!("areas={n}"));
    }

    let request_text = match (request, request_file) {
        (Some(r), None) => Some(r),
        (None, Some(path)) => Some(
            std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?,
        ),
        (None, None) => None,
        // clap's `conflicts_with` already rejects both being set.
        (Some(_), Some(_)) => unreachable!("--request and --request-file are mutually exclusive"),
    };

    let resolved_target = resolve_single_target(&target)?;

    let src = std::fs::read_to_string(&file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let block = block::parse(&src).map_err(|e| anyhow::anyhow!("{}", e.message()))?;

    let mut exprs = Vec::new();
    for step in &block.steps {
        if let Some(when) = &step.when {
            let sid = step.id.clone().unwrap_or_default();
            let parsed = expr::parse(&when.value)
                .map_err(|e| anyhow::anyhow!("step `{sid}`: when: {}", e.message))?;
            exprs.push(parsed);
        }
    }

    let fact_inputs = FactInputs {
        fact_flags: &fact,
        request: request_text.as_deref(),
        no_probe,
    };
    let env = facts::build(&exprs, &fact_inputs)?;
    let fanout_sizes = facts::build_fanout_sizes(&fanout)?;

    let sched = schedule::schedule(&block.steps, &env, &fanout_sizes)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let skill_name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let plan_value = plan::build(
        &block,
        &sched,
        &skill_name,
        resolved_target,
        &file.display().to_string(),
    );

    // `compile` emits pretty-printed JSON on stdout by default — the plan
    // is a machine artifact whose intended use is `> plan.json`. This
    // deliberately inverts `storage info`'s default, where `--json` opts
    // in; here `--human` opts out.
    if human {
        print_human(&sched);
    } else {
        println!("{}", serde_json::to_string_pretty(&plan_value)?);
    }
    Ok(())
}

/// `compile` takes exactly one target — a plan is executed by a single
/// harness. Without an explicit target, Claude is the stable default;
/// installation no longer records a selected harness.
fn resolve_single_target(target: &[Target]) -> Result<Target> {
    // Deduplicate first: clap's `Vec<Target>` accepts repeats, and
    // `--target claude --target claude` still names exactly one harness,
    // so it should not trip the "more than one" error.
    let mut unique: Vec<Target> = target.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() > 1 {
        bail!(
            "compile takes exactly one --target (got {}); a plan is executed by a single harness",
            unique.len()
        );
    }
    if let Some(&t) = unique.first() {
        return Ok(t);
    }

    Ok(Target::Claude)
}

fn print_human(sched: &Schedule) {
    for (i, wave) in sched.waves.iter().enumerate() {
        let ids: Vec<String> = wave
            .iter()
            .map(|s| s.step.id.clone().unwrap_or_default())
            .collect();
        println!(
            "{} {}",
            format!("w{}:", i + 1).yellow().bold(),
            ids.join(", ")
        );
    }
    if !sched.skipped.is_empty() {
        println!();
        println!("{}", "skipped:".bright_black());
        for s in &sched.skipped {
            println!(
                "  {} — {} ({})",
                s.step.id.clone().unwrap_or_default().cyan(),
                s.when_expr,
                s.value.as_str()
            );
        }
    }
}
