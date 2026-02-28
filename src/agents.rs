use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};
use std::process::Command;

const REPO: &str = "BrightBlock/hyprlayer-cli";
const BRANCH: &str = "master";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Claude,
    Copilot,
    OpenCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OpenCodeProvider {
    GithubCopilot,
    Anthropic,
    Abacus,
}

impl fmt::Display for OpenCodeProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubCopilot => write!(f, "GitHub Copilot"),
            Self::Anthropic => write!(f, "Anthropic"),
            Self::Abacus => write!(f, "Abacus"),
        }
    }
}

impl OpenCodeProvider {
    /// All available providers for selection prompts
    pub const ALL: &[OpenCodeProvider] = &[
        OpenCodeProvider::GithubCopilot,
        OpenCodeProvider::Anthropic,
        OpenCodeProvider::Abacus,
    ];

    /// Get the default sonnet model string for this provider
    /// Used for most commands and all agents
    pub fn default_sonnet_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-sonnet-4.5",
            Self::Anthropic => "anthropic/claude-sonnet-4-5",
            Self::Abacus => "abacus/claude-sonnet-4-5-20250929",
        }
    }

    /// Get the default opus model string for this provider
    /// Used for research_codebase, create_plan, and iterate_plan commands
    pub fn default_opus_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-opus-4.5",
            Self::Anthropic => "anthropic/claude-opus-4-5",
            Self::Abacus => "abacus/claude-opus-4-5-20251101",
        }
    }

    /// Get the provider prefix for model strings
    pub fn provider_prefix(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot",
            Self::Anthropic => "anthropic",
            Self::Abacus => "abacus",
        }
    }
}

impl fmt::Display for AgentTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude => write!(f, "Claude Code"),
            Self::Copilot => write!(f, "GitHub Copilot"),
            Self::OpenCode => write!(f, "OpenCode"),
        }
    }
}

impl AgentTool {
    /// All available variants, for use in selection prompts
    pub const ALL: &[AgentTool] = &[AgentTool::Claude, AgentTool::Copilot, AgentTool::OpenCode];

