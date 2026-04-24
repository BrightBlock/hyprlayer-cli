use anyhow::Result;
use colored::Colorize;
use std::process::Command;

use super::{BackendContext, StatusReport, ThoughtsBackend};
use crate::agents::AgentTool;
use crate::config::BackendKind;

/// The Anytype MCP server command the agent tool invokes.
const ANYTYPE_MCP_COMMAND: &str = "npx";
const ANYTYPE_MCP_ARGS: &[&str] = &["-y", "@any-org/anytype-mcp"];
const ANYTYPE_MCP_NAME: &str = "anytype";

/// Default name of the env var holding the Anytype API key when the user
/// doesn't specify one. Referenced from config defaults, init prompts, and
/// the Copilot settings snippet to avoid drift.
pub const DEFAULT_ANYTYPE_TOKEN_ENV: &str = "ANYTYPE_API_KEY";

pub struct AnytypeBackend;

impl ThoughtsBackend for AnytypeBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        ctx.effective
            .backend_settings
            .validate_for(BackendKind::Anytype)?;

        let hooks_updated = crate::hooks::setup_git_hooks(ctx.code_repo, false)?;
        if !hooks_updated.is_empty() {
            println!(
                "{}",
                format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow()
            );
        }

        // Stale `thoughts/` from a prior filesystem backend: warn, don't
        // auto-remove. Use `symlink_metadata` so broken symlinks still trip.
        let stale = ctx.code_repo.join("thoughts");
        if stale.symlink_metadata().is_ok() {
            println!(
                "{}",
                format!(
                    "ℹ️  A `thoughts/` directory exists at {} — likely stale symlinks from a \
                     previous backend. Anytype content lives in the app, not on disk. \
                     You can `rm -rf thoughts/` once you're sure you don't need the old links.",
                    stale.display()
                )
                .bright_black()
            );
        }

        let agent = ctx.agent_tool.ok_or_else(|| {
            anyhow::anyhow!("AI tool not configured. Run 'hyprlayer ai configure' first.")
        })?;

        if is_anytype_mcp_registered(agent) {
            println!(
                "{}",
                format!("✓ Anytype MCP already wired up with {agent} — skipping registration")
                    .green()
            );
        } else {
            let env_var = ctx
                .effective
                .backend_settings
                .api_token_env
                .as_deref()
                .unwrap_or(DEFAULT_ANYTYPE_TOKEN_ENV);
            if std::env::var(env_var).is_err() {
                println!(
                    "{}",
                    format!(
                        "⚠️  Env var {} is not set in the current shell — set it before starting your AI tool. \
                         Issue an API key in the Anytype app under Settings → API Keys.",
                        env_var
                    )
                    .yellow()
                );
            }
            register_anytype_mcp(agent, env_var)?;
        }

        if ctx
            .effective
            .backend_settings
            .type_id
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            println!(
                "{}",
                "No Anytype type configured yet. Your first /create_plan (or similar) in \
                 this repo will create a HyprlayerThought type in the configured space and \
                 persist the ID."
                    .bright_black()
            );
        }

        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
        println!(
            "{}",
            "Anytype backend — the AI agent reads and writes directly via MCP".bright_black()
        );
        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        let mut lines = Vec::new();
        let settings = &ctx.effective.backend_settings;

        let space = settings.space_id.as_deref().unwrap_or("(not set)");
        lines.push(format!("  Space ID: {}", space.cyan()));

        match settings.type_id.as_deref() {
            Some(id) if !id.is_empty() => lines.push(format!("  Type ID: {}", id.cyan())),
            _ => lines.push(format!(
                "  Type ID: {}",
                "(will be created on first write)".bright_black()
            )),
        }

        if let Some(name) = settings.api_token_env.as_deref() {
            let set = std::env::var(name).is_ok();
            let status = if set {
                "✓ set".green().to_string()
            } else {
                "✗ not set".red().to_string()
            };
            lines.push(format!("  API token env: {} ({})", name.cyan(), status));
        }

        lines.push(format!(
            "  MCP server: {}",
            mcp_registration_status(ctx.agent_tool)
                .unwrap_or_else(|| "(unknown)".bright_black().to_string())
        ));

        Ok(StatusReport { lines })
    }
}

fn register_anytype_mcp(agent: AgentTool, env_var: &str) -> Result<()> {
    match agent {
        AgentTool::Claude => run_mcp_add("claude", &["--scope", "user"], "Claude Code", env_var),
        AgentTool::OpenCode => run_mcp_add("opencode", &[], "OpenCode", env_var),
        AgentTool::Copilot => {
            print_copilot_mcp_snippet(env_var);
            Ok(())
        }
    }
}

