//! Builds the `FactEnv` `compile` schedules against: `--fact` pins, then
//! `--request` (binding `request` and every `matches()` field regardless
//! of what it's named — same porting detail as `check`), then live probes
//! unless `--no-probe`. `check` never reaches this module; it builds its
//! own minimal, execution-free environment directly in `check.rs`.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::orchestrate::eval::{self, FactEnv, FactValue};
use crate::orchestrate::expr::{self, Leaf};

pub struct FactInputs<'a> {
    /// Raw `KEY=VALUE` strings from `--fact`. Highest precedence — always
    /// wins over a probe.
    pub fact_flags: &'a [String],
    /// The resolved `--request`/`--request-file` text, if either was given.
    pub request: Option<&'a str>,
    /// Skip all probing (`available()`, `exit0()`, `backend`) — the flag
    /// a hook or an editor should pass.
    pub no_probe: bool,
}

/// Precedence, highest first:
///   1. `--fact`             via: "fact-flag"
///   2. `--request` / `--request-file` (binds `request` and every
///      `matches()` field)   via: "request-flag"
///   3. probes, unless `--no-probe`:
///        `available(x)`  → PATH lookup           via: "probe:path"
///        `exit0(cmd)`    → run it, check status  via: "probe:exec"
///        `backend`       → `HyprlayerConfig`      via: "probe:config"
///   4. nothing              → `Tri::Unknown`      via: "unresolved-default-false"
///
/// Only probes leaves that actually appear in `exprs` — a `PreToolUse`
/// hook or an editor calling this with `--no-probe` pays no execution
/// cost at all, and a guard the block never uses is never touched.
pub fn build(exprs: &[expr::Expr], inputs: &FactInputs) -> Result<FactEnv> {
    let mut env = FactEnv::new();
    let mut leaves: Vec<&Leaf> = Vec::new();
    for e in exprs {
        expr::collect_leaves(e, &mut leaves);
    }

    for raw in inputs.fact_flags {
        let (key, raw_value) = split_key_value(raw)?;
        let value = infer_fact_value(&key, &raw_value)?;
        env.set_if_absent(key, value, "fact-flag");
    }

    if let Some(text) = inputs.request {
        env.set_if_absent("request", FactValue::Str(text.to_string()), "request-flag");
        for leaf in &leaves {
            if let Leaf::Matches { field, .. } = leaf {
                env.set_if_absent(
                    field.clone(),
                    FactValue::Str(text.to_string()),
                    "request-flag",
                );
            }
        }
    }

    if !inputs.no_probe {
        for leaf in &leaves {
            match leaf {
                Leaf::Available(bin) => {
                    env.set_if_absent(
                        eval::leaf_key(leaf),
                        FactValue::Bool(probe_available(bin)),
                        "probe:path",
                    );
                }
                Leaf::Exit0(cmd) => {
                    env.set_if_absent(
                        eval::leaf_key(leaf),
                        FactValue::Bool(probe_exit0(cmd)),
                        "probe:exec",
                    );
                }
                Leaf::Comparison { path, .. } if path == "backend" => {
                    if let Some(backend) = probe_backend() {
                        env.set_if_absent("backend", FactValue::Str(backend), "probe:config");
                    }
                }
                _ => {}
            }
        }
    }

    Ok(env)
}

/// `--fanout NAME=N` sizes, kept separate from `FactEnv` — they answer
/// "how many spawns does `over: NAME` produce," not a guard's truth
/// value, so mixing them into the same lookup table would conflate two
/// different kinds of fact.
pub fn build_fanout_sizes(fanout_flags: &[String]) -> Result<BTreeMap<String, usize>> {
    let mut sizes = BTreeMap::new();
    for raw in fanout_flags {
        let (name, raw_value) = split_key_value(raw)?;
        let n: usize = raw_value.parse().map_err(|_| {
            anyhow::anyhow!("--fanout {raw:?}: size must be a non-negative integer")
        })?;
        sizes.insert(name, n);
    }
    Ok(sizes)
}

/// Splits `raw` at the first `=` that is not inside parentheses.
/// Splitting at the first `=` outright breaks `--fact 'exit0(a=b)=true'`;
/// splitting at the last breaks `--fact 'request=a=b'`. Keys are the only
/// thing that may contain parens, and values never are, so this handles
/// both.
fn split_key_value(raw: &str) -> Result<(String, String)> {
    let mut depth: i32 = 0;
    for (i, c) in raw.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 => {
                return Ok((raw[..i].to_string(), raw[i + 1..].to_string()));
            }
            _ => {}
        }
    }
    bail!("{raw:?} is missing '=' (expected KEY=VALUE)")
}

