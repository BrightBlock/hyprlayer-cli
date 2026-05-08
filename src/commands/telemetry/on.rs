use anyhow::Result;

use crate::agents::AgentTool;
use crate::cli::TelemetryOnArgs;
use crate::commands::ai::install_claude_hook_if_applicable;
use crate::telemetry::lifecycle;

pub fn on(args: TelemetryOnArgs) -> Result<()> {
    let config_path = args.config.path()?;
    let mut cfg = args.config.load_or_default()?;

    lifecycle::enroll(&mut cfg);
    cfg.save(&config_path)?;

    // Re-install the Stop hook for Claude users (covers the
    // off → on round trip where `off` uninstalled it).
    if cfg.ai.as_ref().and_then(|a| a.agent_tool.as_ref()).copied() == Some(AgentTool::Claude) {
        install_claude_hook_if_applicable(AgentTool::Claude);
    }
    Ok(())
}