fn run_mcp_add(cli: &str, extra_args: &[&str], label: &str, env_var: &str) -> Result<()> {
    let env_pair = super::common::resolve_mcp_env_pair(env_var)?;
    let mut cmd = Command::new(cli);
    cmd.arg("mcp").arg("add");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(ANYTYPE_MCP_NAME)
        .arg("-e")
        .arg(&env_pair)
        .arg("--")
        .arg(ANYTYPE_MCP_COMMAND);
    for a in ANYTYPE_MCP_ARGS {
        cmd.arg(a);
    }

    let output = cmd.output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run '{} mcp add'. Is the {} CLI installed on PATH? ({})",
            cli,
            label,
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already") {
            println!(
                "{}",
                format!(
                    "ℹ️  Anytype MCP server was already registered with {}",
                    label
                )
                .bright_black()
            );
            return Ok(());
        }
        return Err(anyhow::anyhow!("{} mcp add failed: {}", cli, stderr.trim()));
    }

    println!(
        "{}",
        format!("✓ Registered Anytype MCP server with {}", label).green()
    );
    Ok(())
}

fn print_copilot_mcp_snippet(env_var: &str) {
    println!();
    println!(
        "{}",
        "GitHub Copilot: paste this into your VS Code settings.json (under \
         the \"github.copilot.mcp.servers\" key):"
            .yellow()
    );
    let args_json: Vec<String> = ANYTYPE_MCP_ARGS
        .iter()
        .map(|a| format!("\"{}\"", a))
        .collect();
    println!(
        r#"
  "anytype": {{
    "command": "{}",
    "args": [{}],
    "env": {{ "{}": "${{env:{}}}" }}
  }}
"#,
        ANYTYPE_MCP_COMMAND,
        args_json.join(", "),
        env_var,
        env_var
    );
}

/// Probe the agent's CLI for Anytype MCP registration. Returns:
/// - `Some(true)` if anytype appears in the MCP list
/// - `Some(false)` if the probe succeeded but anytype is absent
/// - `None` if we couldn't probe (Copilot; CLI missing; non-zero exit) —
///   callers treat this as "unknown".
fn probe_anytype_mcp(agent: AgentTool) -> Option<bool> {
    let cli = match agent {
        AgentTool::Claude => "claude",
        AgentTool::OpenCode => "opencode",
        AgentTool::Copilot => return None,
    };
    let output = Command::new(cli).args(["mcp", "list"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(stdout.lines().any(|l| l.contains(ANYTYPE_MCP_NAME)))
}

pub fn is_anytype_mcp_registered(agent: AgentTool) -> bool {
    probe_anytype_mcp(agent).unwrap_or(false)
}

fn mcp_registration_status(agent: Option<AgentTool>) -> Option<String> {
    let agent = agent?;
    match probe_anytype_mcp(agent)? {
        true => Some(format!("✓ registered with {agent}").green().to_string()),
        false => Some(format!("✗ not registered with {agent}").red().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendKind, BackendSettings, EffectiveConfig};
    use tempfile::TempDir;

    fn anytype_effective(settings: BackendSettings) -> EffectiveConfig {
        EffectiveConfig {
            thoughts_repo: String::new(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "alice".to_string(),
            backend: BackendKind::Anytype,
            backend_settings: settings,
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn sync_is_noop() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(BackendSettings::default());
        let ctx = BackendContext::new(tmp.path(), &eff);
        AnytypeBackend.sync(&ctx, None).unwrap();
    }

    #[test]
    fn status_reports_env_var_presence() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(BackendSettings {
            space_id: Some("s1".to_string()),
            api_token_env: Some("HYPRLAYER_TEST_ANYTYPE_TOKEN_PRESENT".to_string()),
            ..Default::default()
        });
        unsafe { std::env::remove_var("HYPRLAYER_TEST_ANYTYPE_TOKEN_PRESENT") };
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = AnytypeBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("s1"));
        assert!(joined.contains("HYPRLAYER_TEST_ANYTYPE_TOKEN_PRESENT"));
    }

    #[test]
    fn status_omits_env_row_when_unset() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(BackendSettings {
            space_id: Some("s1".to_string()),
            api_token_env: None,
            ..Default::default()
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = AnytypeBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(!joined.contains("API token env"));
    }

    #[test]
    fn status_reports_missing_type_id_as_pending() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(BackendSettings {
            space_id: Some("s1".to_string()),
            api_token_env: Some("ANYTYPE_API_KEY".to_string()),
            type_id: None,
            ..Default::default()
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = AnytypeBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("will be created on first write"));
    }

    #[test]
    fn mcp_registration_status_skips_copilot() {
        assert!(mcp_registration_status(Some(AgentTool::Copilot)).is_none());
        assert!(mcp_registration_status(None).is_none());
    }
}
