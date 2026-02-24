use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::{AgentTool, OpenCodeProvider};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
    pub user: String,
    #[serde(default)]
    pub agent_tool: Option<AgentTool>,
    #[serde(default)]
    pub opencode_provider: Option<OpenCodeProvider>,
    #[serde(default)]
    pub opencode_sonnet_model: Option<String>,
    #[serde(default)]
    pub opencode_opus_model: Option<String>,
    #[serde(default)]
    pub repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

/// Effective configuration for a specific repository
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub thoughts_repo: String,
    pub repos_dir: String,
    pub global_dir: String,
    pub profile_name: Option<String>,
    pub mapped_name: Option<String>,
}

impl ThoughtsConfig {
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
            })
    }

    /// Load config from a file path
    pub fn load(config_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let config_file: ConfigFile = serde_json::from_str(&content)
            .with_context(|| "Failed to parse config file")?;
        config_file
            .thoughts
            .ok_or_else(|| anyhow::anyhow!("No thoughts configuration found in config file"))
    }

    /// Save config to a file path
    pub fn save(&self, config_path: &Path) -> Result<()> {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
        }
        let content = serde_json::json!({ "thoughts": self });
        let json = serde_json::to_string_pretty(&content)?;
        fs::write(config_path, json)
            .with_context(|| format!("Failed to write config file: {}", config_path.display()))?;
        Ok(())
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
            profile_name,
            mapped_name: mapping.map(|m| m.repo().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thoughts: Option<ThoughtsConfig>,
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

