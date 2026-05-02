use anyhow::Result;

use crate::cli::AiReinstallArgs;
use crate::commands::ai::record_install;

pub fn reinstall(args: AiReinstallArgs) -> Result<()> {
    let AiReinstallArgs { config } = args;
    let config_path = config.path()?;

    let mut hyprlayer_config = config.load().map_err(|_| {
        anyhow::anyhow!("No configuration found. Run 'hyprlayer ai configure' first.")
    })?;

    let (agent_tool, opencode_provider) = {
        let ai_config = hyprlayer_config.ai.as_ref().ok_or_else(|| {
            anyhow::anyhow!("No AI tool configured. Run 'hyprlayer ai configure' first.")
        })?;
        let agent_tool = ai_config.agent_tool.ok_or_else(|| {
            anyhow::anyhow!("No AI tool configured. Run 'hyprlayer ai configure' first.")
        })?;
        (agent_tool, ai_config.opencode_provider.clone())
    };

    let sha = agent_tool.install(opencode_provider.as_ref(), false)?;
    record_install(&mut hyprlayer_config, &config_path, sha)?;

    Ok(())
}
