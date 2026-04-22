use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::{AgentTool, OpenCodeProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Git,
    Obsidian,
    Notion,
    Anytype,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Git => "git",
            BackendKind::Obsidian => "obsidian",
            BackendKind::Notion => "notion",
            BackendKind::Anytype => "anytype",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackendSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_subpath: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token_env: Option<String>,
}

impl BackendSettings {
    pub fn is_empty(&self) -> bool {
        *self == BackendSettings::default()
    }

    pub fn obsidian_root(&self) -> Option<PathBuf> {
        let vault = expand_path(self.vault_path.as_deref()?);
        Some(
            match self.vault_subpath.as_deref().filter(|s| !s.is_empty()) {
                Some(sub) => vault.join(sub),
                None => vault,
            },
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default, skip_serializing_if = "BackendSettings::is_empty")]
    pub backend_settings: BackendSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RepoMapping {
    String(String),
    Object {
        repo: String,
        profile: Option<String>,
    },
}

impl RepoMapping {
    pub fn repo(&self) -> &str {
        match self {
            RepoMapping::String(s) => s,
            RepoMapping::Object { repo, .. } => repo,
        }
    }

    pub fn profile(&self) -> Option<&str> {
        match self {
            RepoMapping::String(_) => None,
            RepoMapping::Object { profile, .. } => profile.as_deref(),
        }
    }

    /// Create a new RepoMapping, using Object variant if profile is specified
    pub fn new(mapped_name: &str, profile: &Option<String>) -> Self {
        match profile {
            Some(name) => RepoMapping::Object {
                repo: mapped_name.to_string(),
                profile: Some(name.clone()),
            },
            None => RepoMapping::String(mapped_name.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
    pub user: String,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default, skip_serializing_if = "BackendSettings::is_empty")]
    pub backend_settings: BackendSettings,
    #[serde(default)]
    pub repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    #[serde(default)]
    pub agent_tool: Option<AgentTool>,
    #[serde(default)]
    pub opencode_provider: Option<OpenCodeProvider>,
    #[serde(default)]
    pub opencode_sonnet_model: Option<String>,
    #[serde(default)]
    pub opencode_opus_model: Option<String>,
}

/// Effective configuration for a specific repository
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
    pub user: String,
    pub backend: BackendKind,
    pub backend_settings: BackendSettings,
    pub profile_name: Option<String>,
    pub mapped_name: Option<String>,
}

impl ThoughtsConfig {
    /// Check whether the essential thoughts fields are populated.
    /// Returns false when only AI-related fields were configured
    /// (e.g. after `hyprlayer ai configure` but before `thoughts init`).
    pub fn is_thoughts_configured(&self) -> bool {
        !self.thoughts_repo.is_empty()
            && !self.repos_dir.is_empty()
            && !self.global_dir.is_empty()
            && !self.user.is_empty()
    }

    /// Validate that a profile exists in the config (if specified)
    pub fn validate_profile(&self, profile: &Option<String>) -> Result<()> {
        if let Some(name) = profile
            && !self.profiles.contains_key(name)
        {
            return Err(anyhow::anyhow!("Profile \"{}\" does not exist", name));
        }
        Ok(())
    }

    /// Resolve effective thoughts_repo, repos_dir, global_dir based on profile
    pub fn resolve_dirs(&self, profile: &Option<String>) -> ProfileConfig {
        profile
            .as_ref()
            .and_then(|name| self.profiles.get(name))
            .cloned()
            .unwrap_or(ProfileConfig {
                thoughts_repo: self.thoughts_repo.clone(),
                repos_dir: self.repos_dir.clone(),
                global_dir: self.global_dir.clone(),
                backend: self.backend,
                backend_settings: self.backend_settings.clone(),
            })
    }

    /// Find repo mappings whose paths no longer exist on disk.
    pub fn find_orphaned_mappings(&self) -> Vec<String> {
        self.repo_mappings
            .keys()
            .filter(|path| !Path::new(path).is_dir())
            .cloned()
            .collect()
    }

    /// Remove the given repo mappings by path.
    pub fn remove_mappings(&mut self, paths: &[String]) {
        for path in paths {
            self.repo_mappings.remove(path);
        }
    }

