use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};
use std::process::Command;

const REPO: &str = "BrightBlock/hyprlayer-cli";
const BRANCH: &str = "master";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
            Self::Abacus => "abacus/claude-sonnet-4-6",
        }
    }

    /// Get the default opus model string for this provider
    /// Used for research_codebase, create_plan, and iterate_plan commands
    pub fn default_opus_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-opus-4.5",
            Self::Anthropic => "anthropic/claude-opus-4-5",
            Self::Abacus => "abacus/claude-opus-4-6",
        }
    }

    /// Get the default model used for adversarial code reviews.
    /// Abacus routes to its highest-reasoning codex variant for a true
    /// cross-model second opinion; GitHub Copilot uses gpt-5-codex (the
    /// codex variant exposed through Copilot Chat); Anthropic stays on
    /// claude-opus-4-5 because the Anthropic API is Claude-only.
    pub fn default_adversarial_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/gpt-5-codex",
            Self::Anthropic => "anthropic/claude-opus-4-5",
            Self::Abacus => "abacus/gpt-5.3-codex-xhigh",
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
    pub(crate) fn repo_dir(&self) -> &str {
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
        self.is_installed_at(&dest)
    }

    /// Looser variant: does any prior install exist at `dest_dir`, even if
    /// it predates the current sentinel-file set? Used by the auto-reinstall
    /// gate so that exactly the stale installs that need refreshing get
    /// refreshed. `is_installed` would return false for them and the auto
    /// path would never run.
    pub fn has_existing_install(&self) -> bool {
        let Ok(dest) = self.dest_dir() else {
            return false;
        };
        self.has_existing_install_at(&dest)
    }

    fn has_existing_install_at(&self, dest: &Path) -> bool {
        // Per-tool structural directories that have been part of every
        // shipped bundle. If both exist, *something* was installed here
        // by a previous `hyprlayer ai configure`.
        let (a, b) = match self {
            Self::Claude => ("skills", "agents"),
            Self::OpenCode => ("commands", "agents"),
            Self::Copilot => ("prompts", "agents"),
        };
        dest.join(a).is_dir() && dest.join(b).is_dir()
    }

    /// Test-friendly variant of `is_installed` that takes an explicit destination path.
    ///
    /// Checks for sentinel files unique to the current bundle of
    /// commands/skills/agents. An older install with the right top-level
    /// directories but missing newly added files reports not-installed, so
    /// `configure --no-force` re-runs and provisions the new bundle. Bump
    /// these whenever we ship a top-level file existing users should pick up.
    fn is_installed_at(&self, dest: &Path) -> bool {
        match self {
            Self::Claude => {
                dest.join("skills/code_review/SKILL.md").is_file()
                    && dest.join("agents/codebase-locator.md").is_file()
            }
            Self::OpenCode => {
                dest.join("commands/code_review.md").is_file()
                    && dest.join("agents/codebase-locator.md").is_file()
            }
            Self::Copilot => {
                dest.join("prompts/code_review.prompt.md").is_file()
                    && dest.join("agents/codebase-locator.agent.md").is_file()
            }
        }
    }

    /// Print status information for this agent tool.
    /// OpenCode includes provider and model details from config.
    pub fn print_status(&self, config: &crate::config::AiConfig) {
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
    pub fn status_json(&self, config: &crate::config::AiConfig) -> serde_json::Value {
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
    ///
    /// Returns `Some(sha)` when we successfully captured the per-tool
    /// `master` commit SHA *before* the download (the next 24h auto-check
    /// uses this as the freshness baseline). Returns `None` when the
    /// commits API was unreachable but the downloads succeeded — the
    /// install is still good, but we have no SHA to cache, so the next
    /// auto-check will treat the bundle as stale and refresh again. We
    /// don't fail the whole install on commits-API rate-limits because
    /// `hyprlayer ai configure` / `ai reinstall` must continue to work
    /// even when only the commits endpoint is throttled.
    pub fn install(
        &self,
        opencode_provider: Option<&OpenCodeProvider>,
        quiet: bool,
    ) -> Result<Option<String>> {
        let dest = self.dest_dir()?;
        fs::create_dir_all(&dest)?;

        // Recording a post-download SHA could mask `master`-advances that
        // happen mid-install — next-day's check would then compare against
        // an at-or-newer cache and skip the necessary re-sync.
        let sha = fetch_repo_dir_sha(self.repo_dir()).ok();
        let git_ref = sha.as_deref().unwrap_or(BRANCH);

        if !quiet {
            println!("Downloading {} agent files...", self);
        }
        let mut count = 0;
        download_directory(self.repo_dir(), git_ref, &dest, &mut count, quiet)?;
        if !quiet {
            println!("  {:<60}", format!("Downloaded {} files", count));
        }

        if matches!(self, AgentTool::OpenCode)
            && let Some(provider) = opencode_provider
        {
            if !quiet {
                println!("Configuring models for {}...", provider);
            }
            let updated = update_opencode_models(&dest, provider)?;
            if !quiet {
                println!("  {:<60}", format!("Updated {} files", updated));
            }
        }

        Ok(sha)
    }
}

/// Fetch the latest `master` commit SHA that touched `repo_path`.
pub(crate) fn fetch_repo_dir_sha(repo_path: &str) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{REPO}/commits?path={repo_path}&sha={BRANCH}&per_page=1"
    );
    let json = curl_get_json(&url, Some(5))?;
    parse_repo_dir_sha(&json, repo_path)
}