    /// The directory name in the repo that contains this tool's agent files
    fn repo_dir(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Copilot => "copilot",
            Self::OpenCode => "opencode",
        }
    }

    fn dest_dir(&self) -> Result<PathBuf> {
        match self {
            Self::Claude => {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                Ok(home.join(".claude"))
            }
            Self::Copilot => {
                let config = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
                Ok(config.join("Code").join("User"))
            }
            Self::OpenCode => {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                Ok(home.join(".config").join("opencode"))
            }
        }
    }

    /// Display the destination directory for user-facing messages
    pub fn dest_display(&self) -> String {
        match self {
            Self::Claude => format!("~{SEP}.claude{SEP}"),
            #[cfg(target_os = "linux")]
            Self::Copilot => format!("~{SEP}.config{SEP}Code{SEP}User{SEP}"),
            #[cfg(target_os = "macos")]
            Self::Copilot => {
                format!("~{SEP}Library{SEP}Application Support{SEP}Code{SEP}User{SEP}")
            }
            #[cfg(target_os = "windows")]
            Self::Copilot => format!("%APPDATA%{SEP}Code{SEP}User{SEP}"),
            Self::OpenCode => format!("~{SEP}.config{SEP}opencode{SEP}"),
        }
    }

    /// Check if agent files appear to be installed already.
    /// Returns true if the destination directory contains the expected subdirectories.
    pub fn is_installed(&self) -> bool {
        let Ok(dest) = self.dest_dir() else {
            return false;
        };
        match self {
            Self::Claude | Self::OpenCode => {
                dest.join("commands").is_dir() && dest.join("agents").is_dir()
            }
            Self::Copilot => dest.join("prompts").is_dir() && dest.join("agents").is_dir(),
        }
    }

    /// Print status information for this agent tool.
    /// OpenCode includes provider and model details from config.
    pub fn print_status(&self, config: &crate::config::ThoughtsConfig) {
        use colored::Colorize;

        println!("  AI Tool: {}", self.to_string().cyan());

        let status = if self.is_installed() {
            "installed".green()
        } else {
            "not installed".red()
        };
        println!("  Status: {}", status);
        println!("  Location: {}", self.dest_display().cyan());

        match self {
            Self::OpenCode => {
                println!();
                println!("  {}", "OpenCode Settings:".yellow());
                println!(
                    "    Provider: {}",
                    config
                        .opencode_provider
                        .as_ref()
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "not set".to_string())
                        .cyan()
                );
                println!(
                    "    Sonnet Model: {}",
                    config
                        .opencode_sonnet_model
                        .as_deref()
                        .unwrap_or("not set")
                        .cyan()
                );
                println!(
                    "    Opus Model: {}",
                    config
                        .opencode_opus_model
                        .as_deref()
                        .unwrap_or("not set")
                        .cyan()
                );
            }
            Self::Claude | Self::Copilot => {}
        }
    }

    /// Return status as JSON-serializable struct for --json output.
    pub fn status_json(&self, config: &crate::config::ThoughtsConfig) -> serde_json::Value {
        match self {
            Self::OpenCode => serde_json::json!({
                "agentTool": self.to_string(),
                "installed": self.is_installed(),
                "location": self.dest_display(),
                "opencodeProvider": config.opencode_provider.as_ref().map(|p| p.to_string()),
                "opencodeSonnetModel": config.opencode_sonnet_model.clone(),
                "opencodeOpusModel": config.opencode_opus_model.clone(),
            }),
            Self::Claude | Self::Copilot => serde_json::json!({
                "agentTool": self.to_string(),
                "installed": self.is_installed(),
                "location": self.dest_display(),
            }),
        }
    }

    /// Download agent files from GitHub and install to the destination.
    /// For OpenCode, optionally update model fields with provider-specific model.
    pub fn install(&self, opencode_provider: Option<&OpenCodeProvider>) -> Result<()> {
        let dest = self.dest_dir()?;
        fs::create_dir_all(&dest)?;

        println!("Downloading {} agent files...", self);
        let mut count = 0;
        download_directory(self.repo_dir(), &dest, &mut count)?;
        println!("  {:<60}", format!("Downloaded {} files", count));

        // Update model fields if OpenCode and provider specified
        if matches!(self, AgentTool::OpenCode)
            && let Some(provider) = opencode_provider
        {
            println!("Configuring models for {}...", provider);
            let updated = update_opencode_models(&dest, provider)?;
            println!("  {:<60}", format!("Updated {} files", updated));
        }

        Ok(())
    }
}