    /// Get the effective configuration for a repository path.
    /// Resolves profile-specific settings if the repo is mapped to a profile.
    pub fn effective_config_for(&self, repo_path: &str) -> EffectiveConfig {
        let mapping = self.repo_mappings.get(repo_path);

        let profile_name = mapping
            .and_then(|m| m.profile())
            .filter(|name| self.profiles.contains_key(*name))
            .map(|s| s.to_string());

        let dirs = self.resolve_dirs(&profile_name);

        EffectiveConfig {
            thoughts_repo: dirs.thoughts_repo,
            repos_dir: dirs.repos_dir,
            global_dir: dirs.global_dir,
            user: self.user.clone(),
            backend: dirs.backend,
            backend_settings: dirs.backend_settings,
            profile_name,
            mapped_name: mapping.map(|m| m.repo().to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HyprlayerConfig {
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub last_version_check: Option<i64>,
    #[serde(default)]
    pub disable_update_check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thoughts: Option<ThoughtsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiConfig>,
}

/// V1 config shape for migration -- the old ThoughtsConfig with all fields
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct V1ThoughtsConfig {
    #[serde(default)]
    thoughts_repo: String,
    #[serde(default)]
    repos_dir: String,
    #[serde(default)]
    global_dir: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    agent_tool: Option<AgentTool>,
    #[serde(default)]
    opencode_provider: Option<OpenCodeProvider>,
    #[serde(default)]
    opencode_sonnet_model: Option<String>,
    #[serde(default)]
    opencode_opus_model: Option<String>,
    #[serde(default)]
    repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default)]
    profiles: HashMap<String, ProfileConfig>,
    #[serde(default)]
    last_version_check: Option<i64>,
    #[serde(default)]
    disable_update_check: bool,
}

#[derive(Debug, Deserialize)]
struct V1ConfigFile {
    thoughts: Option<V1ThoughtsConfig>,
}

impl HyprlayerConfig {
    /// Load config from a file path, auto-migrating v1 configs to v2.
    pub fn load(config_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let config: HyprlayerConfig =
            serde_json::from_str(&content).with_context(|| "Failed to parse config file")?;

        // Auto-migrate v1 -> v2
        if config.version.is_none() {
            let migrated = Self::migrate_v1(&content)?;
            migrated.save(config_path)?;
            return Ok(migrated);
        }

        Ok(config)
    }

    /// Save config to a file path.
    pub fn save(&self, config_path: &Path) -> Result<()> {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }
        let json = serde_json::to_string_pretty(&self)?;
        fs::write(config_path, json)
            .with_context(|| format!("Failed to write config file: {}", config_path.display()))?;
        Ok(())
    }

    /// Get or create the thoughts section
    pub fn thoughts_mut(&mut self) -> &mut ThoughtsConfig {
        self.thoughts.get_or_insert_with(ThoughtsConfig::default)
    }

    /// Get or create the AI section
    pub fn ai_mut(&mut self) -> &mut AiConfig {
        self.ai.get_or_insert_with(AiConfig::default)
    }

    /// Migrate a v1 config (no version field) to v2 format.
    fn migrate_v1(content: &str) -> Result<Self> {
        let v1: V1ConfigFile =
            serde_json::from_str(content).with_context(|| "Failed to parse v1 config")?;

        let Some(old) = v1.thoughts else {
            return Ok(HyprlayerConfig {
                version: Some(2),
                ..Default::default()
            });
        };

        let thoughts = ThoughtsConfig {
            thoughts_repo: old.thoughts_repo,
            repos_dir: old.repos_dir,
            global_dir: old.global_dir,
            user: old.user,
            backend: BackendKind::default(),
            backend_settings: BackendSettings::default(),
            repo_mappings: old.repo_mappings,
            profiles: old.profiles,
        };

        let ai = AiConfig {
            agent_tool: old.agent_tool,
            opencode_provider: old.opencode_provider,
            opencode_sonnet_model: old.opencode_sonnet_model,
            opencode_opus_model: old.opencode_opus_model,
        };

        Ok(HyprlayerConfig {
            version: Some(2),
            last_version_check: old.last_version_check,
            disable_update_check: old.disable_update_check,
            thoughts: Some(thoughts),
            ai: Some(ai),
        })
    }
}

pub fn get_default_config_path() -> anyhow::Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(config_dir.join("hyprlayer").join("config.json"))
}

pub fn get_default_thoughts_repo() -> anyhow::Result<PathBuf> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home_dir.join("thoughts"))
}

