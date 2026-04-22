use anyhow::Result;
use std::path::Path;

use crate::config::{BackendKind, EffectiveConfig};

pub mod common;
pub mod git;
pub mod obsidian;
pub mod schema;

pub struct BackendContext<'a> {
    pub code_repo: &'a Path,
    pub effective: &'a EffectiveConfig,
}

impl<'a> BackendContext<'a> {
    pub fn new(code_repo: &'a Path, effective: &'a EffectiveConfig) -> Self {
        Self {
            code_repo,
            effective,
        }
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
        BackendKind::Notion => panic!("Notion backend is not yet implemented (phase 3)"),
        BackendKind::Anytype => panic!("Anytype backend is not yet implemented (phase 4)"),
    }
}
