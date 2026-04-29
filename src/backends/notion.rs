use anyhow::Result;
use colored::Colorize;

use super::{BackendContext, StatusReport, ThoughtsBackend};
use crate::config::BackendKind;

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
        ctx.effective
            .backend_settings
            .validate_for(BackendKind::Notion)?;

        crate::hooks::setup_git_hooks(ctx.code_repo, false)?;

        // Use `symlink_metadata` (not `exists`) so broken symlinks — the most
        // likely "stale" shape after the user deletes the old thoughts repo —
        // still trip the warning.
        let stale = ctx.code_repo.join("thoughts");
        if stale.symlink_metadata().is_ok() {
            eprintln!(
                "{}",
                format!(
                    "Warning: stale `thoughts/` directory at {}. Notion content lives \
                     in the database. Remove with `rm -rf thoughts/` if you don't need the \
                     old links.",
                    stale.display()
                )
                .yellow()
            );
        }

        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
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

        // `claude mcp list` doesn't see connectors, so any probe here would
        // mislabel every connector user as "not registered".
        lines.push(format!(
            "  MCP: {}",
            "via agent connector".bright_black()
        ));

        Ok(StatusReport { lines })
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
    fn status_reports_parent_and_database_without_token_or_mcp_rows() {
        let tmp = TempDir::new().unwrap();
        let eff = notion_effective(BackendSettings {
            parent_page_id: Some("p1".to_string()),
            database_id: Some("db1".to_string()),
            // Populate api_token_env to prove we do NOT surface it — hyprlayer
            // explicitly does not manage a notion token anymore.
            api_token_env: Some("HYPRLAYER_SHOULD_NOT_SURFACE".to_string()),
            ..Default::default()
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("p1"));
        assert!(joined.contains("db1"));
        assert!(
            !joined.contains("HYPRLAYER_SHOULD_NOT_SURFACE"),
            "status must not surface api_token_env for notion: {joined}"
        );
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
        let eff = notion_effective(BackendSettings {
            parent_page_id: Some("p1".to_string()),
            database_id: None,
            ..Default::default()
        });
        let ctx = BackendContext::new(tmp.path(), &eff);
        let report = NotionBackend.status(&ctx).unwrap();
        let joined = report.lines.join("\n");
        assert!(joined.contains("will be created on first write"));
    }
}
