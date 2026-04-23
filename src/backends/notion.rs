use anyhow::Result;
use colored::Colorize;
use std::process::Command;

use super::{BackendContext, StatusReport, ThoughtsBackend};
use crate::agents::AgentTool;
use crate::config::BackendKind;

/// The Notion MCP server command the agent tool invokes.
/// Pinned here so we don't depend on user-configurable runtime paths.
const NOTION_MCP_COMMAND: &str = "npx";
const NOTION_MCP_ARGS: &[&str] = &["-y", "@notionhq/notion-mcp-server"];
const NOTION_MCP_NAME: &str = "notion";

/// Default name of the env var holding the Notion integration token when the
/// user doesn't specify one. Referenced from config defaults, init prompts,
/// and the Copilot settings snippet to avoid drift.
pub const DEFAULT_NOTION_TOKEN_ENV: &str = "NOTION_TOKEN";

pub struct NotionBackend;

impl ThoughtsBackend for NotionBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        ctx.effective
            .backend_settings
            .validate_for(BackendKind::Notion)?;

        let env_var = ctx
            .effective
            .backend_settings
            .api_token_env
            .as_deref()
            .unwrap_or(DEFAULT_NOTION_TOKEN_ENV);
        if std::env::var(env_var).is_err() {
            println!(
                "{}",
                format!(
                    "⚠️  Env var {} is not set in the current shell — set it before starting your AI tool.",
                    env_var
                )
                .yellow()
            );
        }

        let hooks_updated = crate::hooks::setup_git_hooks(ctx.code_repo, false)?;
        if !hooks_updated.is_empty() {
            println!(
                "{}",
                format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow()
            );
        }

        // Use `symlink_metadata` (not `exists`) so broken symlinks — the most
        // likely "stale" shape after the user deletes the old thoughts repo —
        // still trip the warning.
        let stale = ctx.code_repo.join("thoughts");
        if stale.symlink_metadata().is_ok() {
            println!(
                "{}",
                format!(
                    "ℹ️  A `thoughts/` directory exists at {} — likely stale symlinks from a \
                     previous backend. Notion content lives in the database, not on disk. \
                     You can `rm -rf thoughts/` once you're sure you don't need the old links.",
                    stale.display()
                )
                .bright_black()
            );
        }

        let agent = ctx.agent_tool.ok_or_else(|| {
            anyhow::anyhow!("AI tool not configured. Run 'hyprlayer ai configure' first.")
        })?;
        register_notion_mcp(agent, env_var)?;

        if ctx
            .effective
            .backend_settings
            .database_id
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            println!(
                "{}",
                "No Notion database configured yet. Your first /create_plan (or similar) in \
                 this repo will create one under the configured parent page and persist the ID."
                    .bright_black()
            );
        }

        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
        println!(
            "{}",
            "Notion backend — the AI agent reads and writes directly via MCP".bright_black()
        );
        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        let mut lines = Vec::new();
        let settings = &ctx.effective.backend_settings;

        let parent = settings.parent_page_id.as_deref().unwrap_or("(not set)");
        lines.push(format!("  Parent page ID: {}", parent.cyan()));

        match settings.database_id.as_deref() {
            Some(id) if !id.is_empty() => lines.push(format!("  Database ID: {}", id.cyan())),
            _ => lines.push(format!(
                "  Database ID: {}",
                "(will be created on first write)".bright_black()
            )),
        }

        match settings.api_token_env.as_deref() {
            Some(name) => {
                let set = std::env::var(name).is_ok();
                let status = if set {
                    "✓ set".green().to_string()
                } else {
                    "✗ not set".red().to_string()
                };
                lines.push(format!("  API token env: {} ({})", name.cyan(), status));
            }
            None => lines.push(format!("  API token env: {}", "(not set)".bright_black())),
        }

        lines.push(format!(
            "  MCP server: {}",
            mcp_registration_status(ctx.agent_tool)
                .unwrap_or_else(|| "(unknown)".bright_black().to_string())
        ));

        Ok(StatusReport { lines })
    }
}

fn register_notion_mcp(agent: AgentTool, env_var: &str) -> Result<()> {
    match agent {
        AgentTool::Claude => run_mcp_add("claude", &["--scope", "user"], "Claude Code", env_var),
        AgentTool::OpenCode => run_mcp_add("opencode", &[], "OpenCode", env_var),
        AgentTool::Copilot => {
            print_copilot_mcp_snippet(env_var);
            Ok(())
        }
    }
}

