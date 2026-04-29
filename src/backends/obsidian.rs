use anyhow::Result;
use colored::Colorize;
use std::fs;

use super::common::FilesystemDirs;
use super::{BackendContext, StatusReport, ThoughtsBackend, common};

pub struct ObsidianBackend;

impl ThoughtsBackend for ObsidianBackend {
    fn init(&self, ctx: &BackendContext) -> Result<()> {
        let obs = ctx.effective.backend.require_obsidian()?;
        let mapped =
            ctx.effective.mapped_name.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Cannot create thoughts tree: repo is not mapped")
            })?;
        let dirs = FilesystemDirs {
            repos_dir: &obs.repos_dir,
            global_dir: &obs.global_dir,
            user: &ctx.effective.user,
            mapped_name: mapped,
        };

        if obs.vault_path.is_empty() {
            return Err(anyhow::anyhow!(
                "Obsidian backend requires vaultPath in settings"
            ));
        }
        let vault_root = crate::config::expand_path(&obs.vault_path);
        if !vault_root.exists() {
            return Err(anyhow::anyhow!(
                "Obsidian vault path does not exist: {}",
                vault_root.display()
            ));
        }
        if !vault_root.join(".obsidian").exists() {
            println!(
                "{}",
                format!(
                    "{} does not contain a .obsidian/ folder — open it in Obsidian to make it a vault.",
                    vault_root.display()
                )
                .yellow()
            );
        }

        let root = obs
            .obsidian_root()
            .ok_or_else(|| anyhow::anyhow!("Obsidian backend requires vaultPath in settings"))?;

        fs::create_dir_all(&root)?;
        common::setup_directory_structure_at(&root, &dirs)?;
        common::setup_symlinks_into(&root, ctx.code_repo, &dirs)?;

        crate::hooks::setup_git_hooks(ctx.code_repo, false)?;
        Ok(())
    }

    fn sync(&self, _ctx: &BackendContext, _message: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn status(&self, ctx: &BackendContext) -> Result<StatusReport> {
        let mut lines = Vec::new();
        let obs = ctx.effective.backend.require_obsidian()?;
        let Some(root) = obs.obsidian_root() else {
            lines.push("  (No vault path configured)".bright_black().to_string());
            return Ok(StatusReport { lines });
        };

        lines.push(format!(
            "  Vault root: {}",
            root.display().to_string().cyan()
        ));

        if !root.exists() {
            lines.push(format!("  Status: {}", "Content root missing".red()));
            return Ok(StatusReport { lines });
        }

        lines.push(format!(
            "  Sync: {}",
            "not applicable (local vault)".bright_black()
        ));
        Ok(StatusReport { lines })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendConfig, EffectiveConfig, ObsidianConfig};
    use tempfile::TempDir;

    fn obsidian_effective(vault_path: String, vault_subpath: Option<String>) -> EffectiveConfig {
        EffectiveConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path,
                vault_subpath,
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn sync_is_noop() {
        let tmp = TempDir::new().unwrap();
        let eff = obsidian_effective(String::new(), None);
        let ctx = BackendContext::new(tmp.path(), &eff);
        ObsidianBackend.sync(&ctx, None).unwrap();
    }

    #[test]
    fn init_creates_tree_and_symlinks_no_git_dir() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let code = tmp.path().join("code");
        std::fs::create_dir_all(&code).unwrap();

        let eff = obsidian_effective(vault.display().to_string(), Some("hyprlayer".to_string()));
        let ctx = BackendContext::new(&code, &eff);
        ObsidianBackend.init(&ctx).unwrap();

        let content_root = vault.join("hyprlayer");
        assert!(content_root.join("repos/myproj/alice").is_dir());
        assert!(content_root.join("repos/myproj/shared").is_dir());
        assert!(content_root.join("global/alice").is_dir());
        assert!(content_root.join("global/shared").is_dir());

        let thoughts = code.join("thoughts");
        assert!(thoughts.join("alice").exists());
        assert!(thoughts.join("shared").exists());
        assert!(thoughts.join("global").exists());

        assert!(!content_root.join(".git").exists());
        assert!(!vault.join(".git").exists());
    }

    #[test]
    fn init_errors_without_vault_path() {
        let tmp = TempDir::new().unwrap();
        let code = tmp.path().join("code");
        std::fs::create_dir_all(&code).unwrap();

        let eff = obsidian_effective(String::new(), None);
        let ctx = BackendContext::new(&code, &eff);
        let err = ObsidianBackend.init(&ctx).unwrap_err();
        assert!(err.to_string().contains("vaultPath"));
    }
}
