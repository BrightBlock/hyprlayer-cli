use anyhow::Result;

use crate::cli::StorageSetTypeIdArgs;
use crate::config::{BackendKind, HyprlayerConfig, RepoMapping, get_current_repo_path};

pub fn set_type_id(args: StorageSetTypeIdArgs) -> Result<()> {
    let StorageSetTypeIdArgs { id, config } = args;

    if id.trim().is_empty() {
        return Err(anyhow::anyhow!("Type ID cannot be empty"));
    }

    let config_path = config.path()?;
    let mut hyprlayer_config = HyprlayerConfig::load(&config_path)?;

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();

    let thoughts = hyprlayer_config.thoughts.as_mut().ok_or_else(|| {
        anyhow::anyhow!("No thoughts configuration found. Run 'hyprlayer thoughts init' first.")
    })?;

    let profile_name = thoughts
        .repo_mappings
        .get(&current_repo_str)
        .and_then(|m: &RepoMapping| m.profile())
        .map(|s| s.to_string());

    let (backend, settings) = match profile_name.as_deref() {
        Some(name) => {
            let profile = thoughts.profiles.get_mut(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Profile \"{}\" referenced by repo mapping does not exist",
                    name
                )
            })?;
            (profile.backend, &mut profile.backend_settings)
        }
        None => (thoughts.backend, &mut thoughts.backend_settings),
    };
    if backend != BackendKind::Anytype {
        return Err(anyhow::anyhow!(
            "Active backend is '{}', but set-type-id is only valid for anytype",
            backend.as_str()
        ));
    }
    settings.type_id = Some(id);
    hyprlayer_config.save(&config_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ConfigArgs;
    use crate::commands::storage::test_util::with_cwd;
    use crate::config::{BackendSettings, ThoughtsConfig};
    use tempfile::TempDir;

    fn seed_config(
        path: &std::path::Path,
        backend: BackendKind,
        current_repo_str: &str,
    ) -> anyhow::Result<()> {
        let config = HyprlayerConfig {
            version: Some(2),
            thoughts: Some(ThoughtsConfig {
                thoughts_repo: "~/t".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
                user: "alice".to_string(),
                backend,
                backend_settings: BackendSettings {
                    space_id: Some("s1".to_string()),
                    api_token_env: Some("ANYTYPE_API_KEY".to_string()),
                    ..Default::default()
                },
                repo_mappings: [(
                    current_repo_str.to_string(),
                    RepoMapping::new("myproj", &None),
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            }),
            ..Default::default()
        };
        config.save(path)?;
        Ok(())
    }

    #[test]
    fn errors_when_active_backend_is_not_anytype() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_config(&cfg_path, BackendKind::Git, &repo_dir.display().to_string()).unwrap();

        with_cwd(&repo_dir, || {
            let err = set_type_id(StorageSetTypeIdArgs {
                id: "type-123".to_string(),
                config: ConfigArgs {
                    config_file: Some(cfg_path.display().to_string()),
                },
            })
            .unwrap_err();
            assert!(err.to_string().contains("only valid for anytype"));
        });
    }

    #[test]
    fn updates_anytype_type_id_on_success() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_config(
            &cfg_path,
            BackendKind::Anytype,
            &repo_dir.display().to_string(),
        )
        .unwrap();

        with_cwd(&repo_dir, || {
            set_type_id(StorageSetTypeIdArgs {
                id: "type-123".to_string(),
                config: ConfigArgs {
                    config_file: Some(cfg_path.display().to_string()),
                },
            })
            .unwrap();
        });

        let loaded = HyprlayerConfig::load(&cfg_path).unwrap();
        let t = loaded.thoughts.unwrap();
        assert_eq!(t.backend_settings.type_id.as_deref(), Some("type-123"));
    }

    #[test]
    fn rejects_empty_id() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        seed_config(
            &cfg_path,
            BackendKind::Anytype,
            &repo_dir.display().to_string(),
        )
        .unwrap();

        with_cwd(&repo_dir, || {
            let err = set_type_id(StorageSetTypeIdArgs {
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