/// The key format tells us the value's type: `exists(...)`, `flag(...)`,
/// `available(...)`, and `exit0(...)` are booleans; `count(...)` is an
/// integer; anything else (a plain path or a `matches()` field name) is
/// a string.
fn infer_fact_value(key: &str, raw_value: &str) -> Result<FactValue> {
    if key.starts_with("exists(")
        || key.starts_with("flag(")
        || key.starts_with("available(")
        || key.starts_with("exit0(")
    {
        Ok(FactValue::Bool(parse_bool(raw_value)?))
    } else if key.starts_with("count(") {
        let n: i64 = raw_value
            .parse()
            .map_err(|_| anyhow::anyhow!("--fact {key}={raw_value:?} is not an integer"))?;
        Ok(FactValue::Int(n))
    } else {
        Ok(FactValue::Str(raw_value.to_string()))
    }
}

fn parse_bool(s: &str) -> Result<bool> {
    match s {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => bail!("expected true/false, got {other:?}"),
    }
}

fn probe_available(bin: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        #[cfg(windows)]
        {
            if dir.join(format!("{bin}.exe")).is_file() || dir.join(bin).is_file() {
                return true;
            }
        }
        #[cfg(not(windows))]
        {
            let candidate = dir.join(bin);
            if let Ok(meta) = std::fs::metadata(&candidate) {
                use std::os::unix::fs::PermissionsExt;
                if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                    return true;
                }
            }
        }
    }
    false
}

/// Runs a guard command for its exit status alone.
///
/// Both streams are discarded, and that is load-bearing rather than tidy:
/// `compile`'s stdout **is** the plan artifact, so a probe that inherited
/// stdout would splice its own output in front of the JSON and break the
/// byte-identical guarantee. `exit0(git log -1 --format=%ai)` does exactly
/// that — the contract of this leaf is the status, never the output.
fn probe_exit0(cmd: &str) -> bool {
    #[cfg(windows)]
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    #[cfg(not(windows))]
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.is_ok_and(|s| s.success())
}

/// Reads the effective config the same way `storage info` does
/// (`src/commands/storage/info.rs`) — the one place `orchestrate` touches
/// `HyprlayerConfig`, which is why `config_args()` returns `None` for the
/// whole group in `src/cli/mod.rs`.
fn probe_backend() -> Option<String> {
    let path = crate::config::get_default_config_path().ok()?;
    if !path.exists() {
        return None;
    }
    let cfg = crate::config::HyprlayerConfig::load(&path).ok()?;
    let current_repo = crate::config::get_current_repo_path().ok()?;
    let effective = cfg
        .thoughts
        .as_ref()?
        .effective_config_for(&current_repo.display().to_string());
    Some(effective.backend.kind().as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paren_aware_split_handles_a_parenthesized_key() {
        let (k, v) = split_key_value("exit0(a=b)=true").unwrap();
        assert_eq!(k, "exit0(a=b)");
        assert_eq!(v, "true");
    }

    #[test]
    fn paren_aware_split_handles_a_value_containing_equals() {
        let (k, v) = split_key_value("request=a=b").unwrap();
        assert_eq!(k, "request");
        assert_eq!(v, "a=b");
    }

    #[test]
    fn a_fact_beats_a_probe() {
        let exprs = vec![expr::parse("available(definitely-not-a-real-binary-xyz)").unwrap()];
        let inputs = FactInputs {
            fact_flags: &["available(definitely-not-a-real-binary-xyz)=true".to_string()],
            request: None,
            no_probe: false,
        };
        let env = build(&exprs, &inputs).unwrap();
        let entry = env
            .get("available(definitely-not-a-real-binary-xyz)")
            .unwrap();
        assert_eq!(entry.via, "fact-flag");
        assert!(matches!(entry.value, FactValue::Bool(true)));
    }

    #[test]
    fn no_probe_skips_execution_and_leaves_exit0_unresolved() {
        let exprs = vec![expr::parse("exit0(true)").unwrap()];
        let inputs = FactInputs {
            fact_flags: &[],
            request: None,
            no_probe: true,
        };
        let env = build(&exprs, &inputs).unwrap();
        assert!(env.get("exit0(true)").is_none());
    }

    #[test]
    fn request_binds_every_matches_field_regardless_of_name() {
        let exprs = vec![expr::parse(r#"matches(request, "x") and matches(topic, "y")"#).unwrap()];
        let inputs = FactInputs {
            fact_flags: &[],
            request: Some("hello world"),
            no_probe: true,
        };
        let env = build(&exprs, &inputs).unwrap();
        assert!(
            matches!(&env.get("request").unwrap().value, FactValue::Str(s) if s == "hello world")
        );
        assert!(
            matches!(&env.get("topic").unwrap().value, FactValue::Str(s) if s == "hello world")
        );
    }

    #[test]
    fn areas_and_fanout_conflict_is_rejected_by_the_cli_layer_not_here() {
        // build_fanout_sizes itself just parses NAME=N pairs; the
        // --areas/--fanout collision check lives in the CLI handler,
        // which desugars --areas into a --fanout entry before calling in.
        let sizes = build_fanout_sizes(&["areas=4".to_string()]).unwrap();
        assert_eq!(sizes.get("areas"), Some(&4));
    }
}
