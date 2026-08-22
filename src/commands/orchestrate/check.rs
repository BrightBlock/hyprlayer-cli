use std::io::Write;

use anyhow::{Result, bail};
use colored::Colorize;
use serde_json::json;

use crate::cli::OrchestrateCheckArgs;
use crate::orchestrate::check::{CheckOptions, Finding, Report, Severity, check_file};
use crate::orchestrate::target::Target;

pub fn check(args: OrchestrateCheckArgs) -> Result<()> {
    let OrchestrateCheckArgs {
        files,
        json: as_json,
        agents_dir,
        target,
    } = args;

    // Default is every installed target, not one: the question a skill
    // author actually has is "does this block work everywhere it will
    // run," and since OpenCode silently loads Claude's skills, a
    // single-target default would answer the less useful question by
    // omission.
    //
    // Sorted and deduplicated: clap's `Vec<Target>` accepts repeats, and
    // a repeated `--target claude --target claude` would otherwise emit
    // two identical `targets[]` blocks and run check 6 twice, breaking
    // the documented "one entry per active target" contract and
    // double-counting for any consumer summing `targets[].errors`.
    // Sorting also makes the output order independent of flag order.
    let targets: Vec<Target> = if target.is_empty() {
        Target::ALL
            .iter()
            .copied()
            .filter(Target::is_installed)
            .collect()
    } else {
        let mut t = target;
        t.sort();
        t.dedup();
        t
    };

    // A flat list of paths has no way to say which registry it belongs
    // to, so in a multi-target run it's ambiguous which target it's
    // overriding.
    if !agents_dir.is_empty() && targets.len() != 1 {
        bail!(
            "--agents-dir requires exactly one --target (got {}); pass --target <one> to disambiguate",
            targets.len()
        );
    }

    let opts = CheckOptions {
        agents_dir,
        targets: targets.clone(),
    };

    let reports: Vec<Report> = files.iter().map(|f| check_file(f, &opts)).collect();
    let ok = !reports.iter().any(Report::has_errors);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&build_json(&reports, &targets, ok))?
        );
    } else {
        print_human(&reports, &targets);
    }

    if !ok {
        // A skill with lint findings is a *successful* run of the
        // checker, not a program error — a returned `Err` would print
        // anyhow's Debug dump and mark the telemetry event `Failure`.
        // Non-zero exit without `Err`, exactly as `codex stream` does
        // (`src/commands/codex/stream.rs`).
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(1);
    }
    Ok(())
}

fn build_json(reports: &[Report], targets: &[Target], ok: bool) -> serde_json::Value {
    let files: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            let flat: Vec<&Finding> = r.findings.iter().filter(|f| f.target.is_none()).collect();
            let target_blocks: Vec<serde_json::Value> = targets
                .iter()
                .map(|&t| {
                    let own: Vec<&Finding> =
                        r.findings.iter().filter(|f| f.target == Some(t)).collect();
                    let errors = own.iter().filter(|f| f.severity == Severity::Error).count();
                    let warnings = own
                        .iter()
                        .filter(|f| f.severity == Severity::Warning)
                        .count();
                    json!({
                        "target": t.as_str(),
                        "ok": errors == 0,
                        "errors": errors,
                        "warnings": warnings,
                        "findings": own.iter().map(|f| finding_json(f)).collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "file": r.file.display().to_string(),
                "ok": !r.has_errors(),
                "errors": r.error_count(),
                "warnings": r.warning_count(),
                "findings": flat.iter().map(|f| finding_json(f)).collect::<Vec<_>>(),
                "targets": target_blocks,
            })
        })
        .collect();
    json!({ "version": 1, "ok": ok, "files": files })
}

fn finding_json(f: &Finding) -> serde_json::Value {
    json!({
        "severity": match f.severity { Severity::Error => "error", Severity::Warning => "warning" },
        "check": f.check,
        "target": f.target.map(|t| t.as_str()),
        "step": f.step,
        "line": f.line,
        "col": f.col,
        "message": f.message,
        "hint": f.hint,
    })
}

fn print_finding(finding: &Finding, file_name: &str, indent: &str) {
    let label = match finding.severity {
        Severity::Error => "error".red().to_string(),
        Severity::Warning => "warn".yellow().to_string(),
    };
    let pos = match (finding.line, finding.col) {
        (Some(l), Some(c)) => format!("{file_name}:{l}:{c}"),
        _ => file_name.to_string(),
    };
    println!(
        "{indent}{label}  [check {}] {pos}  {}",
        finding.check, finding.message
    );
    if let Some(hint) = &finding.hint {
        println!("{indent}       {}", hint.bright_black());
    }
}

fn print_human(reports: &[Report], targets: &[Target]) {
    for r in reports {
        let file_name = r
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| r.file.display().to_string());

        if r.has_errors() {
            println!("{}  {}", "FAIL".red().bold(), r.file.display());
        } else {
            println!("{}    {}", "ok".green().bold(), r.file.display());
        }

        for finding in r.findings.iter().filter(|f| f.target.is_none()) {
            print_finding(finding, &file_name, "      ");
        }

        // Group check 6 by target, so the reader can tell at a glance
        // which harness (if any) is unhappy and why.
        for &t in targets {
            let own: Vec<&Finding> = r.findings.iter().filter(|f| f.target == Some(t)).collect();
            let errors = own.iter().filter(|f| f.severity == Severity::Error).count();
            let warnings = own
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .count();
            if errors == 0 && warnings == 0 {
                println!("      {:<9} {}", t.to_string().cyan(), "ok".green());
                continue;
            }
            println!("      {:<9}", t.to_string().cyan());
            for finding in own {
                print_finding(finding, &file_name, "                ");
            }
        }
    }
}