fn parse_repo_dir_sha(json: &str, repo_path: &str) -> Result<String> {
    // GitHub returns an object with `message` on errors (e.g. 403
    // rate-limited); detect that before assuming the array shape.
    if let Ok(err) = serde_json::from_str::<GitHubError>(json)
        && let Some(message) = err.message
    {
        return Err(anyhow::anyhow!(
            "GitHub commits API error for '{}': {}",
            repo_path,
            message
        ));
    }

    let entries: Vec<serde_json::Value> =
        serde_json::from_str(json).context("Failed to parse GitHub commits API response")?;
    entries
        .first()
        .and_then(|e| e.get("sha"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("GitHub returned no commits for '{}'", repo_path))
}

/// Download a directory from the repo using the GitHub Contents API.
/// Recursively fetches subdirectories and downloads each file individually.
///
/// `git_ref` is the resolved commit SHA (or branch name) to pin every
/// listing + raw fetch to. Pinning across the recursion prevents a
/// mid-install `master` advance from producing a torn install where
/// some files come from commit A and others from commit B.
fn download_directory(
    repo_path: &str,
    git_ref: &str,
    dest: &Path,
    count: &mut usize,
    quiet: bool,
) -> Result<()> {
    let api_url = format!("https://api.github.com/repos/{REPO}/contents/{repo_path}?ref={git_ref}");

    let json = curl_get_json(&api_url, Some(15))?;

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
                // The contents API's `download_url` is already pinned to
                // the `?ref=<git_ref>` we requested, so reusing it keeps
                // the whole download tied to a single SHA.
                let url = entry
                    .download_url
                    .ok_or_else(|| anyhow::anyhow!("No download URL for {}", entry.path))?;
                if !quiet {
                    print!("  {:<60}\r", entry.path);
                    std::io::stdout().flush().ok();
                }
                curl_download_file(&url, &dest_path)?;
                *count += 1;
            }
            "dir" => {
                // No explicit `create_dir_all` here — `curl_download_file`
                // creates each file's parent on demand, which covers this
                // subdir as soon as we download anything into it.
                download_directory(&entry.path, git_ref, &dest_path, count, quiet)?;
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
/// Optionally applies a timeout (in seconds) via curl's `--max-time`.
pub(crate) fn curl_get_json(url: &str, timeout_secs: Option<u32>) -> Result<String> {
    let timeout_str = timeout_secs.map(|s| s.to_string());
    let mut args = vec![
        "-sL",
        "-H",
        "Accept: application/vnd.github.v3+json",
        "-H",
        "User-Agent: hyprlayer-cli",
    ];
    if let Some(ref t) = timeout_str {
        args.extend(["--max-time", t]);
    }
    args.push(url);

    let output = Command::new("curl")
        .args(&args)
        .output()
        .context("curl not found — install curl to download agent files")?;

    if !output.status.success() {
        anyhow::bail!("GitHub API request failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Download a single file to disk.
///
/// `--fail-with-body` makes curl exit non-zero on HTTP 4xx/5xx so a 404
/// HTML page or rate-limit JSON envelope can never be persisted as a
/// fake "agent file." `--max-time` caps the per-file fetch so a stalled
/// connection on the startup auto-reinstall path can't hang the user's
/// command indefinitely.
fn curl_download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let dest_str = dest.display().to_string();
    let status = Command::new("curl")
        .args([
            "-sSL",
            "--fail-with-body",
            "--max-time",
            "30",
            "-o",
            &dest_str,
            url,
        ])
        .status()
        .context("curl not found")?;

    if !status.success() {
        // Don't leave a partial / error-page body on disk.
        let _ = fs::remove_file(dest);
        return Err(anyhow::anyhow!("Failed to download {}", dest.display()));
    }
    Ok(())
}

/// Template placeholders used in OpenCode agent/command files
const SONNET_MODEL_PLACEHOLDER: &str = "{{SONNET_MODEL}}";
const OPUS_MODEL_PLACEHOLDER: &str = "{{OPUS_MODEL}}";
const ADVERSARIAL_MODEL_PLACEHOLDER: &str = "{{ADVERSARIAL_MODEL}}";

/// Replace model placeholders in a file with provider-specific values.
/// Returns true if any replacements were made.
fn replace_model_placeholders(path: &Path, provider: &OpenCodeProvider) -> Result<bool> {
    let content = fs::read_to_string(path)?;

    if !content.contains(SONNET_MODEL_PLACEHOLDER)
        && !content.contains(OPUS_MODEL_PLACEHOLDER)
        && !content.contains(ADVERSARIAL_MODEL_PLACEHOLDER)
    {
        return Ok(false);
    }

    let updated = content
        .replace(SONNET_MODEL_PLACEHOLDER, provider.default_sonnet_model())
        .replace(OPUS_MODEL_PLACEHOLDER, provider.default_opus_model())
        .replace(
            ADVERSARIAL_MODEL_PLACEHOLDER,
            provider.default_adversarial_model(),
        );

    fs::write(path, updated)?;
    Ok(true)
}

/// Update all model placeholders in OpenCode agent/command files.
/// Files use {{SONNET_MODEL}}, {{OPUS_MODEL}}, and {{ADVERSARIAL_MODEL}} placeholders.
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

    /// Create `path` (and any missing parent dirs) as an empty stub file.
    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "stub").unwrap();
    }

    #[test]
    fn parse_repo_dir_sha_happy_path() {
        let json = r#"[{"sha":"abc123def456","commit":{"message":"x"}}]"#;
        let sha = parse_repo_dir_sha(json, "claude").unwrap();
        assert_eq!(sha, "abc123def456");
    }

    #[test]
    fn parse_repo_dir_sha_picks_first_entry() {
        let json = r#"[
            {"sha":"first","commit":{"message":"a"}},
            {"sha":"second","commit":{"message":"b"}}
        ]"#;
        let sha = parse_repo_dir_sha(json, "claude").unwrap();
        assert_eq!(sha, "first");
    }

    #[test]
    fn parse_repo_dir_sha_empty_array_errors() {
        let err = parse_repo_dir_sha("[]", "claude").unwrap_err();
        assert!(
            err.to_string().contains("no commits"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_repo_dir_sha_missing_sha_field_errors() {
        let json = r#"[{"commit":{"message":"x"}}]"#;
        let err = parse_repo_dir_sha(json, "claude").unwrap_err();
        assert!(
            err.to_string().contains("no commits"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_repo_dir_sha_non_string_sha_errors() {
        let json = r#"[{"sha":42}]"#;
        let err = parse_repo_dir_sha(json, "claude").unwrap_err();
        assert!(
            err.to_string().contains("no commits"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_repo_dir_sha_malformed_json_errors() {
        let err = parse_repo_dir_sha("not json", "claude").unwrap_err();
        assert!(
            err.to_string().contains("Failed to parse"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_repo_dir_sha_propagates_github_error_message() {
        // GitHub returns an object with a `message` field on errors (e.g.
        // 403 rate-limited). The parser must surface the message rather
        // than emit a generic "failed to parse" error.
        let json = r#"{"message":"API rate limit exceeded","documentation_url":"..."}"#;
        let err = parse_repo_dir_sha(json, "claude").unwrap_err();
        assert!(
            err.to_string().contains("API rate limit exceeded"),
            "unexpected error: {err}"
        );
    }

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
            "abacus/claude-sonnet-4-6"
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
            "abacus/claude-opus-4-6"
        );
    }

    #[test]
    fn opencode_provider_adversarial_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_adversarial_model(),
            "github-copilot/gpt-5-codex"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_adversarial_model(),
            "anthropic/claude-opus-4-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_adversarial_model(),
            "abacus/gpt-5.3-codex-xhigh"
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
        assert!(result.contains("model: abacus/claude-opus-4-6"));
        assert!(!result.contains("{{OPUS_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_replaces_adversarial() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_adversarial_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("adversarial-reviewer.md");

        let content = "---\nmodel: {{ADVERSARIAL_MODEL}}\n---\n# Adversarial";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Abacus).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: abacus/gpt-5.3-codex-xhigh"));
        assert!(!result.contains("{{ADVERSARIAL_MODEL}}"));

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
    fn update_opencode_models_replaces_adversarial_alongside_others() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_adversarial_with_others");
        let agents_dir = temp_dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        fs::write(
            agents_dir.join("adversarial-reviewer.md"),
            "---\nmodel: {{ADVERSARIAL_MODEL}}\n---\n# Adversarial",
        )
        .unwrap();
        fs::write(
            agents_dir.join("analyzer.md"),
            "---\nmodel: {{SONNET_MODEL}}\n---\n# Analyzer",
        )
        .unwrap();

        let count = update_opencode_models(&temp_dir, &OpenCodeProvider::Abacus).unwrap();
        assert_eq!(count, 2);

        let adversarial = fs::read_to_string(agents_dir.join("adversarial-reviewer.md")).unwrap();
        assert!(adversarial.contains("model: abacus/gpt-5.3-codex-xhigh"));
        assert!(!adversarial.contains("{{ADVERSARIAL_MODEL}}"));

        let analyzer = fs::read_to_string(agents_dir.join("analyzer.md")).unwrap();
        assert!(analyzer.contains("model: abacus/claude-sonnet-4-6"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Round-trip test: copy the real shipped opencode/agents/adversarial-reviewer.md
    /// into a tempdir and verify substitution leaves no `{{...}}` placeholders behind
    /// for any provider. Catches regressions where someone removes the placeholder
    /// from the template or adds a new placeholder without updating the substitution
    /// machinery.
    #[test]
    fn opencode_adversarial_reviewer_template_substitutes_for_all_providers() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let template = manifest_dir.join("opencode/agents/adversarial-reviewer.md");
        let template_body = fs::read_to_string(&template).expect("opencode template missing");

        for provider in OpenCodeProvider::ALL {
            let temp_dir = std::env::temp_dir().join(format!(
                "hyprlayer_test_real_template_{}",
                provider.provider_prefix()
            ));
            let agents_dir = temp_dir.join("agents");
            fs::create_dir_all(&agents_dir).unwrap();
            fs::write(agents_dir.join("adversarial-reviewer.md"), &template_body).unwrap();

            update_opencode_models(&temp_dir, provider).unwrap();

            let resolved = fs::read_to_string(agents_dir.join("adversarial-reviewer.md")).unwrap();
            assert!(
                !resolved.contains("{{"),
                "{:?} substitution left a `{{{{...}}}}` placeholder in the template:\n{}",
                provider,
                resolved
            );
            assert!(
                resolved.contains(&format!("model: {}", provider.default_adversarial_model())),
                "{:?} did not produce the expected model line. Got:\n{}",
                provider,
                resolved
            );

            fs::remove_dir_all(&temp_dir).ok();
        }
    }

    #[test]
    fn claude_is_installed_requires_skills() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_claude_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("skills/code_review/SKILL.md"));
        touch(&case_full.join("agents/codebase-locator.md"));
        assert!(AgentTool::Claude.is_installed_at(&case_full));

        // Existing install with the right top-level dirs but no sentinels —
        // configure --no-force must re-run to provision the new bundle.
        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("skills")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_dirs_only));

        // Old layout (commands/ instead of skills/) must report not-installed.
        let case_legacy = temp_root.join("commands_and_agents");
        fs::create_dir_all(case_legacy.join("commands")).unwrap();
        fs::create_dir_all(case_legacy.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_legacy));

        let case_skills_only = temp_root.join("skills_only");
        fs::create_dir_all(case_skills_only.join("skills")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_skills_only));

        let case_agents_only = temp_root.join("agents_only");
        fs::create_dir_all(case_agents_only.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_agents_only));

        let case_no_agent = temp_root.join("no_locator_agent");
        touch(&case_no_agent.join("skills/code_review/SKILL.md"));
        fs::create_dir_all(case_no_agent.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_no_agent));

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn opencode_is_installed_requires_code_review_and_codebase_locator() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_opencode_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("commands/code_review.md"));
        touch(&case_full.join("agents/codebase-locator.md"));
        assert!(AgentTool::OpenCode.is_installed_at(&case_full));

        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("commands")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::OpenCode.is_installed_at(&case_dirs_only));

        let case_no_agent = temp_root.join("no_locator_agent");
        touch(&case_no_agent.join("commands/code_review.md"));
        fs::create_dir_all(case_no_agent.join("agents")).unwrap();
        assert!(!AgentTool::OpenCode.is_installed_at(&case_no_agent));

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn copilot_is_installed_requires_code_review_and_codebase_locator() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_copilot_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("prompts/code_review.prompt.md"));
        touch(&case_full.join("agents/codebase-locator.agent.md"));
        assert!(AgentTool::Copilot.is_installed_at(&case_full));

        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("prompts")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::Copilot.is_installed_at(&case_dirs_only));

        fs::remove_dir_all(&temp_root).ok();
    }

    /// `has_existing_install` must accept any layout that *was* a valid
    /// install at some point — sentinel files may have moved/renamed
    /// between bundles, but the structural directories haven't. A pre-
    /// `code_review` install is exactly the case the auto-reinstall path
    /// needs to refresh.
    #[test]
    fn has_existing_install_accepts_dirs_without_current_sentinels() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_has_existing_install");
        fs::remove_dir_all(&temp_root).ok();

        for (tool, dir_a, dir_b) in [
            (AgentTool::Claude, "skills", "agents"),
            (AgentTool::OpenCode, "commands", "agents"),
            (AgentTool::Copilot, "prompts", "agents"),
        ] {
            // Bare structural dirs (no sentinels) — `is_installed_at`
            // would reject this; `has_existing_install_at` must accept it.
            let dest = temp_root.join(format!("{tool:?}_dirs_only"));
            fs::create_dir_all(dest.join(dir_a)).unwrap();
            fs::create_dir_all(dest.join(dir_b)).unwrap();
            assert!(
                tool.has_existing_install_at(&dest),
                "{tool:?} should treat bare structural dirs as a prior install"
            );
            assert!(
                !tool.is_installed_at(&dest),
                "{tool:?} strict check should reject the bare-dirs case"
            );

            // Missing one of the two structural dirs — not a real install.
            let partial = temp_root.join(format!("{tool:?}_partial"));
            fs::create_dir_all(partial.join(dir_a)).unwrap();
            assert!(
                !tool.has_existing_install_at(&partial),
                "{tool:?} should not treat a half-populated dir as installed"
            );

            // Empty dest dir — never installed.
            let empty = temp_root.join(format!("{tool:?}_empty"));
            fs::create_dir_all(&empty).unwrap();
            assert!(
                !tool.has_existing_install_at(&empty),
                "{tool:?} should not treat an empty dir as installed"
            );
        }

        fs::remove_dir_all(&temp_root).ok();
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
