use anyhow::Result;
use std::path::Path;

use crate::agents::AgentTool;
use crate::config::{BackendKind, EffectiveConfig};

pub mod anytype;
pub mod common;
pub mod git;
pub mod notion;
pub mod obsidian;
pub mod schema;

pub struct BackendContext<'a> {
    pub code_repo: &'a Path,
    pub effective: &'a EffectiveConfig,
    /// The active AI tool, when configured. Only backends that register MCP
    /// servers (notion, anytype) need this; others ignore it.
    pub agent_tool: Option<AgentTool>,
}

impl<'a> BackendContext<'a> {
    pub fn new(code_repo: &'a Path, effective: &'a EffectiveConfig) -> Self {
        Self {
            code_repo,
            effective,
            agent_tool: None,
        }
    }

    pub fn with_agent_tool(mut self, agent_tool: Option<AgentTool>) -> Self {
        self.agent_tool = agent_tool;
        self
    }
}

pub struct StatusReport {
    pub lines: Vec<String>,
}

pub trait ThoughtsBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()>;
    fn sync(&self, ctx: &BackendContext, message: Option<&str>) -> Result<()>;
    fn status(&self, ctx: &BackendContext) -> Result<StatusReport>;
}

pub fn for_kind(kind: BackendKind) -> Box<dyn ThoughtsBackend> {
    match kind {
        BackendKind::Git => Box::new(git::GitBackend),
        BackendKind::Obsidian => Box::new(obsidian::ObsidianBackend),
        BackendKind::Notion => Box::new(notion::NotionBackend),
        BackendKind::Anytype => Box::new(anytype::AnytypeBackend),
    }
}
