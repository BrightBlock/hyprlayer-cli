use anyhow::Result;

use crate::cli::StorageSetDatabaseIdArgs;
use crate::config::{HyprlayerConfig, get_current_repo_path};

pub fn set_database_id(args: StorageSetDatabaseIdArgs) -> Result<()> {
    let StorageSetDatabaseIdArgs { id, config } = args;

    if id.trim().is_empty() {
        return Err(anyhow::anyhow!("Database ID cannot be empty"));
    }

    let config_path = config.path()?;
    let mut hyprlayer_config = HyprlayerConfig::load(&config_path)?;

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();

    let thoughts = hyprlayer_config.thoughts.as_mut().ok_or_else(|| {
        anyhow::anyhow!("No thoughts configuration found. Run 'hyprlayer thoughts init' first.")
    })?;

    let backend = thoughts.active_backend_mut(&current_repo_str)?;
    backend.require_notion_mut("set-database-id")?.database_id = Some(id);
    hyprlayer_config.save(&config_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ConfigArgs;
    use crate::commands::storage::test_util::with_cwd;
    use crate::config::{BackendConfig, GitConfig, NotionConfig, RepoMapping, ThoughtsConfig};
    use tempfile::TempDir;

    fn seed_notion_config(path: &std::path::Path, current_repo_str: &str) -> anyhow::Result<()> {
        let config = HyprlayerConfig {
            version: Some(3),
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: BackendConfig::Notion(NotionConfig {
                    parent_page_id: "p1".to_string(),
                    database_id: None,
                }),
                repo_mappings: [(
                    current_repo_str.to_string(),
                    RepoMapping::new("myproj", &None),
                )]
                .into_iter()
                .collect(),
                profiles: Default::default(),
            }),
            ..Default::default()
        };
        config.save(path)?;
        Ok(())
    }

    fn seed_git_config(path: &std::path::Path, current_repo_str: &str) -> anyhow::Result<()> {
        let config = HyprlayerConfig {
            version: Some(3),
            thoughts: Some(ThoughtsConfig {
                user: "alice".to_string(),
                backend: BackendConfig::Git(GitConfig {
                    thoughts_repo: "~/t".to_string(),
                    repos_dir: "repos".to_string(),
                    global_dir: "global".to_string(),
                }),
                repo_mappings: [(
                    current_repo_str.to_string(),
                    RepoMapping::new("myproj", &None),
                )]
                .into_iter()
                .collect(),
                profiles: Default::default(),
            }),
            ..Default::default()
        };
        config.save(path)?;
        Ok(())
    }

    #[test]
    fn errors_when_active_backend_is_not_notion() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_git_config(&cfg_path, &repo_dir.display().to_string()).unwrap();

        with_cwd(&repo_dir, || {
            let err = set_database_id(StorageSetDatabaseIdArgs {
                id: "db-123".to_string(),
                config: ConfigArgs {
                    config_file: Some(cfg_path.display().to_string()),
                },
            })
            .unwrap_err();
            assert!(err.to_string().contains("only valid for notion"));
        });
    }

    #[test]
    fn updates_notion_database_id_on_success() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_notion_config(&cfg_path, &repo_dir.display().to_string()).unwrap();

        with_cwd(&repo_dir, || {
            set_database_id(StorageSetDatabaseIdArgs {
                id: "db-123".to_string(),
                config: ConfigArgs {
                    config_file: Some(cfg_path.display().to_string()),
                },
            })
            .unwrap();
        });

        let loaded = HyprlayerConfig::load(&cfg_path).unwrap();
        let t = loaded.thoughts.unwrap();
        let n = t.backend.as_notion().unwrap();
        assert_eq!(n.database_id.as_deref(), Some("db-123"));
    }

    #[test]
    fn rejects_empty_id() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_notion_config(&cfg_path, &repo_dir.display().to_string()).unwrap();

        with_cwd(&repo_dir, || {
            let err = set_database_id(StorageSetDatabaseIdArgs {
                id: "   ".to_string(),
                config: ConfigArgs {
                    config_file: Some(cfg_path.display().to_string()),
                },
            })
            .unwrap_err();
            assert!(err.to_string().contains("cannot be empty"));
        });
    }
}
