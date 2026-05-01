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

    /// Local-filesystem backends create `<code_repo>/thoughts/` symlinks and
    /// write markdown to disk. Flat-list backends (notion, anytype) delegate
    /// all I/O to MCP tools the agent invokes.
    pub fn uses_filesystem(self) -> bool {
        matches!(self, BackendKind::Git | BackendKind::Obsidian)
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianConfig {
    pub vault_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_subpath: Option<String>,
    pub repos_dir: String,
    pub global_dir: String,
}

impl ObsidianConfig {
    pub fn obsidian_root(&self) -> Option<PathBuf> {
        if self.vault_path.is_empty() {
            return None;
        }
        let vault = expand_path(&self.vault_path);
        Some(
            match self.vault_subpath.as_deref().filter(|s| !s.is_empty()) {
                Some(sub) => vault.join(sub),
                None => vault,
            },
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotionConfig {
    pub parent_page_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnytypeConfig {
    pub space_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BackendConfig {
    Git(GitConfig),
    Obsidian(ObsidianConfig),
    Notion(NotionConfig),
    Anytype(AnytypeConfig),
}

impl Default for BackendConfig {
    fn default() -> Self {
        BackendConfig::Git(GitConfig::default())
    }
}

impl BackendConfig {
    pub fn kind(&self) -> BackendKind {
        match self {
            BackendConfig::Git(_) => BackendKind::Git,
            BackendConfig::Obsidian(_) => BackendKind::Obsidian,
            BackendConfig::Notion(_) => BackendKind::Notion,
            BackendConfig::Anytype(_) => BackendKind::Anytype,
        }
    }

    pub fn as_git(&self) -> Option<&GitConfig> {
        if let Self::Git(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_obsidian(&self) -> Option<&ObsidianConfig> {
        if let Self::Obsidian(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_notion(&self) -> Option<&NotionConfig> {
        if let Self::Notion(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_anytype(&self) -> Option<&AnytypeConfig> {
        if let Self::Anytype(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_notion_mut(&mut self) -> Option<&mut NotionConfig> {
        if let Self::Notion(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_anytype_mut(&mut self) -> Option<&mut AnytypeConfig> {
        if let Self::Anytype(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn require_git(&self) -> Result<&GitConfig> {
        self.as_git()
            .ok_or_else(|| dispatch_mismatch(BackendKind::Git, self.kind()))
    }

    pub fn require_obsidian(&self) -> Result<&ObsidianConfig> {
        self.as_obsidian()
            .ok_or_else(|| dispatch_mismatch(BackendKind::Obsidian, self.kind()))
    }

    pub fn require_notion(&self) -> Result<&NotionConfig> {
        self.as_notion()
            .ok_or_else(|| dispatch_mismatch(BackendKind::Notion, self.kind()))
    }

    pub fn require_anytype(&self) -> Result<&AnytypeConfig> {
        self.as_anytype()
            .ok_or_else(|| dispatch_mismatch(BackendKind::Anytype, self.kind()))
    }

    pub fn require_notion_mut(&mut self, action: &str) -> Result<&mut NotionConfig> {
        let actual = self.kind();
        self.as_notion_mut().ok_or_else(|| {
            anyhow::anyhow!("Active backend is '{actual}', but {action} is only valid for notion")
        })
    }

    pub fn require_anytype_mut(&mut self, action: &str) -> Result<&mut AnytypeConfig> {
        let actual = self.kind();
        self.as_anytype_mut().ok_or_else(|| {
            anyhow::anyhow!("Active backend is '{actual}', but {action} is only valid for anytype")
        })
    }

    /// Filesystem-backed backends expose a `repos_dir` for laying out the
    /// on-disk thoughts tree. Notion and Anytype have no such concept.
    pub fn filesystem_repos_dir(&self) -> Option<&str> {
        match self {
            BackendConfig::Git(g) => Some(&g.repos_dir),
            BackendConfig::Obsidian(o) => Some(&o.repos_dir),
            BackendConfig::Notion(_) | BackendConfig::Anytype(_) => None,
        }
    }
}

fn dispatch_mismatch(expected: BackendKind, actual: BackendKind) -> anyhow::Error {
    anyhow::anyhow!("{expected} backend dispatched on {actual} config")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    pub backend: BackendConfig,
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
    pub user: String,
    #[serde(default)]
    pub backend: BackendConfig,
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
    pub user: String,
    pub backend: BackendConfig,
    pub profile_name: Option<String>,
    pub mapped_name: Option<String>,
}

impl ThoughtsConfig {
    /// Check whether the essential thoughts fields are populated.
    /// Returns false when only AI-related fields were configured
    /// (e.g. after `hyprlayer ai configure` but before `thoughts init`).
    pub fn is_thoughts_configured(&self) -> bool {
        if self.user.is_empty() {
            return false;
        }
        match &self.backend {
            BackendConfig::Git(g) => {
                !g.thoughts_repo.is_empty() && !g.repos_dir.is_empty() && !g.global_dir.is_empty()
            }
            BackendConfig::Obsidian(o) => {
                !o.vault_path.is_empty() && !o.repos_dir.is_empty() && !o.global_dir.is_empty()
            }
            BackendConfig::Notion(n) => !n.parent_page_id.is_empty(),
            BackendConfig::Anytype(a) => !a.space_id.is_empty(),
        }
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

    /// Resolve the effective profile entry — the named profile if mapped, or
    /// the top-level backend config wrapped as a synthetic ProfileConfig.
    pub fn resolve_dirs(&self, profile: &Option<String>) -> ProfileConfig {
        profile
            .as_ref()
            .and_then(|name| self.profiles.get(name))
            .cloned()
            .unwrap_or(ProfileConfig {
                backend: self.backend.clone(),
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

    /// Mutable counterpart to `effective_config_for`'s backend resolution:
    /// returns `&mut` the backend for the profile this repo is mapped to,
    /// or `&mut self.backend` when there's no mapping.
    pub fn active_backend_mut(&mut self, repo_path: &str) -> Result<&mut BackendConfig> {
        let profile_name = self
            .repo_mappings
            .get(repo_path)
            .and_then(|m| m.profile())
            .map(|s| s.to_string());

        match profile_name {
            Some(name) => {
                let profile = self.profiles.get_mut(&name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Profile \"{}\" referenced by repo mapping does not exist",
                        name
                    )
                })?;
                Ok(&mut profile.backend)
            }
            None => Ok(&mut self.backend),
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

        let backend = profile_name
            .as_ref()
            .and_then(|n| self.profiles.get(n))
            .map(|p| p.backend.clone())
            .unwrap_or_else(|| self.backend.clone());

        EffectiveConfig {
            user: self.user.clone(),
            backend,
            profile_name,
            mapped_name: mapping.map(|m| m.repo().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for HyprlayerConfig {
    fn default() -> Self {
        Self {
            version: Some(3),
            last_version_check: None,
            disable_update_check: false,
            thoughts: None,
            ai: None,
        }
    }
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
    profiles: HashMap<String, V2ProfileConfig>,
    #[serde(default)]
    last_version_check: Option<i64>,
    #[serde(default)]
    disable_update_check: bool,
}

#[derive(Debug, Deserialize)]
struct V1ConfigFile {
    thoughts: Option<V1ThoughtsConfig>,
}

/// V2 backend settings — flat union of all fields. Used only by the v2→v3
/// migration path.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct V2BackendSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vault_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vault_subpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    database_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_token_env: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct V2ProfileConfig {
    #[serde(default)]
    thoughts_repo: String,
    #[serde(default)]
    repos_dir: String,
    #[serde(default)]
    global_dir: String,
    #[serde(default)]
    backend: BackendKind,
    #[serde(default)]
    backend_settings: V2BackendSettings,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct V2ThoughtsConfig {
    #[serde(default)]
    thoughts_repo: String,
    #[serde(default)]
    repos_dir: String,
    #[serde(default)]
    global_dir: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    backend: BackendKind,
    #[serde(default)]
    backend_settings: V2BackendSettings,
    #[serde(default)]
    repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default)]
    profiles: HashMap<String, V2ProfileConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct V2HyprlayerConfig {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    last_version_check: Option<i64>,
    #[serde(default)]
    disable_update_check: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thoughts: Option<V2ThoughtsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai: Option<AiConfig>,
}

#[derive(Deserialize)]
struct VersionPeek {
    #[serde(default)]
    version: Option<u32>,
}

impl HyprlayerConfig {
    /// Load config from a file path, auto-migrating older shapes (v1, v2) to v3.
    pub fn load(config_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let peek: VersionPeek =
            serde_json::from_str(&content).with_context(|| "Failed to parse config file")?;
        let version = peek.version.unwrap_or(0);

        let cfg = match version {
            0 | 1 => {
                let v2 = Self::migrate_v1(&content)?;
                Self::migrate_v2(&serde_json::to_string(&v2)?)?
            }
            2 => Self::migrate_v2(&content)?,
            3 => {
                serde_json::from_str(&content).with_context(|| "Failed to parse v3 config file")?
            }
            v => return Err(anyhow::anyhow!("Unknown config version: {v}")),
        };

        if version != 3 {
            cfg.save(config_path)?;
        }
        Ok(cfg)
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

    /// Migrate a v1 config (no version field) to a v2-shaped intermediate
    /// representation. The result is fed straight into `migrate_v2` to land
    /// on the live v3 shape — v1 is never deserialized into the live types.
    fn migrate_v1(content: &str) -> Result<V2HyprlayerConfig> {
        let v1: V1ConfigFile =
            serde_json::from_str(content).with_context(|| "Failed to parse v1 config")?;

        let Some(old) = v1.thoughts else {
            return Ok(V2HyprlayerConfig {
                version: Some(2),
                ..Default::default()
            });
        };

        let thoughts = V2ThoughtsConfig {
            thoughts_repo: old.thoughts_repo,
            repos_dir: old.repos_dir,
            global_dir: old.global_dir,
            user: old.user,
            backend: BackendKind::default(),
            backend_settings: V2BackendSettings::default(),
            repo_mappings: old.repo_mappings,
            profiles: old.profiles,
        };

        let ai = AiConfig {
            agent_tool: old.agent_tool,
            opencode_provider: old.opencode_provider,
            opencode_sonnet_model: old.opencode_sonnet_model,
            opencode_opus_model: old.opencode_opus_model,
        };

        Ok(V2HyprlayerConfig {
            version: Some(2),
            last_version_check: old.last_version_check,
            disable_update_check: old.disable_update_check,
            thoughts: Some(thoughts),
            ai: Some(ai),
        })
    }

    /// Migrate a v2-shaped JSON document to the v3 tagged-enum shape. The
    /// input is parsed via the `V2*` shadow types so the live `BackendConfig`
    /// is constructed via `build_v3_backend`, which also discards stale dead
    /// fields (e.g. `apiTokenEnv` left over from a prior backend).
    fn migrate_v2(content: &str) -> Result<HyprlayerConfig> {
        let v2: V2HyprlayerConfig =
            serde_json::from_str(content).with_context(|| "Failed to parse v2 config")?;

        let thoughts = v2.thoughts.map(|t| ThoughtsConfig {
            user: t.user,
            backend: build_v3_backend(
                t.backend,
                &t.backend_settings,
                &t.thoughts_repo,
                &t.repos_dir,
                &t.global_dir,
            ),
            repo_mappings: t.repo_mappings,
            profiles: t
                .profiles
                .into_iter()
                .map(|(k, p)| {
                    (
                        k,
                        ProfileConfig {
                            backend: build_v3_backend(
                                p.backend,
                                &p.backend_settings,
                                &p.thoughts_repo,
                                &p.repos_dir,
                                &p.global_dir,
                            ),
                        },
                    )
                })
                .collect(),
        });

        Ok(HyprlayerConfig {
            version: Some(3),
            last_version_check: v2.last_version_check,
            disable_update_check: v2.disable_update_check,
            thoughts,
            ai: v2.ai,
        })
    }
}

fn build_v3_backend(
    kind: BackendKind,
    s: &V2BackendSettings,
    thoughts_repo: &str,
    repos_dir: &str,
    global_dir: &str,
) -> BackendConfig {
    match kind {
        BackendKind::Git => BackendConfig::Git(GitConfig {
            thoughts_repo: thoughts_repo.to_string(),
            repos_dir: repos_dir.to_string(),
            global_dir: global_dir.to_string(),
        }),
        BackendKind::Obsidian => BackendConfig::Obsidian(ObsidianConfig {
            vault_path: s.vault_path.clone().unwrap_or_default(),
            vault_subpath: s.vault_subpath.clone(),
            repos_dir: repos_dir.to_string(),
            global_dir: global_dir.to_string(),
        }),
        BackendKind::Notion => BackendConfig::Notion(NotionConfig {
            parent_page_id: s.parent_page_id.clone().unwrap_or_default(),
            database_id: s.database_id.clone(),
        }),
        BackendKind::Anytype => BackendConfig::Anytype(AnytypeConfig {
            space_id: s.space_id.clone().unwrap_or_default(),
            type_id: s.type_id.clone(),
            api_token_env: s.api_token_env.clone(),
        }),
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

    fn git_thoughts(thoughts_repo: &str, repos_dir: &str, global_dir: &str) -> ThoughtsConfig {
        ThoughtsConfig {
            user: "testuser".to_string(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: thoughts_repo.to_string(),
                repos_dir: repos_dir.to_string(),
                global_dir: global_dir.to_string(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn thoughts_config_default_values() {
        let config = ThoughtsConfig::default();
        assert_eq!(config.user, "");
        assert_eq!(config.backend, BackendConfig::default());
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
            version: Some(3),
            last_version_check: Some(1700000000),
            disable_update_check: true,
            thoughts: Some(git_thoughts("~/thoughts", "repos", "global")),
            ai: Some(AiConfig {
                agent_tool: Some(AgentTool::Claude),
                ..Default::default()
            }),
        };

        config.save(&config_path).unwrap();
        let loaded = HyprlayerConfig::load(&config_path).unwrap();

        assert_eq!(loaded.version, Some(3));
        assert_eq!(loaded.last_version_check, Some(1700000000));
        assert!(loaded.disable_update_check);

        let thoughts = loaded.thoughts.unwrap();
        assert_eq!(
            thoughts.backend.as_git().unwrap().thoughts_repo,
            "~/thoughts"
        );
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
        let v2 = HyprlayerConfig::migrate_v1(json).unwrap();
        let serialized = serde_json::to_string(&v2).unwrap();
        let config = HyprlayerConfig::migrate_v2(&serialized).unwrap();

        assert_eq!(config.version, Some(3));
        assert_eq!(config.last_version_check, Some(1700000000));
        assert!(!config.disable_update_check);

        let thoughts = config.thoughts.unwrap();
        let git = thoughts.backend.as_git().unwrap();
        assert_eq!(git.thoughts_repo, "~/thoughts");
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
        let v2 = HyprlayerConfig::migrate_v1(json).unwrap();
        let config = HyprlayerConfig::migrate_v2(&serde_json::to_string(&v2).unwrap()).unwrap();
        let ai = config.ai.unwrap();
        assert!(matches!(ai.agent_tool, Some(AgentTool::Copilot)));

        let thoughts = config.thoughts.unwrap();
        assert!(!thoughts.is_thoughts_configured());
    }

    #[test]
    fn migrate_v1_no_thoughts_key() {
        let json = r#"{}"#;
        let v2 = HyprlayerConfig::migrate_v1(json).unwrap();
        let config = HyprlayerConfig::migrate_v2(&serde_json::to_string(&v2).unwrap()).unwrap();
        assert_eq!(config.version, Some(3));
        assert!(config.thoughts.is_none());
        assert!(config.ai.is_none());
    }

    #[test]
    fn migrate_v1_minimal_thoughts() {
        let json = r#"{"thoughts": {"thoughtsRepo": "~/t", "reposDir": "r", "globalDir": "g", "user": "u"}}"#;
        let v2 = HyprlayerConfig::migrate_v1(json).unwrap();
        let config = HyprlayerConfig::migrate_v2(&serde_json::to_string(&v2).unwrap()).unwrap();
        assert_eq!(config.version, Some(3));
        assert!(config.last_version_check.is_none());
        assert!(!config.disable_update_check);

        let thoughts = config.thoughts.unwrap();
        let git = thoughts.backend.as_git().unwrap();
        assert_eq!(git.thoughts_repo, "~/t");
        assert!(thoughts.is_thoughts_configured());

        let ai = config.ai.unwrap();
        assert!(ai.agent_tool.is_none());
    }

    #[test]
    fn migrate_v1_then_v2_chains_correctly() {
        // v1 config: no version key, AI fields under thoughts. Should land at v3.
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v1_chain");
        let config_path = temp_dir.join("config.json");
        fs::create_dir_all(&temp_dir).unwrap();

        let v1_json = r#"{
            "thoughts": {
                "thoughtsRepo": "~/thoughts",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "agentTool": "claude"
            }
        }"#;
        fs::write(&config_path, v1_json).unwrap();

        let cfg = HyprlayerConfig::load(&config_path).unwrap();
        assert_eq!(cfg.version, Some(3));
        let thoughts = cfg.thoughts.unwrap();
        let git = thoughts.backend.as_git().unwrap();
        assert_eq!(git.thoughts_repo, "~/thoughts");

        let on_disk: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 3);
        assert_eq!(on_disk["thoughts"]["backend"]["kind"], "git");

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn v3_config_does_not_trigger_migration() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v3_no_migrate");
        let config_path = temp_dir.join("config.json");

        let config = HyprlayerConfig {
            version: Some(3),
            thoughts: Some(git_thoughts("~/thoughts", "repos", "global")),
            ..Default::default()
        };

        config.save(&config_path).unwrap();
        let bytes_before = fs::read(&config_path).unwrap();
        let loaded = HyprlayerConfig::load(&config_path).unwrap();
        let bytes_after = fs::read(&config_path).unwrap();

        // Idempotency: loading a v3 file does not rewrite it.
        assert_eq!(bytes_before, bytes_after);

        assert_eq!(loaded.version, Some(3));
        let thoughts = loaded.thoughts.unwrap();
        assert_eq!(
            thoughts.backend.as_git().unwrap().thoughts_repo,
            "~/thoughts"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn migrate_v2_git_round_trip() {
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "~/thoughts",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "git"
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        assert_eq!(cfg.version, Some(3));
        let git = cfg.thoughts.unwrap().backend.as_git().cloned().unwrap();
        assert_eq!(git.thoughts_repo, "~/thoughts");
        assert_eq!(git.repos_dir, "repos");
        assert_eq!(git.global_dir, "global");
    }

    #[test]
    fn migrate_v2_obsidian_round_trip() {
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "obsidian",
                "backendSettings": {
                    "vaultPath": "/vault",
                    "vaultSubpath": "hyprlayer"
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let o = cfg
            .thoughts
            .unwrap()
            .backend
            .as_obsidian()
            .cloned()
            .unwrap();
        assert_eq!(o.vault_path, "/vault");
        assert_eq!(o.vault_subpath.as_deref(), Some("hyprlayer"));
        assert_eq!(o.repos_dir, "repos");
        assert_eq!(o.global_dir, "global");
    }

    #[test]
    fn migrate_v2_notion_round_trip() {
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "notion",
                "backendSettings": {
                    "parentPageId": "p1",
                    "databaseId": "d1"
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let n = cfg.thoughts.unwrap().backend.as_notion().cloned().unwrap();
        assert_eq!(n.parent_page_id, "p1");
        assert_eq!(n.database_id.as_deref(), Some("d1"));
    }

    #[test]
    fn migrate_v2_anytype_round_trip() {
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "anytype",
                "backendSettings": {
                    "spaceId": "s1",
                    "typeId": "t1",
                    "apiTokenEnv": "ANYTYPE_API_KEY"
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let a = cfg.thoughts.unwrap().backend.as_anytype().cloned().unwrap();
        assert_eq!(a.space_id, "s1");
        assert_eq!(a.type_id.as_deref(), Some("t1"));
        assert_eq!(a.api_token_env.as_deref(), Some("ANYTYPE_API_KEY"));
    }

    #[test]
    fn migrate_v2_strips_unused_filesystem_fields_from_notion() {
        // Reproducer: the v2 init path always wrote `reposDir: "repos"` and
        // `globalDir: "global"` even for Notion. After v3, those are NOT part
        // of NotionConfig, so the on-disk shape should not contain them.
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "/will/be/discarded",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "notion",
                "backendSettings": {
                    "parentPageId": "p1"
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let serialized = serde_json::to_value(&cfg).unwrap();
        let backend = &serialized["thoughts"]["backend"];
        assert_eq!(backend["kind"], "notion");
        assert!(backend.get("thoughtsRepo").is_none());
        assert!(backend.get("reposDir").is_none());
        assert!(backend.get("globalDir").is_none());
        assert!(serialized["thoughts"].get("thoughtsRepo").is_none());
        assert!(serialized["thoughts"].get("reposDir").is_none());
        assert!(serialized["thoughts"].get("globalDir").is_none());
        assert!(serialized["thoughts"].get("backendSettings").is_none());
    }

    #[test]
    fn migrate_v2_notion_drops_stale_anytype_token() {
        // v2 had a single shared BackendSettings union, so a stale `apiTokenEnv`
        // could be left over from a prior anytype init. The v3 NotionConfig
        // has no such field at all — migration should drop it.
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "notion",
                "backendSettings": {
                    "parentPageId": "p1",
                    "apiTokenEnv": "STALE_ANYTYPE_KEY",
                    "spaceId": "stale-space"
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let serialized = serde_json::to_value(&cfg).unwrap();
        let backend = &serialized["thoughts"]["backend"];
        assert_eq!(backend["kind"], "notion");
        assert!(backend.get("apiTokenEnv").is_none());
        assert!(backend.get("spaceId").is_none());
    }

    #[test]
    fn migrate_v2_preserves_profiles_with_mixed_backends() {
        let json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "~/t",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "git",
                "profiles": {
                    "obs": {
                        "thoughtsRepo": "",
                        "reposDir": "repos",
                        "globalDir": "global",
                        "backend": "obsidian",
                        "backendSettings": {
                            "vaultPath": "/vault"
                        }
                    },
                    "anyt": {
                        "thoughtsRepo": "",
                        "reposDir": "repos",
                        "globalDir": "global",
                        "backend": "anytype",
                        "backendSettings": {
                            "spaceId": "s1"
                        }
                    }
                }
            }
        }"#;
        let cfg = HyprlayerConfig::migrate_v2(json).unwrap();
        let thoughts = cfg.thoughts.unwrap();
        assert!(matches!(thoughts.backend, BackendConfig::Git(_)));
        let obs = thoughts.profiles.get("obs").unwrap();
        assert!(matches!(obs.backend, BackendConfig::Obsidian(_)));
        let any = thoughts.profiles.get("anyt").unwrap();
        assert!(matches!(any.backend, BackendConfig::Anytype(_)));
    }

    /// Phase 4 fixture test: a v2 Notion config on disk loads, migrates, and
    /// the rewritten file uses the v3 shape (`version: 3`, `backend: { kind:
    /// "notion", ... }`) with no leftover top-level filesystem fields.
    #[test]
    fn migrate_v2_notion_writes_v3_shape_to_disk() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v2_notion_disk");
        let config_path = temp_dir.join("config.json");
        fs::create_dir_all(&temp_dir).unwrap();

        let v2_json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "/should/not/appear",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "notion",
                "backendSettings": {
                    "parentPageId": "p1",
                    "databaseId": "d1"
                }
            }
        }"#;
        fs::write(&config_path, v2_json).unwrap();

        let cfg = HyprlayerConfig::load(&config_path).unwrap();
        assert_eq!(cfg.version, Some(3));
        assert!(matches!(
            cfg.thoughts.as_ref().unwrap().backend,
            BackendConfig::Notion(_)
        ));

        let on_disk: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 3);
        assert_eq!(on_disk["thoughts"]["backend"]["kind"], "notion");
        assert_eq!(on_disk["thoughts"]["backend"]["parentPageId"], "p1");
        assert_eq!(on_disk["thoughts"]["backend"]["databaseId"], "d1");
        assert!(on_disk["thoughts"].get("backendSettings").is_none());
        assert!(on_disk["thoughts"].get("thoughtsRepo").is_none());
        assert!(on_disk["thoughts"].get("reposDir").is_none());
        assert!(on_disk["thoughts"].get("globalDir").is_none());

        // Idempotency: a second load of the now-v3 file does not rewrite it.
        let bytes_after_first = fs::read(&config_path).unwrap();
        HyprlayerConfig::load(&config_path).unwrap();
        let bytes_after_second = fs::read(&config_path).unwrap();
        assert_eq!(bytes_after_first, bytes_after_second);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Phase 4 fixture test: v2 Obsidian writes a v3 ObsidianConfig.
    #[test]
    fn migrate_v2_obsidian_writes_v3_shape_to_disk() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v2_obsidian_disk");
        let config_path = temp_dir.join("config.json");
        fs::create_dir_all(&temp_dir).unwrap();

        let v2_json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "obsidian",
                "backendSettings": {
                    "vaultPath": "/vault",
                    "vaultSubpath": "hyprlayer"
                }
            }
        }"#;
        fs::write(&config_path, v2_json).unwrap();

        HyprlayerConfig::load(&config_path).unwrap();

        let on_disk: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 3);
        assert_eq!(on_disk["thoughts"]["backend"]["kind"], "obsidian");
        assert_eq!(on_disk["thoughts"]["backend"]["vaultPath"], "/vault");
        assert_eq!(on_disk["thoughts"]["backend"]["vaultSubpath"], "hyprlayer");
        assert_eq!(on_disk["thoughts"]["backend"]["reposDir"], "repos");
        assert_eq!(on_disk["thoughts"]["backend"]["globalDir"], "global");
        assert!(on_disk["thoughts"].get("backendSettings").is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Phase 4 fixture test: v2 Anytype writes a v3 AnytypeConfig.
    #[test]
    fn migrate_v2_anytype_writes_v3_shape_to_disk() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_v2_anytype_disk");
        let config_path = temp_dir.join("config.json");
        fs::create_dir_all(&temp_dir).unwrap();

        let v2_json = r#"{
            "version": 2,
            "thoughts": {
                "thoughtsRepo": "",
                "reposDir": "repos",
                "globalDir": "global",
                "user": "alice",
                "backend": "anytype",
                "backendSettings": {
                    "spaceId": "s1",
                    "typeId": "t1",
                    "apiTokenEnv": "ANYTYPE_API_KEY"
                }
            }
        }"#;
        fs::write(&config_path, v2_json).unwrap();

        HyprlayerConfig::load(&config_path).unwrap();

        let on_disk: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 3);
        assert_eq!(on_disk["thoughts"]["backend"]["kind"], "anytype");
        assert_eq!(on_disk["thoughts"]["backend"]["spaceId"], "s1");
        assert_eq!(on_disk["thoughts"]["backend"]["typeId"], "t1");
        assert_eq!(
            on_disk["thoughts"]["backend"]["apiTokenEnv"],
            "ANYTYPE_API_KEY"
        );
        assert!(on_disk["thoughts"].get("backendSettings").is_none());
        assert!(on_disk["thoughts"].get("reposDir").is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn backend_config_serde_round_trip() {
        for cfg in [
            BackendConfig::Git(GitConfig {
                thoughts_repo: "~/t".to_string(),
                repos_dir: "r".to_string(),
                global_dir: "g".to_string(),
            }),
            BackendConfig::Obsidian(ObsidianConfig {
                vault_path: "/v".to_string(),
                vault_subpath: Some("hyprlayer".to_string()),
                repos_dir: "r".to_string(),
                global_dir: "g".to_string(),
            }),
            BackendConfig::Notion(NotionConfig {
                parent_page_id: "p".to_string(),
                database_id: Some("d".to_string()),
            }),
            BackendConfig::Anytype(AnytypeConfig {
                space_id: "s".to_string(),
                type_id: Some("t".to_string()),
                api_token_env: Some("ANYTYPE_API_KEY".to_string()),
            }),
        ] {
            let json = serde_json::to_string(&cfg).unwrap();
            let back: BackendConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(back, cfg);
        }
    }

    #[test]
    fn backend_config_kind_returns_matching_discriminant() {
        assert_eq!(
            BackendConfig::Git(GitConfig::default()).kind(),
            BackendKind::Git
        );
        assert_eq!(
            BackendConfig::Obsidian(ObsidianConfig::default()).kind(),
            BackendKind::Obsidian
        );
        assert_eq!(
            BackendConfig::Notion(NotionConfig::default()).kind(),
            BackendKind::Notion
        );
        assert_eq!(
            BackendConfig::Anytype(AnytypeConfig::default()).kind(),
            BackendKind::Anytype
        );
    }

    #[test]
    fn backend_config_default_is_git() {
        assert_eq!(BackendConfig::default().kind(), BackendKind::Git);
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
    fn is_thoughts_configured_per_variant() {
        // Git: requires all three filesystem fields plus user.
        let mut t = git_thoughts("~/t", "r", "g");
        assert!(t.is_thoughts_configured());
        t.user = String::new();
        assert!(!t.is_thoughts_configured());

        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: "~/t".to_string(),
                repos_dir: String::new(),
                global_dir: "g".to_string(),
            }),
            ..Default::default()
        };
        assert!(!t.is_thoughts_configured());

        // Obsidian: requires vault_path + repos_dir + global_dir + user.
        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path: "/v".to_string(),
                vault_subpath: None,
                repos_dir: "r".to_string(),
                global_dir: "g".to_string(),
            }),
            ..Default::default()
        };
        assert!(t.is_thoughts_configured());

        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path: String::new(),
                vault_subpath: None,
                repos_dir: "r".to_string(),
                global_dir: "g".to_string(),
            }),
            ..Default::default()
        };
        assert!(!t.is_thoughts_configured());

        // Notion: requires parent_page_id + user.
        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Notion(NotionConfig {
                parent_page_id: "p".to_string(),
                database_id: None,
            }),
            ..Default::default()
        };
        assert!(t.is_thoughts_configured());

        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Notion(NotionConfig::default()),
            ..Default::default()
        };
        assert!(!t.is_thoughts_configured());

        // Anytype: requires space_id + user.
        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Anytype(AnytypeConfig {
                space_id: "s".to_string(),
                type_id: None,
                api_token_env: None,
            }),
            ..Default::default()
        };
        assert!(t.is_thoughts_configured());

        let t = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Anytype(AnytypeConfig::default()),
            ..Default::default()
        };
        assert!(!t.is_thoughts_configured());
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
    fn obsidian_root_joins_vault_and_subpath() {
        let s = ObsidianConfig {
            vault_path: "/vault".to_string(),
            vault_subpath: Some("hyprlayer".to_string()),
            repos_dir: "r".to_string(),
            global_dir: "g".to_string(),
        };
        assert_eq!(
            s.obsidian_root().unwrap(),
            PathBuf::from("/vault/hyprlayer")
        );
    }

    #[test]
    fn obsidian_root_handles_empty_subpath() {
        let s = ObsidianConfig {
            vault_path: "/vault".to_string(),
            vault_subpath: Some(String::new()),
            repos_dir: "r".to_string(),
            global_dir: "g".to_string(),
        };
        assert_eq!(s.obsidian_root().unwrap(), PathBuf::from("/vault"));
    }

    #[test]
    fn obsidian_root_handles_missing_subpath() {
        let s = ObsidianConfig {
            vault_path: "/vault".to_string(),
            vault_subpath: None,
            repos_dir: "r".to_string(),
            global_dir: "g".to_string(),
        };
        assert_eq!(s.obsidian_root().unwrap(), PathBuf::from("/vault"));
    }

    #[test]
    fn obsidian_root_expands_tilde() {
        let s = ObsidianConfig {
            vault_path: "~/vault".to_string(),
            vault_subpath: Some("hyprlayer".to_string()),
            repos_dir: "r".to_string(),
            global_dir: "g".to_string(),
        };
        let root = s.obsidian_root().unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(root, home.join("vault").join("hyprlayer"));
    }

    #[test]
    fn obsidian_root_returns_none_without_vault_path() {
        let s = ObsidianConfig::default();
        assert!(s.obsidian_root().is_none());
    }

    #[test]
    fn effective_config_resolves_backend_from_profile() {
        let mut cfg = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: "~/t".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            ..Default::default()
        };
        cfg.profiles.insert(
            "obs".to_string(),
            ProfileConfig {
                backend: BackendConfig::Obsidian(ObsidianConfig {
                    vault_path: "/vault".to_string(),
                    vault_subpath: None,
                    repos_dir: "repos".to_string(),
                    global_dir: "global".to_string(),
                }),
            },
        );
        cfg.repo_mappings.insert(
            "/some/repo".to_string(),
            RepoMapping::new("myproj", &Some("obs".to_string())),
        );

        let eff = cfg.effective_config_for("/some/repo");
        let obs = eff.backend.as_obsidian().unwrap();
        assert_eq!(obs.vault_path, "/vault");
        assert_eq!(eff.profile_name.as_deref(), Some("obs"));
    }

    #[test]
    fn effective_config_falls_back_to_top_level_backend() {
        let cfg = ThoughtsConfig {
            user: "u".to_string(),
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path: "/vault".to_string(),
                vault_subpath: None,
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            ..Default::default()
        };
        let eff = cfg.effective_config_for("/unmapped/repo");
        assert_eq!(eff.backend.kind(), BackendKind::Obsidian);
        assert_eq!(eff.backend.as_obsidian().unwrap().vault_path, "/vault");
        assert!(eff.mapped_name.is_none());
    }
}
