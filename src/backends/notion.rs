use anyhow::Result;
use colored::Colorize;

use super::{BackendContext, StatusReport, ThoughtsBackend, common};

pub struct NotionBackend;

impl ThoughtsBackend for NotionBackend {
    /// Initialize the Notion backend.
    ///
    /// Hyprlayer relies on the agent tool's Notion connector (Claude.ai etc.)
    /// and does not register its own MCP server. Connectors are managed by
    /// the agent tool, not `claude mcp add`; a second hyprlayer-registered
    /// server would shadow the working connector. Consequently `init()` never
    /// prompts for a token, never calls `claude mcp add`, and assumes the
    /// connector is wired up — auth errors surface at first tool call, not
    /// here.
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        let notion = ctx.effective.backend.require_notion()?;
        if notion.parent_page_id.is_empty() {
            return Err(anyhow::anyhow!(
                "Notion backend requires parentPageId in settings"
            ));
        }

        crate::hooks::setup_git_hooks(ctx.code_repo, false)?;

        common::warn_stale_thoughts_dir(ctx.code_repo, "Notion content lives in the database");

        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        let mut lines = Vec::new();
        let notion = ctx.effective.backend.require_notion()?;

        let parent = if notion.parent_page_id.is_empty() {
            "(not set)"
        } else {
            notion.parent_page_id.as_str()
        };
        lines.push(format!("  Parent page ID: {}", parent.cyan()));

        match notion.database_id.as_deref() {
            Some(id) if !id.is_empty() => lines.push(format!("  Database ID: {}", id.cyan())),
            _ => lines.push(format!(
                "  Database ID: {}",
                "(will be created on first write)".bright_black()
            )),
        }

        // `claude mcp list` doesn't see connectors, so any probe here would
        // mislabel every connector user as "not registered".
        lines.push(format!("  MCP: {}", "via agent connector".bright_black()));

        Ok(StatusReport { lines })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendConfig, EffectiveConfig, NotionConfig};
    use tempfile::TempDir;

    fn notion_effective(notion: NotionConfig) -> EffectiveConfig {
        EffectiveConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Notion(notion),
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn sync_is_noop() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(NotionConfig {
            parent_page_id: "p1".to_string(),
            database_id: None,
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        NotionBackend.sync(&ctx, None).unwrap();
    }

    #[test]
    fn status_reports_parent_and_database_without_token_or_mcp_rows() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(NotionConfig {
            parent_page_id: "p1".to_string(),
            database_id: Some("db1".to_string()),
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("p1"));
        assert!(joined.contains("db1"));
        assert!(
            !joined.contains("API token env"),
            "status must not include an API-token-env row for notion: {joined}"
        );
        assert!(
            !joined.to_lowercase().contains("registered with"),
            "status must not run a `claude mcp list` probe for notion (that misses \
             the connector and is always misleading): {joined}"
        );
    }

    #[test]
    fn status_reports_missing_database_id_as_pending() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(NotionConfig {
            parent_page_id: "p1".to_string(),
            database_id: None,
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("will be created on first write"));
    }
}
