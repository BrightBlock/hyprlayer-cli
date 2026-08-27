use anyhow::Result;
use colored::Colorize;
use std::process::Command;

use super::{BackendContext, StatusReport, ThoughtsBackend, common};

/// The Anytype MCP server command the agent tool invokes.
const ANYTYPE_MCP_COMMAND: &str = "npx";
const ANYTYPE_MCP_ARGS: &[&str] = &["-y", "@any-org/anytype-mcp"];
const ANYTYPE_MCP_NAME: &str = "anytype";

/// Default name of the env var holding the Anytype API key when the user
/// doesn't specify one. Referenced from config defaults and init prompts.
pub const DEFAULT_ANYTYPE_TOKEN_ENV: &str = "ANYTYPE_API_KEY";

#[derive(Debug, Clone, Copy)]
enum BaseCli {
    Claude,
    Codex,
}

impl BaseCli {
    const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

pub struct AnytypeBackend;

impl ThoughtsBackend for AnytypeBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        let any = ctx.effective.backend.require_anytype()?;
        if any.space_id.is_empty() {
            return Err(anyhow::anyhow!(
                "Anytype backend requires spaceId in settings"
            ));
        }

        crate::hooks::setup_git_hooks(ctx.code_repo, false)?;

        common::warn_stale_thoughts_dir(ctx.code_repo, "Anytype content lives in the app");

        let env_var = any
            .api_token_env
            .as_deref()
            .unwrap_or(DEFAULT_ANYTYPE_TOKEN_ENV);
        if std::env::var(env_var).is_err() {
            eprintln!(
                "{}",
                format!(
                    "Warning: env var {} is not set. Set it before starting Claude or Codex. \
                     Issue an API key in Anytype under Settings → API Keys.",
                    env_var
                )
                .yellow()
            );
        }
        reconcile_anytype_mcp(env_var);

        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        let mut lines = Vec::new();
        let any = ctx.effective.backend.require_anytype()?;

        let space = if any.space_id.is_empty() {
            "(not set)"
        } else {
            any.space_id.as_str()
        };
        lines.push(format!("  Space ID: {}", space.cyan()));

        match any.type_id.as_deref() {
            Some(id) if !id.is_empty() => lines.push(format!("  Type ID: {}", id.cyan())),
            _ => lines.push(format!(
                "  Type ID: {}",
                "(will be created on first write)".bright_black()
            )),
        }

        if let Some(name) = any.api_token_env.as_deref() {
            let set = std::env::var(name).is_ok();
            let status = if set {
                "set".green().to_string()
            } else {
                "not set".red().to_string()
            };
            lines.push(format!("  API token env: {} ({})", name.cyan(), status));
        }

        lines.push(format!("  MCP server: {}", mcp_registration_status()));

        Ok(StatusReport { lines })
    }
}

fn reconcile_anytype_mcp(env_var: &str) {
    let mut found_cli = false;
    for cli in BaseCli::ALL {
        if !cli_is_available(cli) {
            continue;
        }
        found_cli = true;
        if probe_anytype_mcp(cli) == Some(true) {
            continue;
        }
        if let Err(error) = run_mcp_add(cli, env_var) {
            eprintln!(
                "warning: could not register Anytype MCP with {}: {error}",
                cli.label()
            );
        }
    }
    if !found_cli {
        eprintln!(
            "warning: neither Claude Code nor Codex is available on PATH; \
             install either CLI, then rerun `hyprlayer thoughts init --force` to register Anytype MCP"
        );
    }
}

fn mcp_add_args(cli: BaseCli, env_pair: &str) -> Vec<String> {
    let mut args = vec!["mcp".into(), "add".into()];
    match cli {
        BaseCli::Claude => {
            args.extend(["--scope".into(), "user".into(), ANYTYPE_MCP_NAME.into()]);
            args.extend(["-e".into(), env_pair.into()]);
        }
        BaseCli::Codex => {
            args.extend(["--env".into(), env_pair.into(), ANYTYPE_MCP_NAME.into()]);
        }
    }
    args.push("--".into());
    args.push(ANYTYPE_MCP_COMMAND.into());
    args.extend(ANYTYPE_MCP_ARGS.iter().map(|arg| (*arg).to_string()));
    args
}