/// Download a directory from the repo using the GitHub Contents API.
/// Recursively fetches subdirectories and downloads each file individually.
///
/// API: GET /repos/{owner}/{repo}/contents/{path}?ref={branch}
/// Returns JSON array of entries with `type` ("file"|"dir"), `path`, and `download_url`.
fn download_directory(repo_path: &str, dest: &Path, count: &mut usize) -> Result<()> {
    let api_url = format!("https://api.github.com/repos/{REPO}/contents/{repo_path}?ref={BRANCH}");

    let json = curl_get_json(&api_url)?;

    // The API returns a JSON object with a "message" field on errors (e.g. 404)
    if let Ok(err) = serde_json::from_str::<GitHubError>(&json)
        && let Some(message) = err.message
    {
        return Err(anyhow::anyhow!(
            "Agent files for '{}' are not available on GitHub ({})",
            repo_path,
            message
        ));
    }

    let entries: Vec<GitHubEntry> =
        serde_json::from_str(&json).context("Failed to parse GitHub API response")?;

    for entry in entries {
        let dest_path = dest.join(&entry.name);
        match entry.entry_type.as_str() {
            "file" => {
                let url = entry
                    .download_url
                    .ok_or_else(|| anyhow::anyhow!("No download URL for {}", entry.path))?;
                print!("  {:<60}\r", entry.path);
                std::io::stdout().flush().ok();
                curl_download_file(&url, &dest_path)?;
                *count += 1;
            }
            "dir" => {
                fs::create_dir_all(&dest_path)?;
                download_directory(&entry.path, &dest_path, count)?;
            }
            _ => {} // skip symlinks, submodules, etc.
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct GitHubError {
    message: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    download_url: Option<String>,
}

/// GET a URL and return the response body as a string.
fn curl_get_json(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-sL",
            "-H",
            "Accept: application/vnd.github.v3+json",
            url,
        ])
        .output()
        .context("curl not found — install curl to download agent files")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("GitHub API request failed"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Download a single file to disk.
fn curl_download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let status = Command::new("curl")
        .args([
            "-sL",
            "-o",
            &dest.display().to_string(),
            url,
        ])
        .status()
        .context("curl not found")?;

    if !status.success() {
        return Err(anyhow::anyhow!("Failed to download {}", dest.display()));
    }
    Ok(())
}

/// Template placeholders used in OpenCode agent/command files
const SONNET_MODEL_PLACEHOLDER: &str = "{{SONNET_MODEL}}";
const OPUS_MODEL_PLACEHOLDER: &str = "{{OPUS_MODEL}}";

/// Replace model placeholders in a file with provider-specific values.
/// Returns true if any replacements were made.
fn replace_model_placeholders(path: &Path, provider: &OpenCodeProvider) -> Result<bool> {
    let content = fs::read_to_string(path)?;

    if !content.contains(SONNET_MODEL_PLACEHOLDER) && !content.contains(OPUS_MODEL_PLACEHOLDER) {
        return Ok(false);
    }

    let updated = content
        .replace(SONNET_MODEL_PLACEHOLDER, provider.default_sonnet_model())
        .replace(OPUS_MODEL_PLACEHOLDER, provider.default_opus_model());

    fs::write(path, updated)?;
    Ok(true)
}

/// Update all model placeholders in OpenCode agent/command files.
/// Files use {{SONNET_MODEL}} and {{OPUS_MODEL}} placeholders.
fn update_opencode_models(dest_dir: &Path, provider: &OpenCodeProvider) -> Result<usize> {
    let dirs = ["agents", "commands"];

    dirs.iter()
        .filter_map(|dir| {
            let path = dest_dir.join(dir);
            path.is_dir().then_some(path)
        })
        .flat_map(|dir| fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .try_fold(0, |count, entry| {
            let updated = replace_model_placeholders(&entry.path(), provider)?;
            Ok::<_, anyhow::Error>(count + usize::from(updated))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_display_uses_platform_separator() {
        for tool in AgentTool::ALL {
            let display = tool.dest_display();
            assert!(
                !display.contains(if SEP == "/" { "\\" } else { "/" }),
                "{} dest_display contains wrong separator: {}",
                tool,
                display
            );
            assert!(
                display.ends_with(SEP),
                "{} dest_display should end with SEP: {}",
                tool,
                display
            );
        }
    }

    #[test]
    fn dest_display_claude_contains_claude_dir() {
        let display = AgentTool::Claude.dest_display();
        assert!(
            display.contains(".claude"),
            "Expected .claude in: {}",
            display
        );
    }

    #[test]
    fn dest_display_opencode_contains_opencode_dir() {
        let display = AgentTool::OpenCode.dest_display();
        assert!(
            display.contains("opencode"),
            "Expected opencode in: {}",
            display
        );
    }

    #[test]
    fn dest_display_copilot_contains_code_user() {
        let display = AgentTool::Copilot.dest_display();
        assert!(
            display.contains(&format!("Code{SEP}User")),
            "Expected Code{}User in: {}",
            SEP,
            display
        );
    }

    #[test]
    fn opencode_provider_serializes_to_kebab_case() {
        let json = serde_json::to_string(&OpenCodeProvider::GithubCopilot).unwrap();
        assert_eq!(json, "\"github-copilot\"");

        let json = serde_json::to_string(&OpenCodeProvider::Anthropic).unwrap();
        assert_eq!(json, "\"anthropic\"");

        let json = serde_json::to_string(&OpenCodeProvider::Abacus).unwrap();
        assert_eq!(json, "\"abacus\"");
    }

    #[test]
    fn opencode_provider_deserializes_from_kebab_case() {
        let provider: OpenCodeProvider = serde_json::from_str("\"github-copilot\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::GithubCopilot);

        let provider: OpenCodeProvider = serde_json::from_str("\"anthropic\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::Anthropic);

        let provider: OpenCodeProvider = serde_json::from_str("\"abacus\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::Abacus);
    }

    #[test]
    fn opencode_provider_display_names() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.to_string(),
            "GitHub Copilot"
        );
        assert_eq!(OpenCodeProvider::Anthropic.to_string(), "Anthropic");
        assert_eq!(OpenCodeProvider::Abacus.to_string(), "Abacus");
    }

    #[test]
    fn opencode_provider_sonnet_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_sonnet_model(),
            "github-copilot/claude-sonnet-4.5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_sonnet_model(),
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_sonnet_model(),
            "abacus/claude-sonnet-4-5-20250929"
        );
    }

    #[test]
    fn opencode_provider_opus_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_opus_model(),
            "github-copilot/claude-opus-4.5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_opus_model(),
            "anthropic/claude-opus-4-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_opus_model(),
            "abacus/claude-opus-4-5-20251101"
        );
    }

    #[test]
    fn opencode_provider_prefixes() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.provider_prefix(),
            "github-copilot"
        );
        assert_eq!(OpenCodeProvider::Anthropic.provider_prefix(), "anthropic");
        assert_eq!(OpenCodeProvider::Abacus.provider_prefix(), "abacus");
    }

    #[test]
    fn replace_model_placeholders_replaces_sonnet() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_sonnet_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("test_agent.md");

        let content = "---\nmodel: {{SONNET_MODEL}}\n---\n# Agent";
        fs::write(&file_path, content).unwrap();

        let updated =
            replace_model_placeholders(&file_path, &OpenCodeProvider::GithubCopilot).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: github-copilot/claude-sonnet-4.5"));
        assert!(!result.contains("{{SONNET_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_replaces_opus() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_opus_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("research.md");

        let content = "---\nmodel: {{OPUS_MODEL}}\n---\n# Research";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Abacus).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: abacus/claude-opus-4-5-20251101"));
        assert!(!result.contains("{{OPUS_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_skips_files_without_placeholders() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_no_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("no_placeholder.md");

        let content = "---\ndescription: No model field\n---\n# Test";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Anthropic).unwrap();
        assert!(!updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert_eq!(result, content);

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn update_opencode_models_replaces_placeholders() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_opencode_placeholders");
        let agents_dir = temp_dir.join("agents");
        let commands_dir = temp_dir.join("commands");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::create_dir_all(&commands_dir).unwrap();

        // Agent with sonnet placeholder
        fs::write(
            agents_dir.join("analyzer.md"),
            "---\nmodel: {{SONNET_MODEL}}\n---\n# Analyzer",
        )
        .unwrap();

        // Command with opus placeholder
        fs::write(
            commands_dir.join("research.md"),
            "---\nmodel: {{OPUS_MODEL}}\n---\n# Research",
        )
        .unwrap();

        // Command without placeholder (should not count)
        fs::write(
            commands_dir.join("commit.md"),
            "---\ndescription: Commit\n---\n# Commit",
        )
        .unwrap();

        let count = update_opencode_models(&temp_dir, &OpenCodeProvider::GithubCopilot).unwrap();
        assert_eq!(count, 2); // Only files with placeholders

        let agent = fs::read_to_string(agents_dir.join("analyzer.md")).unwrap();
        assert!(agent.contains("model: github-copilot/claude-sonnet-4.5"));

        let research = fs::read_to_string(commands_dir.join("research.md")).unwrap();
        assert!(research.contains("model: github-copilot/claude-opus-4.5"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn update_opencode_models_with_different_providers() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_providers");
        let commands_dir = temp_dir.join("commands");
        fs::create_dir_all(&commands_dir).unwrap();

        // Test with Anthropic
        fs::write(
            commands_dir.join("test.md"),
            "---\nmodel: {{SONNET_MODEL}}\nopus: {{OPUS_MODEL}}\n---\n# Test",
        )
        .unwrap();

        update_opencode_models(&temp_dir, &OpenCodeProvider::Anthropic).unwrap();

        let result = fs::read_to_string(commands_dir.join("test.md")).unwrap();
        assert!(result.contains("model: anthropic/claude-sonnet-4-5"));
        assert!(result.contains("opus: anthropic/claude-opus-4-5"));

        fs::remove_dir_all(&temp_dir).ok();
    }
}