pub fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}

pub fn get_current_repo_path() -> anyhow::Result<PathBuf> {
    std::env::current_dir().map_err(|e| anyhow::anyhow!("Could not get current directory: {}", e))
}

pub fn get_repo_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed_repo")
        .to_string()
}

pub fn sanitize_directory_name(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thoughts_config_default_values() {
        let config = ThoughtsConfig::default();
        assert_eq!(config.thoughts_repo, "");
        assert_eq!(config.repos_dir, "");
        assert_eq!(config.global_dir, "");
        assert_eq!(config.user, "");
        assert!(config.repo_mappings.is_empty());
        assert!(config.profiles.is_empty());
    }

    #[test]
    fn ai_config_default_values() {
        let config = AiConfig::default();
        assert!(config.agent_tool.is_none());
        assert!(config.opencode_provider.is_none());
        assert!(config.opencode_sonnet_model.is_none());
        assert!(config.opencode_opus_model.is_none());
    }

    #[test]
    fn hyprlayer_config_save_load_round_trip() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_config_round_trip");
        let config_path = temp_dir.join("config.json");

        let config = HyprlayerConfig {
            version: Some(2),
            last_version_check: Some(1700000000),
            disable_update_check: true,
            thoughts: Some(ThoughtsConfig {
                thoughts_repo: "~/thoughts".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
                user: "testuser".to_string(),
                ..Default::default()
            }),
            ai: Some(AiConfig {
                agent_tool: Some(AgentTool::Claude),
                ..Default::default()
            }),
        };

        config.save(&config_path).unwrap();
        let loaded = HyprlayerConfig::load(&config_path).unwrap();

        assert_eq!(loaded.version, Some(2));
        assert_eq!(loaded.last_version_check, Some(1700000000));
        assert!(loaded.disable_update_check);

        let thoughts = loaded.thoughts.unwrap();
        assert_eq!(thoughts.thoughts_repo, "~/thoughts");
        assert_eq!(thoughts.user, "testuser");
        assert!(thoughts.repo_mappings.is_empty());

        let ai = loaded.ai.unwrap();
        assert!(matches!(ai.agent_tool, Some(AgentTool::Claude)));
        assert!(ai.opencode_provider.is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn migrate_v1_full_config() {
        let json = r#"{
            "thoughts": {
                "thoughtsRepo": "~/thoughts",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "testuser",
                "agentTool": "claude",
                "opencodeProvider": null,
                "opencodeSonnetModel": null,
                "opencodeOpusModel": null,
                "repoMappings": {},
                "profiles": {},
                "lastVersionCheck": 1700000000,
                "disableUpdateCheck": false
            }
        }"#;
        let config = HyprlayerConfig::migrate_v1(json).unwrap();

        assert_eq!(config.version, Some(2));
        assert_eq!(config.last_version_check, Some(1700000000));
        assert!(!config.disable_update_check);

        let thoughts = config.thoughts.unwrap();
        assert_eq!(thoughts.thoughts_repo, "~/thoughts");
        assert_eq!(thoughts.user, "testuser");

        let ai = config.ai.unwrap();
        assert!(matches!(ai.agent_tool, Some(AgentTool::Claude)));
    }

    #[test]
    fn migrate_v1_ai_only() {
        let json = r#"{
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "",
                "globalDir": "",
                "user": "",
                "agentTool": "copilot"
            }
        }"#;
        let config = HyprlayerConfig::migrate_v1(json).unwrap();
        let ai = config.ai.unwrap();
        assert!(matches!(ai.agent_tool, Some(AgentTool::Copilot)));

        let thoughts = config.thoughts.unwrap();
        assert!(!thoughts.is_thoughts_configured());
    }

    #[test]
    fn migrate_v1_no_thoughts_key() {
        let json = r#"{}"#;
        let config = HyprlayerConfig::migrate_v1(json).unwrap();
        assert_eq!(config.version, Some(2));
        assert!(config.thoughts.is_none());
        assert!(config.ai.is_none());
    }

    #[test]
    fn migrate_v1_minimal_thoughts() {
        let json = r#"{"thoughts": {"thoughtsRepo": "~/t", "reposDir": "r", "globalDir": "g", "user": "u"}}"#;
        let config = HyprlayerConfig::migrate_v1(json).unwrap();
        assert_eq!(config.version, Some(2));
        assert!(config.last_version_check.is_none());
        assert!(!config.disable_update_check);

        let thoughts = config.thoughts.unwrap();
        assert_eq!(thoughts.thoughts_repo, "~/t");
        assert!(thoughts.is_thoughts_configured());

        let ai = config.ai.unwrap();
        assert!(ai.agent_tool.is_none());
    }

    #[test]
    fn v2_config_does_not_trigger_migration() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v2_no_migrate");
        let config_path = temp_dir.join("config.json");

        let config = HyprlayerConfig {
            version: Some(2),
            thoughts: Some(ThoughtsConfig {
                thoughts_repo: "~/thoughts".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
                user: "testuser".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        config.save(&config_path).unwrap();
        let loaded = HyprlayerConfig::load(&config_path).unwrap();

        assert_eq!(loaded.version, Some(2));
        let thoughts = loaded.thoughts.unwrap();
        assert_eq!(thoughts.thoughts_repo, "~/thoughts");

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn sanitize_directory_name_replaces_special_chars() {
        assert_eq!(sanitize_directory_name("my-project"), "my-project");
        assert_eq!(sanitize_directory_name("my_project"), "my_project");
        assert_eq!(sanitize_directory_name("my project"), "my_project");
        assert_eq!(sanitize_directory_name("my/project"), "my_project");
        assert_eq!(sanitize_directory_name("my.project.rs"), "my_project_rs");
    }

    #[test]
    fn get_repo_name_from_path_extracts_last_component() {
        assert_eq!(
            get_repo_name_from_path(Path::new("/home/user/projects/myrepo")),
            "myrepo"
        );
        assert_eq!(get_repo_name_from_path(Path::new("/")), "unnamed_repo");
    }

    #[test]
    fn repo_mapping_string_variant() {
        let mapping = RepoMapping::new("my-repo", &None);
        assert_eq!(mapping.repo(), "my-repo");
        assert!(mapping.profile().is_none());
    }

    #[test]
    fn repo_mapping_object_variant_with_profile() {
        let mapping = RepoMapping::new("my-repo", &Some("work".to_string()));
        assert_eq!(mapping.repo(), "my-repo");
        assert_eq!(mapping.profile(), Some("work"));
    }

    #[test]
    fn is_thoughts_configured_returns_false_for_default() {
        let config = ThoughtsConfig::default();
        assert!(!config.is_thoughts_configured());
    }

    #[test]
    fn is_thoughts_configured_returns_false_when_fields_partially_set() {
        let config = ThoughtsConfig {
            thoughts_repo: "~/thoughts".to_string(),
            repos_dir: "repos".to_string(),
            // global_dir and user are empty
            ..Default::default()
        };
        assert!(!config.is_thoughts_configured());
    }

    #[test]
    fn is_thoughts_configured_returns_true_when_all_fields_set() {
        let config = ThoughtsConfig {
            thoughts_repo: "~/thoughts".to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "testuser".to_string(),
            ..Default::default()
        };
        assert!(config.is_thoughts_configured());
    }

    #[test]
    fn backend_kind_default_is_git() {
        assert_eq!(BackendKind::default(), BackendKind::Git);
    }

    #[test]
    fn backend_kind_serde_round_trip() {
        for kind in [
            BackendKind::Git,
            BackendKind::Obsidian,
            BackendKind::Notion,
            BackendKind::Anytype,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: BackendKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn backend_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&BackendKind::Git).unwrap(), "\"git\"");
        assert_eq!(
            serde_json::to_string(&BackendKind::Obsidian).unwrap(),
            "\"obsidian\""
        );
        assert_eq!(
            serde_json::to_string(&BackendKind::Notion).unwrap(),
            "\"notion\""
        );
        assert_eq!(
            serde_json::to_string(&BackendKind::Anytype).unwrap(),
            "\"anytype\""
        );
    }

    #[test]
    fn config_without_backend_field_deserializes_to_git() {
        let json = r#"{
            "thoughtsRepo": "~/thoughts",
            "reposDir": "repos",
            "globalDir": "global",
            "user": "testuser"
        }"#;
        let cfg: ThoughtsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.backend, BackendKind::Git);
        assert!(cfg.backend_settings.is_empty());
    }

    #[test]
    fn profile_without_backend_field_deserializes_to_git() {
        let json = r#"{
            "thoughtsRepo": "~/work-thoughts",
            "reposDir": "repos",
            "globalDir": "global"
        }"#;
        let profile: ProfileConfig = serde_json::from_str(json).unwrap();
        assert_eq!(profile.backend, BackendKind::Git);
        assert!(profile.backend_settings.is_empty());
    }

    #[test]
    fn backend_settings_is_empty_for_default() {
        assert!(BackendSettings::default().is_empty());
    }

    #[test]
    fn backend_settings_is_not_empty_when_field_set() {
        let s = BackendSettings {
            vault_path: Some("~/vault".to_string()),
            ..Default::default()
        };
        assert!(!s.is_empty());
    }

    #[test]
    fn backend_settings_skips_empty_in_serde() {
        let cfg = ThoughtsConfig {
            thoughts_repo: "~/thoughts".to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "u".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        // Default backend (git) is still emitted because Default isn't skipped
        // for the enum, but the empty settings struct should be omitted.
        assert!(!json.contains("backendSettings"));
    }

    #[test]
    fn obsidian_root_joins_vault_and_subpath() {
        let s = BackendSettings {
            vault_path: Some("/vault".to_string()),
            vault_subpath: Some("hyprlayer".to_string()),
            ..Default::default()
        };
        assert_eq!(
            s.obsidian_root().unwrap(),
            PathBuf::from("/vault/hyprlayer")
        );
    }

    #[test]
    fn obsidian_root_handles_empty_subpath() {
        let s = BackendSettings {
            vault_path: Some("/vault".to_string()),
            vault_subpath: Some("".to_string()),
            ..Default::default()
        };
        assert_eq!(s.obsidian_root().unwrap(), PathBuf::from("/vault"));
    }

    #[test]
    fn obsidian_root_handles_missing_subpath() {
        let s = BackendSettings {
            vault_path: Some("/vault".to_string()),
            vault_subpath: None,
            ..Default::default()
        };
        assert_eq!(s.obsidian_root().unwrap(), PathBuf::from("/vault"));
    }

    #[test]
    fn obsidian_root_expands_tilde() {
        let s = BackendSettings {
            vault_path: Some("~/vault".to_string()),
            vault_subpath: Some("hyprlayer".to_string()),
            ..Default::default()
        };
        let root = s.obsidian_root().unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(root, home.join("vault").join("hyprlayer"));
    }

    #[test]
    fn obsidian_root_returns_none_without_vault_path() {
        let s = BackendSettings::default();
        assert!(s.obsidian_root().is_none());
    }

    #[test]
    fn effective_config_resolves_backend_from_profile() {
        let mut cfg = ThoughtsConfig {
            thoughts_repo: "~/t".to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "u".to_string(),
            backend: BackendKind::Git,
            ..Default::default()
        };
        cfg.profiles.insert(
            "obs".to_string(),
            ProfileConfig {
                thoughts_repo: "~/p".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
                backend: BackendKind::Obsidian,
                backend_settings: BackendSettings {
                    vault_path: Some("/vault".to_string()),
                    ..Default::default()
                },
            },
        );
        cfg.repo_mappings.insert(
            "/some/repo".to_string(),
            RepoMapping::new("myproj", &Some("obs".to_string())),
        );

        let eff = cfg.effective_config_for("/some/repo");
        assert_eq!(eff.backend, BackendKind::Obsidian);
        assert_eq!(eff.backend_settings.vault_path.as_deref(), Some("/vault"));
        assert_eq!(eff.profile_name.as_deref(), Some("obs"));
    }

    #[test]
    fn effective_config_falls_back_to_top_level_backend() {
        let cfg = ThoughtsConfig {
            thoughts_repo: "~/t".to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "u".to_string(),
            backend: BackendKind::Obsidian,
            backend_settings: BackendSettings {
                vault_path: Some("/vault".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let eff = cfg.effective_config_for("/unmapped/repo");
        assert_eq!(eff.backend, BackendKind::Obsidian);
        assert_eq!(eff.backend_settings.vault_path.as_deref(), Some("/vault"));
        assert!(eff.mapped_name.is_none());
    }
}
