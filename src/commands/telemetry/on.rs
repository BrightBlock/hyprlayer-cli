use anyhow::Result;

use crate::cli::TelemetryOnArgs;
use crate::commands::ai::{
    install_claude_hook_if_applicable, install_opencode_plugin_if_applicable,
};
use crate::telemetry::lifecycle;

pub fn on(args: TelemetryOnArgs) -> Result<()> {
    let config_path = args.config.path()?;
    let mut cfg = args.config.load_or_default()?;

    lifecycle::enroll(&mut cfg, &config_path)?;

    // Re-install whichever harness-specific lifecycle artifact
    // matches the user's configured agent (covers the off → on
    // round trip where `off` uninstalled them). Both orchestrators
    // are no-ops when the agent doesn't match, so calling both
    // unconditionally is safe and lets each owner decide.
    if let Some(agent) = cfg.ai.as_ref().and_then(|a| a.agent_tool.as_ref()).copied() {
        install_claude_hook_if_applicable(agent, &cfg);
        install_opencode_plugin_if_applicable(agent, &cfg);
    }
    Ok(())
}
