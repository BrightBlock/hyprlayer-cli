use anyhow::Result;

use crate::cli::AiReinstallArgs;

pub fn reinstall(args: AiReinstallArgs) -> Result<()> {
    let AiReinstallArgs { config } = args;

    // Load config with descriptive error using combinator
    let hyprlayer_config = config.load().map_err(|_| {
        anyhow::anyhow!("No configuration found. Run 'hyprlayer ai configure' first.")
    })?;

    let ai_config = hyprlayer_config.ai.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No AI tool configured. Run 'hyprlayer ai configure' first.")
    })?;

    // Extract agent tool with ok_or_else for Option -> Result conversion
    let agent_tool = ai_config.agent_tool.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No AI tool configured. Run 'hyprlayer ai configure' first.")
    })?;

    agent_tool.install(ai_config.opencode_provider.as_ref())?;

    Ok(())
}
