use anyhow::Result;
use colored::Colorize;
use std::process::Command;

use super::{BackendContext, StatusReport, ThoughtsBackend};
use crate::agents::AgentTool;
use crate::config::{BackendKind, HyprlayerConfig, get_default_config_path};

/// The Notion MCP server command the agent tool invokes.
/// Pinned here so we don't depend on user-configurable runtime paths.
const NOTION_MCP_COMMAND: &str = "npx";
const NOTION_MCP_ARGS: &[&str] = &["-y", "@notionhq/notion-mcp-server"];
const NOTION_MCP_NAME: &str = "notion";

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
            .unwrap_or("NOTION_TOKEN");
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

        // Install the pre-commit guard and clean up any stale post-commit
        // auto-sync hook from a previous local-filesystem backend. Notion has
        // no local sync, so the post-commit hook would just fire on every
        // commit and no-op. `setup_git_hooks` silently returns an empty Vec
        // when the user isn't in a git repo — Notion users often aren't.
        let hooks_updated = crate::hooks::setup_git_hooks(ctx.code_repo, false)?;
        if !hooks_updated.is_empty() {
            println!(
                "{}",
                format!("✓ Updated git hooks: {}", hooks_updated.join(", ")).yellow()
            );
        }

        // Warn (don't delete) if stale `thoughts/` symlinks exist from a
        // previous git/obsidian init — deleting them would destroy the user's
        // path to old content; leaving them silent would be confusing.
        let stale = ctx.code_repo.join("thoughts");
        if stale.exists() {
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

        let agent = load_agent_tool()?;
        register_notion_mcp(&agent, env_var)?;

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
            mcp_registration_status().unwrap_or_else(|| "(unknown)".bright_black().to_string())
        ));

        Ok(StatusReport { lines })
    }
}

fn load_agent_tool() -> Result<AgentTool> {
    let config_path = get_default_config_path()?;
    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "No hyprlayer config found. Run 'hyprlayer ai configure' first."
        ));
    }
    let config = HyprlayerConfig::load(&config_path)?;
    config.ai.and_then(|a| a.agent_tool).ok_or_else(|| {
        anyhow::anyhow!("AI tool not configured. Run 'hyprlayer ai configure' first.")
    })
}

fn register_notion_mcp(agent: &AgentTool, env_var: &str) -> Result<()> {
    match agent {
        AgentTool::Claude => run_claude_mcp_add(env_var),
        AgentTool::OpenCode => run_opencode_mcp_add(env_var),
        AgentTool::Copilot => {
            print_copilot_mcp_snippet(env_var);
            Ok(())
        }
    }
}

fn run_claude_mcp_add(env_var: &str) -> Result<()> {
    let env_pair = format!("{}=${}", env_var, env_var);
    // Claude's variadic `-e` consumes arguments until the next flag or `--`,
    // so the server name must come BEFORE `-e` or it's mis-parsed as an env
    // value (yes, even claude's own docs example gets this wrong).
    let mut cmd = Command::new("claude");
    cmd.arg("mcp")
        .arg("add")
        .arg("--scope")
        .arg("user")
        .arg(NOTION_MCP_NAME)
        .arg("-e")
        .arg(&env_pair)
        .arg("--")
        .arg(NOTION_MCP_COMMAND);
    for a in NOTION_MCP_ARGS {
        cmd.arg(a);
    }

    let output = cmd.output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run 'claude mcp add'. Is the Claude Code CLI installed on PATH? ({})",
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Already-registered is not a hard failure — Claude's CLI returns
        // non-zero with a clear message; surface it but don't abort init.
        if stderr.contains("already") {
            println!(
                "{}",
                "ℹ️  Notion MCP server was already registered with Claude Code".bright_black()
            );
            return Ok(());
        }
        return Err(anyhow::anyhow!("claude mcp add failed: {}", stderr.trim()));
    }

    println!(
        "{}",
        "✓ Registered Notion MCP server with Claude Code".green()
    );
    Ok(())
}

fn run_opencode_mcp_add(env_var: &str) -> Result<()> {
    let env_pair = format!("{}=${}", env_var, env_var);
    let mut cmd = Command::new("opencode");
    cmd.arg("mcp")
        .arg("add")
        .arg(NOTION_MCP_NAME)
        .arg("-e")
        .arg(&env_pair)
        .arg("--")
        .arg(NOTION_MCP_COMMAND);
    for a in NOTION_MCP_ARGS {
        cmd.arg(a);
    }

    let output = cmd.output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run 'opencode mcp add'. Is the OpenCode CLI installed on PATH? ({})",
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already") {
            println!(
                "{}",
                "ℹ️  Notion MCP server was already registered with OpenCode".bright_black()
            );
            return Ok(());
        }
        return Err(anyhow::anyhow!(
            "opencode mcp add failed: {}",
            stderr.trim()
        ));
    }

    println!("{}", "✓ Registered Notion MCP server with OpenCode".green());
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
    "env": {{ "NOTION_TOKEN": "${{env:{}}}" }}
  }}
"#,
        NOTION_MCP_COMMAND,
        args_json.join(", "),
        env_var
    );
}

/// Best-effort: ask the Claude CLI whether `notion` is in its MCP server list.
/// Returns None if the CLI isn't reachable so callers can render `(unknown)`.
fn mcp_registration_status() -> Option<String> {
    let output = Command::new("claude").args(["mcp", "list"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|l| l.contains(NOTION_MCP_NAME)) {
        Some("✓ registered with Claude Code".green().to_string())
    } else {
        Some("✗ not registered with Claude Code".red().to_string())
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