fn run_mcp_add(cli: BaseCli, env_var: &str) -> Result<()> {
    let env_pair = super::common::resolve_mcp_env_pair(env_var)?;
    let mut cmd = Command::new(cli.command());
    cmd.args(mcp_add_args(cli, &env_pair));

    let output = cmd.output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run '{} mcp add'. Is the {} CLI installed on PATH? ({})",
            cli.command(),
            cli.label(),
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already") {
            return Ok(());
        }
        return Err(anyhow::anyhow!(
            "{} mcp add failed: {}",
            cli.command(),
            stderr.trim()
        ));
    }
    Ok(())
}

/// Probe one base CLI for Anytype MCP registration. Returns:
/// - `Some(true)` if anytype appears in the MCP list
/// - `Some(false)` if the probe succeeded but anytype is absent
/// - `None` if we couldn't probe (CLI missing or non-zero exit) —
///   callers treat this as "unknown".
fn probe_anytype_mcp(cli: BaseCli) -> Option<bool> {
    let output = Command::new(cli.command())
        .args(["mcp", "list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(stdout.lines().any(|l| l.contains(ANYTYPE_MCP_NAME)))
}

fn cli_is_available(cli: BaseCli) -> bool {
    Command::new(cli.command())
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn is_anytype_mcp_registered() -> bool {
    BaseCli::ALL
        .into_iter()
        .any(|cli| probe_anytype_mcp(cli) == Some(true))
}

fn mcp_registration_status() -> String {
    BaseCli::ALL
        .into_iter()
        .map(|cli| {
            let state = match probe_anytype_mcp(cli) {
                Some(true) => "registered".green().to_string(),
                Some(false) => "not registered".red().to_string(),
                None => "unavailable".bright_black().to_string(),
            };
            format!("{}: {state}", cli.label())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AnytypeConfig, BackendConfig, EffectiveConfig};
    use tempfile::TempDir;

    fn anytype_effective(any: AnytypeConfig) -> EffectiveConfig {
        EffectiveConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Anytype(any),
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn sync_is_noop() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(AnytypeConfig {
            space_id: "s1".to_string(),
            type_id: None,
            api_token_env: None,
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        AnytypeBackend.sync(&ctx, None).unwrap();
    }

    #[test]
    fn status_reports_env_var_presence() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(AnytypeConfig {
            space_id: "s1".to_string(),
            type_id: None,
            api_token_env: Some("HYPRLAYER_TEST_ANYTYPE_TOKEN_PRESENT".to_string()),
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
        let eff = anytype_effective(AnytypeConfig {
            space_id: "s1".to_string(),
            type_id: None,
            api_token_env: None,
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = AnytypeBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(!joined.contains("API token env"));
    }

    #[test]
    fn status_reports_missing_type_id_as_pending() {
        let tmp = TempDir::new().unwrap();
        let eff = anytype_effective(AnytypeConfig {
            space_id: "s1".to_string(),
            type_id: None,
            api_token_env: Some("ANYTYPE_API_KEY".to_string()),
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = AnytypeBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("will be created on first write"));
    }

    #[test]
    fn mcp_add_syntax_matches_each_base_cli() {
        assert_eq!(
            mcp_add_args(BaseCli::Claude, "ANYTYPE_API_KEY=secret"),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "anytype",
                "-e",
                "ANYTYPE_API_KEY=secret",
                "--",
                "npx",
                "-y",
                "@any-org/anytype-mcp"
            ]
        );
        assert_eq!(
            mcp_add_args(BaseCli::Codex, "ANYTYPE_API_KEY=secret"),
            [
                "mcp",
                "add",
                "--env",
                "ANYTYPE_API_KEY=secret",
                "anytype",
                "--",
                "npx",
                "-y",
                "@any-org/anytype-mcp"
            ]
        );
    }
}
