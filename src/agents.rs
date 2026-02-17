use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR as SEP};
use std::process::Command;

const REPO: &str = "BrightBlock/hyprlayer-cli";
const BRANCH: &str = "master";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Claude,
    Copilot,
    OpenCode,
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
            Self::Copilot => format!("~{SEP}Library{SEP}Application Support{SEP}Code{SEP}User{SEP}"),
            #[cfg(target_os = "windows")]
            Self::Copilot => format!("%APPDATA%{SEP}Code{SEP}User{SEP}"),
            Self::OpenCode => format!("~{SEP}.config{SEP}opencode{SEP}"),
        }
    }

    /// Download agent files from GitHub and install to the destination.
    /// Uses the GitHub Contents API to fetch only the specific directory needed.
    pub fn install(&self) -> Result<()> {
        let dest = self.dest_dir()?;
        fs::create_dir_all(&dest)?;

        let token = get_github_token().ok_or_else(|| {
            anyhow::anyhow!(
                "GitHub authentication required. Set GITHUB_TOKEN or run `gh auth login`."
            )
        })?;

        println!("Downloading {} agent files...", self);
        let mut count = 0;
        download_directory(&token, self.repo_dir(), &dest, &mut count)?;
        println!("  {:<60}", format!("Downloaded {} files", count));

        Ok(())
    }
}

/// Resolve a GitHub auth token.
/// Mirrors the installer: try GITHUB_TOKEN env var first, then `gh auth token`.
fn get_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            Command::new("gh")
                .args(["auth", "token"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|t| !t.is_empty())
        })
}

/// Download a directory from the repo using the GitHub Contents API.
/// Recursively fetches subdirectories and downloads each file individually.
///
/// API: GET /repos/{owner}/{repo}/contents/{path}?ref={branch}
/// Returns JSON array of entries with `type` ("file"|"dir"), `path`, and `download_url`.
fn download_directory(token: &str, repo_path: &str, dest: &Path, count: &mut usize) -> Result<()> {
    let api_url = format!(
        "https://api.github.com/repos/{REPO}/contents/{repo_path}?ref={BRANCH}"
    );

    let json = curl_get_json(&api_url, token)?;

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
                curl_download_file(&url, &dest_path, token)?;
                *count += 1;
            }
            "dir" => {
                fs::create_dir_all(&dest_path)?;
                download_directory(token, &entry.path, &dest_path, count)?;
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
fn curl_get_json(url: &str, token: &str) -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-sL",
            "-H",
            &format!("Authorization: token {token}"),
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
fn curl_download_file(url: &str, dest: &Path, token: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let status = Command::new("curl")
        .args([
            "-sL",
            "-H",
            &format!("Authorization: token {token}"),
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
        assert!(display.contains(".claude"), "Expected .claude in: {}", display);
    }

    #[test]
    fn dest_display_opencode_contains_opencode_dir() {
        let display = AgentTool::OpenCode.dest_display();
        assert!(display.contains("opencode"), "Expected opencode in: {}", display);
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
}