/// Register the Notion MCP server with a Claude-style CLI (`<cli> mcp add …`).
/// Shared by Claude Code and OpenCode, which differ only in their extra flags
/// (Claude needs `--scope user`; OpenCode has no equivalent) and the friendly
/// label printed to the user.
fn run_mcp_add(cli: &str, extra_args: &[&str], label: &str, env_var: &str) -> Result<()> {
    let env_pair = format!("{}=${}", env_var, env_var);
    let mut cmd = Command::new(cli);
    cmd.arg("mcp").arg("add");
    for a in extra_args {
        cmd.arg(a);
    }
    // Variadic `-e` (on Claude) consumes arguments until the next flag or
    // `--`, so the server name must come BEFORE `-e` — even Claude's own
    // docs example gets this wrong.
    cmd.arg(NOTION_MCP_NAME)
        .arg("-e")
        .arg(&env_pair)
        .arg("--")
        .arg(NOTION_MCP_COMMAND);
    for a in NOTION_MCP_ARGS {
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
                    "ℹ️  Notion MCP server was already registered with {}",
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
        format!("✓ Registered Notion MCP server with {}", label).green()
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
    let args_json: Vec<String> = NOTION_MCP_ARGS
        .iter()
        .map(|a| format!("\"{}\"", a))
        .collect();
    println!(
        r#"
  "notion": {{
    "command": "{}",
    "args": [{}],
    "env": {{ "{}": "${{env:{}}}" }}
  }}
"#,
        NOTION_MCP_COMMAND,
        args_json.join(", "),
        env_var,
        env_var
    );
}

/// Best-effort: ask the agent's CLI whether `notion` is in its MCP server list.
/// Returns `None` if the agent isn't Claude/OpenCode (Copilot has no CLI to
/// probe) or the CLI isn't reachable, so callers can render `(unknown)` and
/// skip a misleading "✗ not registered" row.
fn mcp_registration_status(agent: Option<AgentTool>) -> Option<String> {
    let (cli, label) = match agent? {
        AgentTool::Claude => ("claude", "Claude Code"),
        AgentTool::OpenCode => ("opencode", "OpenCode"),
        AgentTool::Copilot => return None,
    };
    let output = Command::new(cli).args(["mcp", "list"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|l| l.contains(NOTION_MCP_NAME)) {
        Some(format!("✓ registered with {}", label).green().to_string())
    } else {
        Some(format!("✗ not registered with {}", label).red().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendKind, BackendSettings, EffectiveConfig};
    use tempfile::TempDir;

    fn notion_effective(settings: BackendSettings) -> EffectiveConfig {
        EffectiveConfig {
            thoughts_repo: String::new(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "alice".to_string(),
            backend: BackendKind::Notion,
            backend_settings: settings,
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn sync_is_noop() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(BackendSettings::default());
        let ctx = BackendContext::new(tmp.path(), &eff);
        NotionBackend.sync(&ctx, None).unwrap();
    }

    #[test]
    fn status_reports_env_var_presence() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(BackendSettings {
            parent_page_id: Some("p1".to_string()),
            api_token_env: Some("HYPRLAYER_TEST_NOTION_TOKEN_PRESENT".to_string()),
            ..Default::default()
        });
        // Intentionally don't set the env var — status should surface ✗.
        unsafe { std::env::remove_var("HYPRLAYER_TEST_NOTION_TOKEN_PRESENT") };
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("p1"));
        assert!(joined.contains("HYPRLAYER_TEST_NOTION_TOKEN_PRESENT"));
    }

    #[test]
    fn mcp_registration_status_skips_copilot() {
        // Copilot has no CLI to probe — returning None lets the status view
        // render `(unknown)` instead of a misleading `✗ not registered`.
        assert!(mcp_registration_status(Some(AgentTool::Copilot)).is_none());
        assert!(mcp_registration_status(None).is_none());
    }

    #[test]
    fn status_reports_missing_database_id_as_pending() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(BackendSettings {
            parent_page_id: Some("p1".to_string()),
            api_token_env: Some("NOTION_TOKEN".to_string()),
            database_id: None,
            ..Default::default()
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("will be created on first write"));
    }
}
